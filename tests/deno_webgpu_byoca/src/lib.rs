use std::cell::RefCell;
use std::ffi::c_void;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
  NSApplication, NSApplicationActivationPolicy, NSBackingStoreType,
  NSEventMask, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{
  NSDefaultRunLoopMode, NSPoint, NSRect, NSSize, NSString,
};
use objc2_quartz_core::CAMetalLayer;

struct WindowState {
  app: Retained<NSApplication>,
  window: Retained<NSWindow>,
  _view: Retained<NSView>,
  layer: Retained<CAMetalLayer>,
}

thread_local! {
  static WINDOW_STATE: RefCell<Option<WindowState>> = const { RefCell::new(None) };
}

#[no_mangle]
pub extern "C" fn byow_create_window(width: u32, height: u32) -> *mut c_void {
  let Some(mtm) = MainThreadMarker::new() else {
    return std::ptr::null_mut();
  };

  WINDOW_STATE.with(|slot| {
    if let Some(state) = slot.borrow().as_ref() {
      return Retained::as_ptr(&state.layer).cast::<c_void>().cast_mut();
    }

    let window_state = create_window(width, height, mtm);
    let layer = Retained::as_ptr(&window_state.layer).cast::<c_void>().cast_mut();
    *slot.borrow_mut() = Some(window_state);
    layer
  })
}

#[no_mangle]
pub extern "C" fn byow_poll_events() {
  let Some(app) = current_app() else {
    return;
  };

  while let Some(event) = app.nextEventMatchingMask_untilDate_inMode_dequeue(
    NSEventMask::Any,
    None,
    unsafe { NSDefaultRunLoopMode },
    true,
  ) {
    app.sendEvent(&event);
  }

  unsafe {
    let _: () = objc2::msg_send![&*app, updateWindows];
  }
}

#[no_mangle]
pub extern "C" fn byow_close_window() {
  WINDOW_STATE.with(|state| {
    if let Some(state) = state.borrow_mut().take() {
      state.window.close();
    }
  });
}

fn create_window(
  width: u32,
  height: u32,
  mtm: MainThreadMarker,
) -> WindowState {
  let app = NSApplication::sharedApplication(mtm);
  app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

  let rect = NSRect::new(
    NSPoint::new(0.0, 0.0),
    NSSize::new(width as f64, height as f64),
  );
  let style = NSWindowStyleMask::Titled
    | NSWindowStyleMask::Closable
    | NSWindowStyleMask::Miniaturizable
    | NSWindowStyleMask::Resizable;

  let window = unsafe {
    NSWindow::initWithContentRect_styleMask_backing_defer(
      NSWindow::alloc(mtm),
      rect,
      style,
      NSBackingStoreType::Buffered,
      false,
    )
  };

  let title = NSString::from_str("Deno WebGPU BYOW CAMetalLayer");
  window.setTitle(&title);
  window.center();

  let view = NSView::initWithFrame(NSView::alloc(mtm), rect);
  view.setWantsLayer(true);

  let layer = CAMetalLayer::layer();
  layer.setFrame(rect);
  layer.setContentsScale(window.backingScaleFactor());
  view.setLayer(Some(&layer));

  window.setContentView(Some(&view));
  window.makeKeyAndOrderFront(None::<&AnyObject>);
  #[allow(deprecated)]
  app.activateIgnoringOtherApps(true);
  window.display();

  WindowState {
    app,
    window,
    _view: view,
    layer,
  }
}

fn current_app() -> Option<Retained<NSApplication>> {
  WINDOW_STATE.with(|state| {
    state.borrow().as_ref().map(|state| unsafe {
      Retained::retain(Retained::as_ptr(&state.app).cast_mut()).unwrap()
    })
  })
}
