use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::OnceLock;

use crate::actor::{ActorError, ActorRef};
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
use super::view::{ViewError, ViewRef};

pub trait WindowDelegate: 'static {
    fn did_create(&mut self) {}
    fn will_close(&mut self) {}
    fn did_close(&mut self) {}
    fn did_click(&mut self) {}
}
impl WindowDelegate for () {}

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

struct WindowState<D> {
    delegate: RefCell<D>,
    closed: Cell<bool>,
    actor: RefCell<Option<ActorRef>>,
}
impl<D: WindowDelegate> WindowState<D> {
    fn invoke(&self, operation: impl FnOnce(&mut D)) {
        if catch_unwind(AssertUnwindSafe(|| {
            let Ok(mut delegate) = self.delegate.try_borrow_mut() else {
                std::process::abort()
            };
            operation(&mut delegate);
        }))
        .is_err()
        {
            std::process::abort()
        }
    }
    fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        self.invoke(WindowDelegate::will_close);
        self.invoke(WindowDelegate::did_close);
        if let Some(actor) = self.actor.borrow_mut().take() {
            actor.remove();
        }
    }

    fn click(&self) {
        if !self.closed.get() {
            self.invoke(WindowDelegate::did_click);
        }
    }
}

struct DelegateBox {
    state: *const (),
    close: unsafe fn(*const ()),
    release: unsafe fn(*const ()),
}
unsafe fn close_state<D: WindowDelegate>(raw: *const ()) {
    // SAFETY: raw is an Rc pointer retained for the delegate lifetime.
    let state = unsafe { Rc::from_raw(raw.cast::<WindowState<D>>()) };
    state.close();
    let _ = Rc::into_raw(state);
}
unsafe fn release_state<D>(raw: *const ()) {
    // SAFETY: This consumes the delegate's transferred Rc reference.
    unsafe { drop(Rc::from_raw(raw.cast::<WindowState<D>>())) };
}
unsafe fn click_state<D: WindowDelegate>(raw: *const ()) {
    // SAFETY: raw is an Rc pointer retained for the window lifetime.
    let state = unsafe { Rc::from_raw(raw.cast::<WindowState<D>>()) };
    state.click();
    let _ = Rc::into_raw(state);
}
unsafe fn delegate_box(object: Id) -> *mut DelegateBox {
    unsafe { native::get_pointer_ivar(object, c"_zpdWindowState".as_ptr()) }
}
unsafe extern "C" fn window_will_close(object: Id, _: native::Sel, _: Id) {
    // SAFETY: The callback receiver is live on entry; retaining it keeps the delegate and its
    // ivar state alive if closing removes the owning Actor subtree synchronously.
    let _receiver = unsafe { Strong::retain(object) };
    if let Some(state) = unsafe { delegate_box(object).as_ref() } {
        unsafe { (state.close)(state.state) }
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
        unsafe { (state.release)(state.state) };
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
            b"dealloc\0",
            delegate_dealloc as unsafe extern "C" fn(_, _),
            b"v@:\0",
        );
        native::objc_registerClassPair(class);
        class as usize
    }) as native::Class
}

struct ClickBox {
    state: *const (),
    click: unsafe fn(*const ()),
    release: unsafe fn(*const ()),
}

unsafe fn click_box(object: Id) -> *mut ClickBox {
    // SAFETY: ZpdRustWindow registers this pointer-sized ivar before use.
    unsafe { native::get_pointer_ivar(object, c"_zpdClickState".as_ptr()) }
}

unsafe extern "C" fn send_event(object: Id, _: native::Sel, event: Id) {
    // SAFETY: NSEvent.type returns an integer enum; 2 is leftMouseUp.
    let event_type = unsafe { native::send_i64(event, native::sel(b"type\0")) };
    if event_type == 2
        && let Some(state) = unsafe { click_box(object).as_ref() }
    {
        // SAFETY: The window owns the callback state for this dispatch.
        unsafe { (state.click)(state.state) };
    }
    let mut superclass = native::Super {
        receiver: object,
        superclass: native::class(b"NSWindow\0"),
    };
    // SAFETY: objc_msgSendSuper is wrapped by a typed NSWindow sendEvent helper.
    unsafe { native::send_super_void_id(&mut superclass, native::sel(b"sendEvent:\0"), event) };
}

unsafe extern "C" fn window_dealloc(object: Id, _: native::Sel) {
    unsafe { release_click_box(object) };
    // SAFETY: Continue normal NSWindow destruction after releasing Rust state.
    unsafe {
        native::send_super_void(
            object,
            native::class(b"NSWindow\0"),
            native::sel(b"dealloc\0"),
        )
    };
}

unsafe fn release_click_box(object: Id) {
    // SAFETY: ZpdRustWindow exclusively owns its ClickBox allocation.
    let state = unsafe { click_box(object) };
    if !state.is_null() {
        // SAFETY: Clearing the ivar transfers the sole ClickBox allocation to Rust.
        unsafe {
            native::set_pointer_ivar(
                object,
                c"_zpdClickState".as_ptr(),
                std::ptr::null_mut::<ClickBox>(),
            )
        };
        // SAFETY: This is the single allocation installed during window creation.
        let state = unsafe { Box::from_raw(state) };
        // SAFETY: release matches the generic state type used at creation.
        unsafe { (state.release)(state.state) };
    }
}

fn window_class() -> native::Class {
    static CLASS: OnceLock<usize> = OnceLock::new();
    *CLASS.get_or_init(|| {
        // SAFETY: Registration occurs exactly once before any instance is allocated.
        unsafe {
            let class = native::objc_allocateClassPair(
                native::class(b"NSWindow\0"),
                c"ZpdRustWindow".as_ptr(),
                0,
            );
            assert!(!class.is_null());
            assert!(native::class_addIvar(
                class,
                c"_zpdClickState".as_ptr(),
                std::mem::size_of::<Id>(),
                3,
                c"^v".as_ptr(),
            ));
            native::add_method(
                class,
                b"sendEvent:\0",
                send_event as unsafe extern "C" fn(_, _, _),
                b"v@:@\0",
            );
            native::add_method(
                class,
                b"dealloc\0",
                window_dealloc as unsafe extern "C" fn(_, _),
                b"v@:\0",
            );
            native::objc_registerClassPair(class);
            class as usize
        }
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

pub struct Window<'application, D: WindowDelegate> {
    actor: ActorRef,
    content: ActorRef,
    content_controller: ActorRef,
    sidebar: RefCell<Option<SidebarNative>>,
    _delegate: PhantomData<D>,
    _application: PhantomData<&'application Application<()>>,
    _main_thread: PhantomData<Rc<()>>,
}
impl<'application, D: WindowDelegate> Window<'application, D> {
    pub(crate) fn new<A: ApplicationDelegate>(
        application: &'application Application<A>,
        delegate: D,
    ) -> Result<Self, WindowError> {
        let state = Rc::new(WindowState {
            delegate: RefCell::new(delegate),
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
                native::send_id(window_class(), native::sel(b"alloc\0")),
                native::sel(b"initWithContentRect:styleMask:backing:defer:\0"),
                frame,
                (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3),
                2,
                false,
            ))
        }
        .ok_or(WindowError::NativeCreationFailed)?;
        // SAFETY: The subclass owns this click callback allocation until dealloc.
        unsafe {
            native::set_pointer_ivar(
                native_window.as_ptr(),
                c"_zpdClickState".as_ptr(),
                Box::into_raw(Box::new(ClickBox {
                    state: Rc::into_raw(state.clone()).cast(),
                    click: click_state::<D>,
                    release: release_state::<D>,
                })),
            );
        }
        unsafe {
            native::send_void_bool(
                native_window.as_ptr(),
                native::sel(b"setReleasedWhenClosed:\0"),
                false,
            )
        };
        let actor = application.tree().insert_root(native_window);
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
                release_click_box(window);
            })
            .map_err(WindowError::from)?;

        let controller = native::alloc_init(b"NSViewController\0");
        let view = unsafe {
            Strong::from_retained(native::send_id_rect(
                native::send_id(native::class(b"NSView\0"), native::sel(b"alloc\0")),
                native::sel(b"initWithFrame:\0"),
                frame,
            ))
        }
        .ok_or(WindowError::NativeCreationFailed)?;
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
                    close: close_state::<D>,
                    release: release_state::<D>,
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
        state.invoke(WindowDelegate::did_create);
        rollback.disarm();
        Ok(Self {
            actor,
            content,
            content_controller,
            sidebar: RefCell::new(None),
            _delegate: PhantomData,
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
impl<D: WindowDelegate> Drop for Window<'_, D> {
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::{WindowDelegate, WindowState};

    struct Probe {
        clicks: Rc<Cell<usize>>,
        closes: Rc<Cell<usize>>,
    }
    impl WindowDelegate for Probe {
        fn did_click(&mut self) {
            self.clicks.set(self.clicks.get() + 1);
        }
        fn did_close(&mut self) {
            self.closes.set(self.closes.get() + 1);
        }
    }

    #[test]
    fn window_state_ignores_clicks_after_close() {
        // Verifies NSWindow event forwarding cannot call a delegate after actor closure.
        let clicks = Rc::new(Cell::new(0));
        let closes = Rc::new(Cell::new(0));
        let state = WindowState {
            delegate: std::cell::RefCell::new(Probe {
                clicks: clicks.clone(),
                closes: closes.clone(),
            }),
            closed: Cell::new(false),
            actor: std::cell::RefCell::new(None),
        };
        state.click();
        state.close();
        state.click();
        assert_eq!(clicks.get(), 1);
        assert_eq!(closes.get(), 1);
    }
}
