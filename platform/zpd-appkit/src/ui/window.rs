use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::OnceLock;

use crate::actor::{ActorError, ActorRef, EventRouteToken, WindowEventKind};
#[cfg(feature = "wgpu")]
use crate::geometry::PhysicalSize;
use crate::geometry::Rect;
use crate::native::{self, Id, Strong};
use crate::runloop::{Application, ApplicationDelegate};

use super::sidebar::{self, Sidebar, SidebarError, SidebarNative};
#[cfg(feature = "wgpu")]
use super::view::AsView;
#[cfg(feature = "wgpu")]
use super::view::native_rect;
use super::view::{self, ViewError, ViewRef};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowError {
    NativeCreationFailed,
    Closed,
    InvalidHierarchy,
}
impl std::fmt::Display for WindowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NativeCreationFailed => "AppKit failed to create a native object",
            Self::Closed => "the window is closed",
            Self::InvalidHierarchy => "the requested native hierarchy is invalid",
        })
    }
}
impl std::error::Error for WindowError {}
impl From<ActorError> for WindowError {
    fn from(value: ActorError) -> Self {
        match value {
            ActorError::InvalidHierarchy => Self::InvalidHierarchy,
            _ => Self::Closed,
        }
    }
}
impl From<ViewError> for WindowError {
    fn from(value: ViewError) -> Self {
        match value {
            ViewError::NativeCreationFailed => Self::NativeCreationFailed,
            ViewError::InvalidHierarchy => Self::InvalidHierarchy,
            ViewError::Closed => Self::Closed,
        }
    }
}

struct WindowState {
    closed: Cell<bool>,
    actor: RefCell<Option<ActorRef>>,
}
impl WindowState {
    fn resized(&self) {
        let actor = self.actor.borrow().clone();
        if let Some(actor) = actor
            && let Some(tree) = actor.tree_handle()
        {
            tree.emit(&actor, WindowEventKind::DidResize);
        }
    }

    fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        if let Some(actor) = self.actor.borrow_mut().take() {
            if let Some(tree) = actor.tree_handle() {
                tree.emit(&actor, WindowEventKind::WillClose);
                tree.emit(&actor, WindowEventKind::DidClose);
            }
            actor.remove();
        }
    }
}

struct DelegateBox {
    state: *const (),
}
unsafe fn close_state(raw: *const ()) {
    // SAFETY: The delegate owns a live Rc pointer. A temporary strong count
    // keeps the state alive when close synchronously tears down that delegate.
    unsafe { Rc::increment_strong_count(raw.cast::<WindowState>()) };
    // SAFETY: The increment above created the reference consumed here.
    let state = unsafe { Rc::from_raw(raw.cast::<WindowState>()) };
    state.close();
}
unsafe fn resize_state(raw: *const ()) {
    // SAFETY: The delegate owns a live Rc pointer. A temporary strong count keeps the state alive
    // if resize handling synchronously tears down the delegate.
    unsafe { Rc::increment_strong_count(raw.cast::<WindowState>()) };
    // SAFETY: The increment above created the reference consumed here.
    let state = unsafe { Rc::from_raw(raw.cast::<WindowState>()) };
    state.resized();
}
unsafe fn release_state(raw: *const ()) {
    // SAFETY: This consumes the delegate's transferred Rc reference.
    unsafe { drop(Rc::from_raw(raw.cast::<WindowState>())) };
}
unsafe fn delegate_box(object: Id) -> *mut DelegateBox {
    unsafe { native::get_pointer_ivar(object, c"_zpdWindowState".as_ptr()) }
}
unsafe extern "C" fn window_will_close(object: Id, _: native::Sel, _: Id) {
    // SAFETY: The callback receiver is live on entry; retaining it keeps the delegate and its
    // ivar state alive if closing removes the owning Actor subtree synchronously.
    let _receiver = unsafe { Strong::retain(object) };
    if let Some(state) = unsafe { delegate_box(object).as_ref() } {
        unsafe { close_state(state.state) }
    }
}
unsafe extern "C" fn window_did_resize(object: Id, _: native::Sel, _: Id) {
    // SAFETY: The callback receiver is live on entry; retaining it keeps the delegate and its
    // ivar state alive if resize handling synchronously removes the Window subtree.
    let _receiver = unsafe { Strong::retain(object) };
    // SAFETY: This callback is installed only on delegates with a `_zpdWindowState` ivar.
    if let Some(state) = unsafe { delegate_box(object).as_ref() } {
        // SAFETY: The delegate owns the Rc pointer stored in its live state box.
        unsafe { resize_state(state.state) }
    }
}
unsafe extern "C" fn delegate_dealloc(object: Id, _: native::Sel) {
    unsafe { release_delegate_box(object) };
    unsafe {
        native::send_super_void(
            object,
            native::class(b"NSObject\0"),
            native::sel(b"dealloc\0"),
        )
    };
}

unsafe fn release_delegate_box(object: Id) {
    let state = unsafe { delegate_box(object) };
    if !state.is_null() {
        // SAFETY: Clearing the ivar transfers the sole DelegateBox allocation to Rust.
        unsafe {
            native::set_pointer_ivar(
                object,
                c"_zpdWindowState".as_ptr(),
                std::ptr::null_mut::<DelegateBox>(),
            )
        };
        // SAFETY: The native delegate owns exactly one DelegateBox.
        let state = unsafe { Box::from_raw(state) };
        unsafe { release_state(state.state) };
    }
}
fn window_delegate_class() -> native::Class {
    static CLASS: OnceLock<usize> = OnceLock::new();
    *CLASS.get_or_init(|| unsafe {
        let class = native::objc_allocateClassPair(
            native::class(b"NSObject\0"),
            c"ZpdRustWindowDelegate".as_ptr(),
            0,
        );
        assert!(!class.is_null());
        assert!(native::class_addIvar(
            class,
            c"_zpdWindowState".as_ptr(),
            std::mem::size_of::<Id>(),
            3,
            c"^v".as_ptr()
        ));
        native::add_method(
            class,
            b"windowWillClose:\0",
            window_will_close as unsafe extern "C" fn(_, _, _),
            b"v@:@\0",
        );
        native::add_method(
            class,
            b"windowDidResize:\0",
            window_did_resize as unsafe extern "C" fn(_, _, _),
            b"v@:@\0",
        );
        native::add_method(
            class,
            b"dealloc\0",
            delegate_dealloc as unsafe extern "C" fn(_, _),
            b"v@:\0",
        );
        native::objc_registerClassPair(class);
        class as usize
    }) as native::Class
}

struct ActorRollback(Option<ActorRef>);

impl ActorRollback {
    fn disarm(&mut self) {
        self.0.take();
    }
}

impl Drop for ActorRollback {
    fn drop(&mut self) {
        if let Some(actor) = self.0.take() {
            actor.remove();
        }
    }
}

pub struct Window<'application> {
    actor: ActorRef,
    content: ActorRef,
    content_controller: ActorRef,
    sidebar: RefCell<Option<SidebarNative>>,
    _application: PhantomData<&'application Application<()>>,
    _main_thread: PhantomData<Rc<()>>,
}
impl<'application> Window<'application> {
    pub(crate) fn new<A: ApplicationDelegate>(
        application: &'application Application<A>,
        event_route: Option<EventRouteToken>,
    ) -> Result<Self, WindowError> {
        let state = Rc::new(WindowState {
            closed: Cell::new(false),
            actor: std::cell::RefCell::new(None),
        });
        let frame = native::Rect {
            origin: native::Point { x: 0.0, y: 0.0 },
            size: native::Size {
                width: 480.0,
                height: 300.0,
            },
        };
        // SAFETY: This is NSWindow's arm64 designated initializer signature.
        let native_window = unsafe {
            Strong::from_retained(native::send_id_rect_u64_u64_bool(
                native::send_id(native::class(b"NSWindow\0"), native::sel(b"alloc\0")),
                native::sel(b"initWithContentRect:styleMask:backing:defer:\0"),
                frame,
                (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3),
                2,
                false,
            ))
        }
        .ok_or(WindowError::NativeCreationFailed)?;
        unsafe {
            native::send_void_bool(
                native_window.as_ptr(),
                native::sel(b"setReleasedWhenClosed:\0"),
                false,
            )
        };
        let actor = application.tree().insert_window(native_window);
        actor
            .set_event_route(event_route)
            .map_err(WindowError::from)?;
        let mut rollback = ActorRollback(Some(actor.clone()));
        *state.actor.borrow_mut() = Some(actor.clone());
        application
            .tree()
            .add_teardown(&actor, |window| unsafe {
                native::send_void_id(window, native::sel(b"setDelegate:\0"), native::NIL);
                native::send_void_id(
                    window,
                    native::sel(b"setContentViewController:\0"),
                    native::NIL,
                );
            })
            .map_err(WindowError::from)?;

        let controller = native::alloc_init(b"NSViewController\0");
        let view = view::new_layout_view(Rect::new(
            frame.origin.x,
            frame.origin.y,
            frame.size.width,
            frame.size.height,
        ))?;
        unsafe {
            native::send_void_id(
                controller.as_ptr(),
                native::sel(b"setView:\0"),
                view.as_ptr(),
            );
            actor
                .with(|window| {
                    native::send_void_id(
                        window,
                        native::sel(b"setContentViewController:\0"),
                        controller.as_ptr(),
                    )
                })
                .map_err(WindowError::from)?;
        }
        let content = application
            .tree()
            .insert_child(&actor, view)
            .map_err(WindowError::from)?;
        application
            .tree()
            .add_teardown(&content, |view| unsafe {
                view::release_layout_callback(view)
            })
            .map_err(WindowError::from)?;
        let content_controller = application
            .tree()
            .replace_owned(&actor, "content-controller", controller)
            .map_err(WindowError::from)?;
        let class = window_delegate_class();
        let native_delegate = unsafe {
            let object = native::send_id(
                native::send_id(class, native::sel(b"alloc\0")),
                native::sel(b"init\0"),
            );
            native::set_pointer_ivar(
                object,
                c"_zpdWindowState".as_ptr(),
                Box::into_raw(Box::new(DelegateBox {
                    state: Rc::into_raw(state.clone()).cast(),
                })),
            );
            Strong::from_retained(object).ok_or(WindowError::NativeCreationFailed)?
        };
        actor
            .with(|window| unsafe {
                native::send_void_id(
                    window,
                    native::sel(b"setDelegate:\0"),
                    native_delegate.as_ptr(),
                )
            })
            .map_err(WindowError::from)?;
        let delegate_actor = application
            .tree()
            .replace_owned(&actor, "delegate", native_delegate)
            .map_err(WindowError::from)?;
        application
            .tree()
            .add_teardown(&delegate_actor, |delegate| unsafe {
                release_delegate_box(delegate)
            })
            .map_err(WindowError::from)?;
        application.tree().emit(&actor, WindowEventKind::Created);
        rollback.disarm();
        Ok(Self {
            actor,
            content,
            content_controller,
            sidebar: RefCell::new(None),
            _application: PhantomData,
            _main_thread: PhantomData,
        })
    }
    fn ensure_open(&self) -> Result<(), WindowError> {
        if !self.actor.is_alive() {
            Err(WindowError::Closed)
        } else {
            Ok(())
        }
    }
    pub fn is_closed(&self) -> bool {
        self.ensure_open().is_err()
    }
    /// Returns a weak reference to this window's Actor.
    pub fn actor_ref(&self) -> ActorRef {
        self.actor.clone()
    }
    pub fn show(&self) -> Result<(), WindowError> {
        self.ensure_open()?;
        self.actor
            .with(|window| unsafe {
                native::send_void_id(window, native::sel(b"makeKeyAndOrderFront:\0"), native::NIL)
            })
            .map_err(Into::into)
    }
    pub fn set_title(&self, title: &str) -> Result<(), WindowError> {
        self.ensure_open()?;
        let title = native::nsstring(title);
        self.actor
            .with(|window| unsafe {
                native::send_void_id(window, native::sel(b"setTitle:\0"), title.as_ptr())
            })
            .map_err(Into::into)
    }
    pub fn set_identifier(&self, identifier: Option<&str>) -> Result<(), WindowError> {
        self.ensure_open()?;
        let identifier = identifier.map(native::nsstring);
        self.actor
            .with(|window| unsafe {
                native::send_void_id(
                    window,
                    native::sel(b"setAccessibilityIdentifier:\0"),
                    identifier.as_ref().map_or(native::NIL, Strong::as_ptr),
                )
            })
            .map_err(Into::into)
    }
    pub fn set_bounds(&self, bounds: Rect) -> Result<(), WindowError> {
        self.ensure_open()?;
        let screen =
            unsafe { native::send_id(native::class(b"NSScreen\0"), native::sel(b"mainScreen\0")) };
        let screen_frame = if screen.is_null() {
            native::Rect::default()
        } else {
            unsafe { native::send_rect(screen, native::sel(b"frame\0")) }
        };
        let frame = native::Rect {
            origin: native::Point {
                x: bounds.x,
                y: screen_frame.origin.y + screen_frame.size.height - bounds.y - bounds.height,
            },
            size: native::Size {
                width: bounds.width,
                height: bounds.height,
            },
        };
        self.actor
            .with(|window| unsafe {
                native::send_void_rect_bool(
                    window,
                    native::sel(b"setFrame:display:\0"),
                    frame,
                    true,
                )
            })
            .map_err(Into::into)
    }
    pub fn set_size(&self, width: f64, height: f64) -> Result<(), WindowError> {
        self.ensure_open()?;
        let mut frame = self
            .actor
            .with(|window| unsafe { native::send_rect(window, native::sel(b"frame\0")) })
            .map_err(WindowError::from)?;
        frame.size = native::Size { width, height };
        self.actor
            .with(|window| unsafe {
                native::send_void_rect_bool(
                    window,
                    native::sel(b"setFrame:display:\0"),
                    frame,
                    true,
                )
            })
            .map_err(Into::into)
    }
    pub fn set_position(&self, x: f64, y: f64) -> Result<(), WindowError> {
        self.ensure_open()?;
        let screen =
            unsafe { native::send_id(native::class(b"NSScreen\0"), native::sel(b"mainScreen\0")) };
        let screen_frame = if screen.is_null() {
            native::Rect::default()
        } else {
            unsafe { native::send_rect(screen, native::sel(b"frame\0")) }
        };
        self.actor
            .with(|window| unsafe {
                native::send_void_point(
                    window,
                    native::sel(b"setFrameTopLeftPoint:\0"),
                    native::Point {
                        x,
                        y: screen_frame.origin.y + screen_frame.size.height - y,
                    },
                )
            })
            .map_err(Into::into)
    }
    pub fn content_view(&self) -> Result<ViewRef<'_>, WindowError> {
        self.ensure_open()?;
        Ok(ViewRef::from_actor(&self.content))
    }
    pub fn set_content_layout_handler(
        &self,
        callback: impl FnMut(Rect) + 'static,
    ) -> Result<(), WindowError> {
        self.ensure_open()?;
        view::set_layout_callback(ViewRef::from_actor(&self.content), callback)
            .map_err(WindowError::from)
    }
    pub fn set_sidebar<F>(&self, sidebar: &Sidebar, callback: F) -> Result<(), SidebarError>
    where
        F: FnMut(&str) + 'static,
    {
        self.ensure_open().map_err(|_| SidebarError::Closed)?;
        let native = sidebar::install(&self.actor, &self.content_controller, sidebar, callback)?;
        *self.sidebar.borrow_mut() = Some(native);
        Ok(())
    }
    pub fn clear_sidebar(&self) -> Result<(), WindowError> {
        self.ensure_open()?;
        if let Some(sidebar) = self.sidebar.borrow_mut().take() {
            sidebar.clear(&self.actor)?;
        }
        Ok(())
    }
    #[cfg(feature = "wgpu")]
    pub fn create_wgpu_surface(
        &self,
        rect: Rect,
    ) -> Result<WgpuSurface<'application>, WindowError> {
        self.ensure_open()?;
        WgpuSurface::new(&self.actor, &self.content, rect)
    }
}
impl Drop for Window<'_> {
    fn drop(&mut self) {
        self.sidebar.borrow_mut().take();
        if self.actor.is_alive() {
            let _ = self
                .actor
                .with(|window| unsafe { native::send_void(window, native::sel(b"close\0")) });
        }
        self.actor.remove();
    }
}

#[cfg(feature = "wgpu")]
#[derive(Clone, Copy)]
pub struct MetalLayer<'surface> {
    actor: &'surface ActorRef,
}
#[cfg(feature = "wgpu")]
impl MetalLayer<'_> {
    pub fn as_ptr(self) -> Result<*mut std::ffi::c_void, WindowError> {
        self.actor.with(|id| id).map_err(WindowError::from)
    }
}

#[cfg(feature = "wgpu")]
pub struct WgpuSurface<'application> {
    view: super::view::ViewActor,
    layer: ActorRef,
    _application: PhantomData<&'application Application<()>>,
}
#[cfg(feature = "wgpu")]
impl<'application> WgpuSurface<'application> {
    fn new(window: &ActorRef, content: &ActorRef, rect: Rect) -> Result<Self, WindowError> {
        let native_view = unsafe {
            Strong::from_retained(native::send_id_rect(
                native::send_id(native::class(b"NSView\0"), native::sel(b"alloc\0")),
                native::sel(b"initWithFrame:\0"),
                native_rect(rect),
            ))
        }
        .ok_or(WindowError::NativeCreationFailed)?;
        let view_actor = {
            let tree_inner = content.clone();
            // Insert via the content actor's tree by first creating under root is not exposed;
            // retain the view and use the content tree through a temporary reparent.
            let actor_tree = window_tree(window)?;
            let actor = actor_tree
                .insert_child(content, native_view)
                .map_err(WindowError::from)?;
            let _ = tree_inner;
            actor
        };
        let layer_native = unsafe {
            Strong::retain(native::send_id(
                native::class(b"CAMetalLayer\0"),
                native::sel(b"layer\0"),
            ))
        }
        .ok_or(WindowError::NativeCreationFailed)?;
        let actor_tree = window_tree(window)?;
        let layer = actor_tree
            .insert_child(&view_actor, layer_native)
            .map_err(WindowError::from)?;
        view_actor
            .with(|view| {
                layer.with(|layer| unsafe {
                    native::send_void_bool(view, native::sel(b"setWantsLayer:\0"), true);
                    native::send_void_id(view, native::sel(b"setLayer:\0"), layer);
                })
            })
            .map_err(WindowError::from)?
            .map_err(WindowError::from)?;
        content
            .with(|parent| {
                view_actor.with(|view| unsafe {
                    native::send_void_id(parent, native::sel(b"addSubview:\0"), view)
                })
            })
            .map_err(WindowError::from)?
            .map_err(WindowError::from)?;
        let surface = Self {
            view: super::view::ViewActor::from_actor(view_actor),
            layer,
            _application: PhantomData,
        };
        surface.update_drawable_size()?;
        Ok(surface)
    }
    pub fn set_rect(&self, rect: Rect) -> Result<(), WindowError> {
        self.view.as_view().set_frame(rect)?;
        self.update_drawable_size()
    }
    pub fn drawable_size(&self) -> Result<PhysicalSize, WindowError> {
        self.update_drawable_size()?;
        let size = self
            .layer
            .with(|layer| unsafe { native::send_size(layer, native::sel(b"drawableSize\0")) })
            .map_err(WindowError::from)?;
        Ok(PhysicalSize::new(
            size.width.max(0.0).round() as u32,
            size.height.max(0.0).round() as u32,
        ))
    }
    fn update_drawable_size(&self) -> Result<(), WindowError> {
        let bounds = self
            .view
            .actor()
            .with(|view| unsafe { native::send_rect(view, native::sel(b"bounds\0")) })
            .map_err(WindowError::from)?;
        let screen =
            unsafe { native::send_id(native::class(b"NSScreen\0"), native::sel(b"mainScreen\0")) };
        let scale = if screen.is_null() {
            1.0
        } else {
            unsafe { native::send_f64(screen, native::sel(b"backingScaleFactor\0")) }
        };
        self.layer
            .with(|layer| unsafe {
                native::send_void_f64(layer, native::sel(b"setContentsScale:\0"), scale);
                native::send_void_size(
                    layer,
                    native::sel(b"setDrawableSize:\0"),
                    native::Size {
                        width: bounds.size.width * scale,
                        height: bounds.size.height * scale,
                    },
                );
            })
            .map_err(Into::into)
    }
    pub fn metal_layer(&self) -> Result<MetalLayer<'_>, WindowError> {
        self.layer.with(|_| ()).map_err(WindowError::from)?;
        Ok(MetalLayer { actor: &self.layer })
    }
}
#[cfg(feature = "wgpu")]
impl AsView for WgpuSurface<'_> {
    fn as_view(&self) -> ViewRef<'_> {
        self.view.as_view()
    }
}

#[cfg(feature = "wgpu")]
fn window_tree(actor: &ActorRef) -> Result<crate::actor::ActorTree, WindowError> {
    actor.tree_handle().ok_or(WindowError::Closed)
}
