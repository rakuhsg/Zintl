import AppKit
import QuartzCore

@MainActor
private final class WindowState {
  static var current: WindowState?

  let app: NSApplication
  let window: NSWindow
  let view: NSView
  let layer: CAMetalLayer

  init(width: UInt32, height: UInt32) {
    let app = NSApplication.shared
    app.setActivationPolicy(.regular)
    app.finishLaunching()

    let rect = NSRect(
      x: 0,
      y: 0,
      width: Double(width),
      height: Double(height)
    )
    let window = NSWindow(
      contentRect: rect,
      styleMask: [.titled, .closable, .miniaturizable, .resizable],
      backing: .buffered,
      defer: false
    )
    window.title = "Deno WebGPU BYOW CAMetalLayer"
    window.center()

    let view = NSView(frame: rect)
    view.wantsLayer = true

    let layer = CAMetalLayer()
    let scale = window.backingScaleFactor
    layer.frame = view.bounds
    layer.contentsScale = scale
    layer.drawableSize = CGSize(
      width: view.bounds.width * scale,
      height: view.bounds.height * scale
    )
    view.layer = layer

    window.contentView = view
    window.makeKeyAndOrderFront(nil)
    window.orderFrontRegardless()
    app.activate(ignoringOtherApps: true)
    window.display()

    self.app = app
    self.window = window
    self.view = view
    self.layer = layer
  }
}

@MainActor
@_cdecl("byow_create_window")
public func createWindow(width: UInt32, height: UInt32) -> UnsafeMutableRawPointer? {
  if let state = WindowState.current {
    return Unmanaged.passUnretained(state.layer).toOpaque()
  }

  let state = WindowState(width: width, height: height)
  WindowState.current = state
  return Unmanaged.passUnretained(state.layer).toOpaque()
}

@MainActor
@_cdecl("byow_poll_events")
public func pollEvents() {
  guard let state = WindowState.current else {
    return
  }

  while let event = state.app.nextEvent(
    matching: .any,
    until: nil,
    inMode: .default,
    dequeue: true
  ) {
    state.app.sendEvent(event)
  }

  state.app.updateWindows()
}

@MainActor
@_cdecl("byow_close_window")
public func closeWindow() {
  guard let state = WindowState.current else {
    return
  }

  state.window.close()
  WindowState.current = nil
}
