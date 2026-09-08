use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::OnceLock;

use crate::actor::{ActorError, ActorRef, EventRouteToken, WindowEventKind};
#[cfg(feature = "wgpu")]
use crate::geometry::PhysicalSize;
use crate::geometry::Rect;
use crate::native;
use crate::runloop::{Application, ApplicationDelegate};
use zpd_objc::{Id, Strong};

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
    unsafe { zpd_objc::get_pointer_ivar(object, c"_zpdWindowState".as_ptr()) }
}
unsafe extern "C" fn window_will_close(object: Id, _: zpd_objc::Sel, _: Id) {
    // SAFETY: The callback receiver is live on entry; retaining it keeps the delegate and its
    // ivar state alive if closing removes the owning Actor subtree synchronously.
    let _receiver = unsafe { Strong::retain(object) };
    if let Some(state) = unsafe { delegate_box(object).as_ref() } {
        unsafe { close_state(state.state) }
    }
}
unsafe extern "C" fn window_did_resize(object: Id, _: zpd_objc::Sel, _: Id) {
    // SAFETY: The callback receiver is live on entry; retaining it keeps the delegate and its
    // ivar state alive if resize handling synchronously removes the Window subtree.
    let _receiver = unsafe { Strong::retain(object) };
    // SAFETY: This callback is installed only on delegates with a `_zpdWindowState` ivar.
    if let Some(state) = unsafe { delegate_box(object).as_ref() } {
        // SAFETY: The delegate owns the Rc pointer stored in its live state box.
        unsafe { resize_state(state.state) }
    }
}
unsafe extern "C" fn delegate_dealloc(object: Id, _: zpd_objc::Sel) {
    unsafe { release_delegate_box(object) };
    unsafe {
        zpd_objc::msg_send_super!(object, zpd_objc::class!("NSObject"), zpd_objc::sel!("dealloc"), () => ())
    };
}

unsafe fn release_delegate_box(object: Id) {
    let state = unsafe { delegate_box(object) };
    if !state.is_null() {
        // SAFETY: Clearing the ivar transfers the sole DelegateBox allocation to Rust.
        unsafe {
            zpd_objc::set_pointer_ivar(
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
fn window_delegate_class() -> zpd_objc::Class {
    static CLASS: OnceLock<usize> = OnceLock::new();
    *CLASS.get_or_init(|| {
        zpd_objc::decl!(ZpdRustWindowDelegate: [zpd_objc::class!("NSObject")] {
            fields { _zpdWindowState: ptr }
            methods {
                "windowWillClose:": "v@:@" => window_will_close,
                "windowDidResize:": "v@:@" => window_did_resize,
                "dealloc": "v@:" => delegate_dealloc,
            }
        }) as usize
    }) as zpd_objc::Class
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
    sidebar_toolbar: RefCell<Option<super::sidebar_toolbar::SidebarToolbar>>,
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
        let style_mask = native::NS_WINDOW_STYLE_MASK_TITLED
            | native::NS_WINDOW_STYLE_MASK_CLOSABLE
            | native::NS_WINDOW_STYLE_MASK_MINIATURIZABLE
            | native::NS_WINDOW_STYLE_MASK_RESIZABLE;
        // SAFETY: This is NSWindow's arm64 designated initializer signature.
        let native_window = unsafe {
            Strong::from_retained(zpd_objc::msg_send!(zpd_objc::msg_send!(zpd_objc::class!("NSWindow"), zpd_objc::sel!("alloc"), () => zpd_objc::Id), zpd_objc::sel!("initWithContentRect:styleMask:backing:defer:"), ((frame): native::Rect, (style_mask): u64, (native::NS_BACKING_STORE_BUFFERED): u64, (false): bool) => zpd_objc::Id))
        }
        .ok_or(WindowError::NativeCreationFailed)?;
        unsafe {
            zpd_objc::msg_send!(native_window.as_ptr(), zpd_objc::sel!("setReleasedWhenClosed:"), ((false): bool) => ())
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
                zpd_objc::msg_send!(window, zpd_objc::sel!("setDelegate:"), ((zpd_objc::NIL): zpd_objc::Id) => ());
                zpd_objc::msg_send!(window, zpd_objc::sel!("setContentViewController:"), ((zpd_objc::NIL): zpd_objc::Id) => ());
            })
            .map_err(WindowError::from)?;

        let controller = native::alloc_init(zpd_objc::class!("NSViewController"));
        let view = view::new_layout_view(Rect::new(
            frame.origin.x,
            frame.origin.y,
            frame.size.width,
            frame.size.height,
        ))?;
        unsafe {
            zpd_objc::msg_send!(controller.as_ptr(), zpd_objc::sel!("setView:"), ((view.as_ptr()): zpd_objc::Id) => ());
            actor
                .with(|window| {
                    zpd_objc::msg_send!(window, zpd_objc::sel!("setContentViewController:"), ((controller.as_ptr()): zpd_objc::Id) => ())
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
            let object = zpd_objc::msg_send!(zpd_objc::msg_send!(class, zpd_objc::sel!("alloc"), () => zpd_objc::Id), zpd_objc::sel!("init"), () => zpd_objc::Id);
            zpd_objc::set_pointer_ivar(
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
                zpd_objc::msg_send!(window, zpd_objc::sel!("setDelegate:"), ((native_delegate.as_ptr()): zpd_objc::Id) => ())
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
            sidebar_toolbar: RefCell::new(None),
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
                zpd_objc::msg_send!(window, zpd_objc::sel!("makeKeyAndOrderFront:"), ((zpd_objc::NIL): zpd_objc::Id) => ())
            })
            .map_err(Into::into)
    }
    pub fn set_title(&self, title: &str) -> Result<(), WindowError> {
        self.ensure_open()?;
        let title = native::nsstring(title);
        self.actor
            .with(|window| unsafe {
                zpd_objc::msg_send!(window, zpd_objc::sel!("setTitle:"), ((title.as_ptr()): zpd_objc::Id) => ())
            })
            .map_err(Into::into)
    }
    pub fn set_identifier(&self, identifier: Option<&str>) -> Result<(), WindowError> {
        self.ensure_open()?;
        let identifier = identifier.map(native::nsstring);
        self.actor
            .with(|window| unsafe {
                zpd_objc::msg_send!(window, zpd_objc::sel!("setAccessibilityIdentifier:"), ((identifier.as_ref().map_or(zpd_objc::NIL, Strong::as_ptr)): zpd_objc::Id) => ())
            })
            .map_err(Into::into)
    }
    pub fn set_full_size_content_view(&self, enabled: bool) -> Result<(), WindowError> {
        self.ensure_open()?;
        self.actor
            .with(|window| {
                // SAFETY: The live actor is an NSWindow and both selectors use NSUInteger masks.
                unsafe {
                    let mut style_mask =
                        zpd_objc::msg_send!(window, zpd_objc::sel!("styleMask"), () => u64);
                    let is_enabled = style_mask
                        & native::NS_WINDOW_STYLE_MASK_FULL_SIZE_CONTENT_VIEW
                        != 0;
                    if enabled != is_enabled {
                        if enabled {
                            style_mask |= native::NS_WINDOW_STYLE_MASK_FULL_SIZE_CONTENT_VIEW;
                        } else {
                            style_mask &= !native::NS_WINDOW_STYLE_MASK_FULL_SIZE_CONTENT_VIEW;
                        }
                        zpd_objc::msg_send!(window, zpd_objc::sel!("setStyleMask:"), ((style_mask): u64) => ())
                    }
                }
            })
            .map_err(Into::into)
    }
    pub fn set_bounds(&self, bounds: Rect) -> Result<(), WindowError> {
        self.ensure_open()?;
        let screen = unsafe {
            zpd_objc::msg_send!(zpd_objc::class!("NSScreen"), zpd_objc::sel!("mainScreen"), () => zpd_objc::Id)
        };
        let screen_frame = if screen.is_null() {
            native::Rect::default()
        } else {
            unsafe { zpd_objc::msg_send!(screen, zpd_objc::sel!("frame"), () => native::Rect) }
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
                zpd_objc::msg_send!(window, zpd_objc::sel!("setFrame:display:"), ((frame): native::Rect, (true): bool) => ())
            })
            .map_err(Into::into)
    }
    pub fn set_size(&self, width: f64, height: f64) -> Result<(), WindowError> {
        self.ensure_open()?;
        let mut frame = self
            .actor
            .with(|window| unsafe {
                zpd_objc::msg_send!(window, zpd_objc::sel!("frame"), () => native::Rect)
            })
            .map_err(WindowError::from)?;
        frame.size = native::Size { width, height };
        self.actor
            .with(|window| unsafe {
                zpd_objc::msg_send!(window, zpd_objc::sel!("setFrame:display:"), ((frame): native::Rect, (true): bool) => ())
            })
            .map_err(Into::into)
    }
    pub fn set_position(&self, x: f64, y: f64) -> Result<(), WindowError> {
        self.ensure_open()?;
        let screen = unsafe {
            zpd_objc::msg_send!(zpd_objc::class!("NSScreen"), zpd_objc::sel!("mainScreen"), () => zpd_objc::Id)
        };
        let screen_frame = if screen.is_null() {
            native::Rect::default()
        } else {
            unsafe { zpd_objc::msg_send!(screen, zpd_objc::sel!("frame"), () => native::Rect) }
        };
        self.actor
            .with(|window| unsafe {
                zpd_objc::msg_send!(window, zpd_objc::sel!("setFrameTopLeftPoint:"), ((native::Point {
                        x,
                        y: screen_frame.origin.y + screen_frame.size.height - y,
                    }): native::Point) => ())
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
    pub fn set_sidebar<F>(&self, sidebar: &Sidebar, mut callback: F) -> Result<(), SidebarError>
    where
        F: FnMut(&str) + 'static,
    {
        self.ensure_open().map_err(|_| SidebarError::Closed)?;
        let frame = self
            .actor
            .with(|window| {
                // SAFETY: The live actor is an NSWindow and `frame` returns an NSRect by value.
                unsafe { zpd_objc::msg_send!(window, zpd_objc::sel!("frame"), () => native::Rect) }
            })
            .map_err(|_| SidebarError::Closed)?;
        let actor = self.actor.clone();
        let collapsed = self
            .sidebar
            .borrow()
            .as_ref()
            .is_some_and(SidebarNative::is_collapsed);
        let native = sidebar::install(
            &self.actor,
            &self.content_controller,
            sidebar,
            collapsed,
            move |id| {
                if let Some(tree) = actor.tree_handle() {
                    tree.emit(
                        &actor,
                        WindowEventKind::SidebarSelectionChanged { id: id.into() },
                    );
                }
                callback(id);
            },
        )?;
        self.actor
            .with(|window| {
                // SAFETY: Restoring the captured NSRect keeps controller replacement from
                // changing the user's current window position or size.
                unsafe {
                    zpd_objc::msg_send!(window, zpd_objc::sel!("setFrame:display:"), ((frame): native::Rect, (true): bool) => ())
                }
            })
            .map_err(|_| SidebarError::Closed)?;
        *self.sidebar.borrow_mut() = Some(native);
        if self.sidebar_toolbar.borrow().is_none() {
            *self.sidebar_toolbar.borrow_mut() = Some(
                super::sidebar_toolbar::SidebarToolbar::install(&self.actor)?,
            );
        }
        Ok(())
    }
    pub fn clear_sidebar(&self) -> Result<(), WindowError> {
        self.ensure_open()?;
        if let Some(sidebar) = self.sidebar.borrow_mut().take() {
            sidebar.clear(&self.actor)?;
        }
        if let Some(toolbar) = self.sidebar_toolbar.borrow_mut().take() {
            toolbar.clear(&self.actor)?;
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
            let _ = self.actor.with(|window| unsafe {
                zpd_objc::msg_send!(window, zpd_objc::sel!("close"), () => ())
            });
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
            Strong::from_retained(zpd_objc::msg_send!(zpd_objc::msg_send!(zpd_objc::class!("NSView"), zpd_objc::sel!("alloc"), () => zpd_objc::Id), zpd_objc::sel!("initWithFrame:"), ((native_rect(rect)): native::Rect) => zpd_objc::Id))
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
            Strong::retain(zpd_objc::msg_send!(zpd_objc::class!("CAMetalLayer"), zpd_objc::sel!("layer"), () => zpd_objc::Id))
        }
        .ok_or(WindowError::NativeCreationFailed)?;
        let actor_tree = window_tree(window)?;
        let layer = actor_tree
            .insert_child(&view_actor, layer_native)
            .map_err(WindowError::from)?;
        view_actor
            .with(|view| {
                layer.with(|layer| unsafe {
                    zpd_objc::msg_send!(view, zpd_objc::sel!("setWantsLayer:"), ((true): bool) => ());
                    zpd_objc::msg_send!(view, zpd_objc::sel!("setLayer:"), ((layer): zpd_objc::Id) => ());
                })
            })
            .map_err(WindowError::from)?
            .map_err(WindowError::from)?;
        content
            .with(|parent| {
                view_actor.with(|view| unsafe {
                    zpd_objc::msg_send!(parent, zpd_objc::sel!("addSubview:"), ((view): zpd_objc::Id) => ())
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
            .with(|layer| unsafe {
                zpd_objc::msg_send!(layer, zpd_objc::sel!("drawableSize"), () => native::Size)
            })
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
            .with(|view| unsafe {
                zpd_objc::msg_send!(view, zpd_objc::sel!("bounds"), () => native::Rect)
            })
            .map_err(WindowError::from)?;
        let screen = unsafe {
            zpd_objc::msg_send!(zpd_objc::class!("NSScreen"), zpd_objc::sel!("mainScreen"), () => zpd_objc::Id)
        };
        let scale = if screen.is_null() {
            1.0
        } else {
            unsafe { zpd_objc::msg_send!(screen, zpd_objc::sel!("backingScaleFactor"), () => f64) }
        };
        self.layer
            .with(|layer| unsafe {
                zpd_objc::msg_send!(layer, zpd_objc::sel!("setContentsScale:"), ((scale): f64) => ());
                zpd_objc::msg_send!(layer, zpd_objc::sel!("setDrawableSize:"), ((native::Size {
                        width: bounds.size.width * scale,
                        height: bounds.size.height * scale,
                    }): native::Size) => ());
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
