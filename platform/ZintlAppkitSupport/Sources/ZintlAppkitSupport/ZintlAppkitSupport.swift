import AppKit
import CoreFoundation
import QuartzCore
import ZintlAppkitSupportTypes

struct State {
  var source: CFRunLoopSource
  var loop: CFRunLoop
}

class RAppDelegate: NSObject, NSApplicationDelegate {
  var ud: UnsafeRawPointer
  var cb: AppCallback

  init(ud: UnsafeRawPointer, cb: AppCallback) {
    self.ud = ud
    self.cb = cb
  }

  func applicationDidFinishLaunching(_ notification: Notification) {
    let mainMenu = NSMenu()

    let appMenuItem = NSMenuItem()
    mainMenu.addItem(appMenuItem)
    let appMenu = NSMenu()
    appMenuItem.submenu = appMenu

    let aboutTitle = "About " + ProcessInfo.processInfo.processName
    appMenu.addItem(
      withTitle: aboutTitle,
      action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)),
      keyEquivalent: ""
    )

    appMenu.addItem(
      withTitle: "Quit",
      action: #selector(NSApplication.terminate(_:)),
      keyEquivalent: "q"
    )

    NSApp.mainMenu = mainMenu

    self.cb.on_launch(self.ud)
  }

  func applicationWillTerminate(_ notification: Notification) {
    self.cb.will_terminate(self.ud)
  }
}

@MainActor
class ZintlAppkitSupportState {
  static let shared = ZintlAppkitSupportState()

  var state: State?
}

typealias PerformFn = (@convention(c) (UnsafeRawPointer?) -> Void)?

class ZintlUdWrapper {
  var cb: PerformFn
  var ud: UnsafeRawPointer

  init(_ ud: UnsafeRawPointer, cb: PerformFn) {
    self.ud = ud
    self.cb = cb
  }
}

func performRustFn(rp: UnsafeMutableRawPointer?) {
  if rp != nil {
    let wrapper = Unmanaged<ZintlUdWrapper>.fromOpaque(rp!)
      .takeUnretainedValue()
    wrapper.cb!(wrapper.ud)
  }
}

func releaseRustFn(rp: UnsafeRawPointer?) {
  if rp != nil {
    _ = Unmanaged<ZintlUdWrapper>.fromOpaque(rp!)
      .takeRetainedValue()
  }
}

struct ZintlWindowCommandSet: Decodable {
  var menus: [ZintlWindowCommandMenu]
}

struct ZintlWindowCommandMenu: Decodable {
  var title: String
  var items: [ZintlWindowCommandItem]
}

struct ZintlWindowCommandItem: Decodable {
  var id: String
  var title: String
  var key: String?
  var modifiers: [String]?
  var enabled: Bool?
}

@MainActor
class ZintlCommandTarget: NSObject {
  weak var window: RWindow?
  let commandID: String

  init(window: RWindow, commandID: String) {
    self.window = window
    self.commandID = commandID
  }

  @objc func handleMenuCommand(_ sender: Any?) {
    self.window?.performCommand(self.commandID)
  }
}

@MainActor
@_cdecl("zintlappkit_init")
func zintlAppkitInit(ud: UnsafeRawPointer, appcbPtr: UnsafePointer<AppCallback>) {
  let appcb = appcbPtr.pointee

  // init appkit app
  let app = NSApplication.shared
  app.setActivationPolicy(.regular)

  let loop = RunLoop.current.getCFRunLoop()

  assert(
    ZintlAppkitSupportState.shared.state == nil,
    "ZintlAppkitSupportState is already initialized"
  )

  var source_cx = CFRunLoopSourceContext()
  source_cx.info = Unmanaged.passRetained(
    ZintlUdWrapper(ud, cb: appcb.perform)
  ).toOpaque()
  source_cx.perform = performRustFn
  source_cx.release = releaseRustFn
  let source = CFRunLoopSourceCreate(nil, 1, &source_cx)!
  CFRunLoopAddSource(loop, source, .commonModes)

  let delegate = RAppDelegate(ud: ud, cb: appcb)

  app.delegate = delegate

  ZintlAppkitSupportState.shared.state = State(source: source, loop: loop)
}

@_cdecl("zintlappkit_schedule")
func zintlAppkitSchedule() {
  Task { @MainActor in
    guard let state = ZintlAppkitSupportState.shared.state else {
      assertionFailure("ZintlAppkitSupportState is not initialized")
      return
    }

    CFRunLoopSourceSignal(state.source)
    CFRunLoopWakeUp(state.loop)
  }
}

@MainActor
@_cdecl("zintlappkit_run")
func zintlAppkitRun() {
  guard ZintlAppkitSupportState.shared.state != nil else {
    assertionFailure("ZintlAppkitSupportState is not initialized")
    return
  }
  NSApp.activate(ignoringOtherApps: true)
  NSApp.run()
}

@MainActor
@_cdecl("zintlappkit_destroy")
func zintlAppkitDestroy() {
  ZintlAppkitSupportState.shared.state = nil
}

@MainActor
class RWindow: NSObject, NSWindowDelegate {
  var window: NSWindow
  var commandSet = ZintlWindowCommandSet(menus: [])
  var commandTargets: [ZintlCommandTarget] = []
  var commandUserData: UnsafeRawPointer?
  var commandCallback: ZintlWindowCommandCallback?
  var commandRelease: ZintlWindowCommandRelease?

  @MainActor
  override init() {
    self.window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 480, height: 300),
      styleMask: [.titled, .closable, .miniaturizable, .resizable],
      backing: .buffered,
      defer: false
    )
    super.init()
    self.window.delegate = self
  }

  @MainActor
  deinit {
    self.clearCommandCallback()
    self.window.close()
  }

  @MainActor
  func setBounds(_ bounds: ZintlRect) {
    self.window.setFrame(
      Self.appkitFrame(fromTopLeftBounds: bounds, screen: self.targetScreen()), display: true)
  }

  @MainActor
  func setSize(width: Double, height: Double) {
    let frame = self.window.frame
    let screen = self.targetScreen()
    let currentTop = Self.topLeftY(fromAppkitFrame: frame, screen: screen)
    self.setBounds(ZintlRect(x: frame.minX, y: currentTop, width: width, height: height))
  }

  @MainActor
  func setPosition(x: Double, y: Double) {
    let frame = self.window.frame
    self.setBounds(ZintlRect(x: x, y: y, width: frame.width, height: frame.height))
  }

  @MainActor
  func setCommands(
    commands: ZintlWindowCommandSet,
    userData: UnsafeRawPointer?,
    callback: ZintlWindowCommandCallback?,
    release: ZintlWindowCommandRelease?
  ) {
    self.clearCommandCallback()
    self.commandSet = commands
    self.commandUserData = userData
    self.commandCallback = callback
    self.commandRelease = release
    self.installCommandsMenuIfActive()
  }

  @MainActor
  func performCommand(_ commandID: String) {
    guard let callback = self.commandCallback else {
      return
    }
    commandID.withCString { commandIDPtr in
      callback(self.commandUserData, commandIDPtr)
    }
  }

  @MainActor
  func windowDidBecomeKey(_ notification: Notification) {
    self.installCommandsMenu()
  }

  @MainActor
  func installCommandsMenuIfActive() {
    if self.window.isKeyWindow || NSApp.keyWindow == nil {
      self.installCommandsMenu()
    }
  }

  @MainActor
  func installCommandsMenu() {
    let mainMenu = NSMenu()
    mainMenu.addItem(Self.appMenuItem())
    self.commandTargets.removeAll()

    for menu in self.commandSet.menus {
      let menuItem = NSMenuItem()
      let submenu = NSMenu(title: menu.title)
      menuItem.submenu = submenu
      mainMenu.addItem(menuItem)

      for command in menu.items {
        let target = ZintlCommandTarget(window: self, commandID: command.id)
        self.commandTargets.append(target)
        let item = NSMenuItem(
          title: command.title,
          action: #selector(ZintlCommandTarget.handleMenuCommand(_:)),
          keyEquivalent: Self.keyEquivalent(command.key)
        )
        item.target = target
        item.keyEquivalentModifierMask = Self.modifierMask(command.modifiers ?? ["cmd"])
        item.isEnabled = command.enabled ?? true
        submenu.addItem(item)
      }
    }

    NSApp.mainMenu = mainMenu
  }

  @MainActor
  func clearCommandCallback() {
    if let release = self.commandRelease, let userData = self.commandUserData {
      release(userData)
    }
    self.commandUserData = nil
    self.commandCallback = nil
    self.commandRelease = nil
  }

  @MainActor
  func targetScreen() -> NSScreen {
    self.window.screen ?? NSScreen.main ?? NSScreen.screens.first!
  }

  static func appkitFrame(fromTopLeftBounds bounds: ZintlRect, screen: NSScreen) -> NSRect {
    NSRect(
      x: bounds.x,
      y: screen.frame.maxY - bounds.y - bounds.height,
      width: bounds.width,
      height: bounds.height
    )
  }

  static func topLeftY(fromAppkitFrame frame: NSRect, screen: NSScreen) -> Double {
    screen.frame.maxY - frame.maxY
  }

  static func keyEquivalent(_ key: String?) -> String {
    guard let key, let first = key.lowercased().first else {
      return ""
    }
    return String(first)
  }

  static func modifierMask(_ modifiers: [String]) -> NSEvent.ModifierFlags {
    var mask: NSEvent.ModifierFlags = []
    for modifier in modifiers {
      switch modifier {
      case "cmd":
        mask.insert(.command)
      case "ctrl":
        mask.insert(.control)
      case "alt":
        mask.insert(.option)
      case "shift":
        mask.insert(.shift)
      default:
        break
      }
    }
    return mask
  }

  static func appMenuItem() -> NSMenuItem {
    let appMenuItem = NSMenuItem()
    let appMenu = NSMenu()
    appMenuItem.submenu = appMenu

    let aboutTitle = "About " + ProcessInfo.processInfo.processName
    appMenu.addItem(
      withTitle: aboutTitle,
      action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)),
      keyEquivalent: ""
    )

    appMenu.addItem(
      withTitle: "Quit",
      action: #selector(NSApplication.terminate(_:)),
      keyEquivalent: "q"
    )

    return appMenuItem
  }
}

@MainActor
class RWgpuSurface {
  var view: NSView
  var metalLayer: CAMetalLayer
  weak var window: NSWindow?

  @MainActor
  init(window: RWindow, rect: ZintlRect) {
    self.window = window.window
    self.view = NSView(frame: RWgpuSurface.nsRect(from: rect))
    self.metalLayer = CAMetalLayer()
    self.view.wantsLayer = true
    self.view.layer = self.metalLayer
    self.updateDrawableSize()
    window.window.contentView?.addSubview(self.view)
  }

  @MainActor
  deinit {
    self.view.removeFromSuperview()
  }

  @MainActor
  func setRect(_ rect: ZintlRect) {
    self.view.frame = RWgpuSurface.nsRect(from: rect)
    self.updateDrawableSize()
  }

  @MainActor
  func drawableSize() -> (UInt32, UInt32) {
    self.updateDrawableSize()
    let size = self.metalLayer.drawableSize
    return (
      UInt32(max(0, size.width.rounded())),
      UInt32(max(0, size.height.rounded()))
    )
  }

  @MainActor
  func updateDrawableSize() {
    let scale = self.window?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 1.0
    self.metalLayer.contentsScale = scale
    self.metalLayer.drawableSize = CGSize(
      width: max(0, self.view.bounds.width * scale),
      height: max(0, self.view.bounds.height * scale)
    )
  }

  static func nsRect(from rect: ZintlRect) -> NSRect {
    NSRect(x: rect.x, y: rect.y, width: rect.width, height: rect.height)
  }
}

@MainActor
@_cdecl("zintlappkit_create_window")
func zintlAppkitCreateWindow() -> UnsafeMutableRawPointer {
  let wnd = RWindow()
  let ptr = Unmanaged.passRetained(wnd).toOpaque()
  return ptr
}

@MainActor
@_cdecl("zintlappkit_show_window")
func zintlAppkitShowWindow(ptr: UnsafeMutableRawPointer) {
  let wnd = Unmanaged<RWindow>.fromOpaque(ptr).takeUnretainedValue()
  NSApp.activate(ignoringOtherApps: true)
  wnd.window.makeKeyAndOrderFront(nil)
}

@MainActor
@_cdecl("zintlappkit_window_set_bounds")
func zintlAppkitWindowSetBounds(ptr: UnsafeRawPointer, bounds: ZintlRect) {
  let wnd = Unmanaged<RWindow>.fromOpaque(ptr).takeUnretainedValue()
  wnd.setBounds(bounds)
}

@MainActor
@_cdecl("zintlappkit_window_set_size")
func zintlAppkitWindowSetSize(ptr: UnsafeRawPointer, width: Double, height: Double) {
  let wnd = Unmanaged<RWindow>.fromOpaque(ptr).takeUnretainedValue()
  wnd.setSize(width: width, height: height)
}

@MainActor
@_cdecl("zintlappkit_window_set_position")
func zintlAppkitWindowSetPosition(ptr: UnsafeRawPointer, x: Double, y: Double) {
  let wnd = Unmanaged<RWindow>.fromOpaque(ptr).takeUnretainedValue()
  wnd.setPosition(x: x, y: y)
}

@MainActor
@_cdecl("zintlappkit_window_set_commands")
func zintlAppkitWindowSetCommands(
  ptr: UnsafeRawPointer,
  commandsJson: UnsafePointer<CChar>,
  userData: UnsafeRawPointer?,
  callback: ZintlWindowCommandCallback?,
  release: ZintlWindowCommandRelease?
) {
  let wnd = Unmanaged<RWindow>.fromOpaque(ptr).takeUnretainedValue()
  let json = String(cString: commandsJson)
  guard let data = json.data(using: .utf8) else {
    release?(userData)
    return
  }
  do {
    let commands = try JSONDecoder().decode(ZintlWindowCommandSet.self, from: data)
    wnd.setCommands(commands: commands, userData: userData, callback: callback, release: release)
  } catch {
    release?(userData)
  }
}

@MainActor
@_cdecl("zintlappkit_destroy_window")
func zintlAppkitDestroyWindow(ptr: UnsafeRawPointer) {
  let _ = Unmanaged<RWindow>.fromOpaque(ptr).takeRetainedValue()
}

@MainActor
@_cdecl("zintlappkit_create_wgpu_surface")
func zintlAppkitCreateWgpuSurface(window: UnsafeRawPointer, rect: ZintlRect)
  -> UnsafeMutableRawPointer
{
  let wnd = Unmanaged<RWindow>.fromOpaque(window).takeUnretainedValue()
  let surface = RWgpuSurface(window: wnd, rect: rect)
  return Unmanaged.passRetained(surface).toOpaque()
}

@MainActor
@_cdecl("zintlappkit_destroy_wgpu_surface")
func zintlAppkitDestroyWgpuSurface(surface: UnsafeRawPointer) {
  let _ = Unmanaged<RWgpuSurface>.fromOpaque(surface).takeRetainedValue()
}

@MainActor
@_cdecl("zintlappkit_wgpu_surface_set_rect")
func zintlAppkitWgpuSurfaceSetRect(surface: UnsafeRawPointer, rect: ZintlRect) {
  let surface = Unmanaged<RWgpuSurface>.fromOpaque(surface).takeUnretainedValue()
  surface.setRect(rect)
}

@MainActor
@_cdecl("zintlappkit_wgpu_surface_drawable_size")
func zintlAppkitWgpuSurfaceDrawableSize(
  surface: UnsafeRawPointer,
  outWidth: UnsafeMutablePointer<UInt32>,
  outHeight: UnsafeMutablePointer<UInt32>
) {
  let surface = Unmanaged<RWgpuSurface>.fromOpaque(surface).takeUnretainedValue()
  let size = surface.drawableSize()
  outWidth.pointee = size.0
  outHeight.pointee = size.1
}

@MainActor
@_cdecl("zintlappkit_wgpu_surface_metal_layer")
func zintlAppkitWgpuSurfaceMetalLayer(surface: UnsafeRawPointer) -> UnsafeMutableRawPointer {
  let surface = Unmanaged<RWgpuSurface>.fromOpaque(surface).takeUnretainedValue()
  return Unmanaged.passUnretained(surface.metalLayer).toOpaque()
}
