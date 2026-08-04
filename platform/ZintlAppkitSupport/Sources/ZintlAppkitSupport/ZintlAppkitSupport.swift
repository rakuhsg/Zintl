import AppKit
import CoreFoundation
import QuartzCore
import ZintlAppkitSupportTypes

struct State {
  var source: CFRunLoopSource
  var loop: CFRunLoop
  var delegate: ZintlAppDelegate
}

class ZintlAppDelegate: NSObject, NSApplicationDelegate {
  var ud: UnsafeRawPointer
  var cb: AppCallback
  var commandSet = ZintlCommandSet(appMenu: nil, menus: [])
  var commandTargets: [ZintlCommandTarget] = []
  var commandUserData: UnsafeRawPointer?
  var commandCallback: ZintlCommandCallback?
  var commandRelease: ZintlCommandRelease?

  init(ud: UnsafeRawPointer, cb: AppCallback) {
    self.ud = ud
    self.cb = cb
  }

  func applicationDidFinishLaunching(_ notification: Notification) {
    self.installCommandsMenu()
    self.cb.on_launch(self.ud)
  }

  func applicationWillTerminate(_ notification: Notification) {
    self.cb.will_terminate(self.ud)
  }

  deinit {
    if let release = self.commandRelease, let userData = self.commandUserData {
      release(userData)
    }
  }

  @MainActor
  func setCommands(
    commands: ZintlCommandSet,
    userData: UnsafeRawPointer?,
    callback: ZintlCommandCallback?,
    release: ZintlCommandRelease?
  ) {
    self.clearCommandCallback()
    self.commandSet = commands
    self.commandUserData = userData
    self.commandCallback = callback
    self.commandRelease = release
    self.installCommandsMenu()
  }

  @MainActor
  func performCommand(_ commandID: String) {
    guard let callback = self.commandCallback else {
      return
    }
    withZintlString(commandID) { commandID in
      callback(self.commandUserData, commandID)
    }
  }

  @MainActor
  func installCommandsMenu() {
    let mainMenu = NSMenu()
    self.commandTargets.removeAll()

    if let appMenu = self.commandSet.appMenu {
      let appMenuItem = NSMenuItem()
      let submenu = NSMenu(title: ProcessInfo.processInfo.processName)
      appMenuItem.submenu = submenu
      mainMenu.addItem(appMenuItem)
      self.installCommandItems(appMenu.items, into: submenu)
    }

    for menu in self.commandSet.menus {
      let menuItem = NSMenuItem()
      let submenu = NSMenu(title: menu.title)
      menuItem.submenu = submenu
      mainMenu.addItem(menuItem)
      self.installCommandItems(menu.items, into: submenu)
    }

    NSApp.mainMenu = mainMenu
  }

  @MainActor
  func installCommandItems(_ commands: [ZintlCommandItem], into menu: NSMenu) {
    for command in commands {
      let target = ZintlCommandTarget(app: self, commandID: command.id, role: command.role)
      self.commandTargets.append(target)
      let item = NSMenuItem(
        title: command.title,
        action: #selector(ZintlCommandTarget.handleMenuCommand(_:)),
        keyEquivalent: ZintlCommandTarget.keyEquivalent(command.key)
      )
      item.target = target
      item.keyEquivalentModifierMask = ZintlCommandTarget.modifierMask(
        command.modifiers ?? ["cmd"])
      item.isEnabled = command.enabled ?? true
      ZintlCommandTarget.setRoleSymbol(command.role, on: item, title: command.title)
      menu.addItem(item)
    }
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

struct ZintlCommandSet: Decodable {
  var appMenu: ZintlWindowAppMenu?
  var menus: [ZintlCommandMenu]
}

struct ZintlWindowAppMenu: Decodable {
  var items: [ZintlCommandItem]
}

struct ZintlCommandMenu: Decodable {
  var title: String
  var items: [ZintlCommandItem]
}

struct ZintlCommandItem: Decodable {
  var id: String?
  var title: String
  var role: String?
  var key: String?
  var modifiers: [String]?
  var enabled: Bool?
}

@MainActor
class ZintlCommandTarget: NSObject {
  weak var app: ZintlAppDelegate?
  let commandID: String?
  let role: String?

  init(app: ZintlAppDelegate, commandID: String?, role: String?) {
    self.app = app
    self.commandID = commandID
    self.role = role
  }

  @objc func handleMenuCommand(_ sender: Any?) {
    if let role {
      Self.performRole(role)
      return
    }
    guard let commandID else {
      return
    }
    self.app?.performCommand(commandID)
  }

  static func performRole(_ role: String) {
    switch role {
    case "about":
      NSApp.orderFrontStandardAboutPanel(nil)
    case "quit":
      NSApp.terminate(nil)
    default:
      break
    }
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

  static func setRoleSymbol(_ role: String?, on item: NSMenuItem, title: String) {
    guard #available(macOS 11.0, *),
      let role,
      let symbolName = Self.symbolName(forRole: role),
      let image = NSImage(systemSymbolName: symbolName, accessibilityDescription: title)
    else {
      return
    }

    image.isTemplate = true
    item.image = image
  }

  static func symbolName(forRole role: String) -> String? {
    switch role {
    case "about":
      return "info.circle"
    case "quit":
      return "xmark.rectangle"
    default:
      return nil
    }
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

  var sourceCx = CFRunLoopSourceContext()
  sourceCx.info = Unmanaged.passRetained(
    ZintlUdWrapper(ud, cb: appcb.perform)
  ).toOpaque()
  sourceCx.perform = performRustFn
  sourceCx.release = releaseRustFn
  let source = CFRunLoopSourceCreate(nil, 1, &sourceCx)!
  CFRunLoopAddSource(loop, source, .commonModes)

  let delegate = ZintlAppDelegate(ud: ud, cb: appcb)

  app.delegate = delegate

  ZintlAppkitSupportState.shared.state = State(source: source, loop: loop, delegate: delegate)
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
  if let state = ZintlAppkitSupportState.shared.state {
    CFRunLoopRemoveSource(state.loop, state.source, .commonModes)
    CFRunLoopSourceInvalidate(state.source)
  }
  NSApp.delegate = nil
  ZintlAppkitSupportState.shared.state = nil
}

@MainActor
class ZintlWindow: NSObject, NSWindowDelegate, NSToolbarDelegate {
  var window: NSWindow
  let contentController: NSViewController
  var sidebarController: ZintlSidebarViewController?
  var splitViewController: NSSplitViewController?
  var ownsSidebarToolbar = false
  var insertedSidebarToolbarItem = false
  var insertedSidebarTrackingSeparator = false
  var userData: UnsafeRawPointer?
  var callback: WindowCallback?
  var releaseCallback: ZintlWindowRelease?
  var clickMonitor: Any?
  var isClosed = false

  @MainActor
  init(userData: UnsafeRawPointer?, callback: WindowCallback?) {
    self.userData = userData
    self.callback = callback
    self.releaseCallback = callback?.release
    self.contentController = NSViewController()
    self.contentController.view = NSView()
    self.window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 480, height: 300),
      styleMask: [.titled, .closable, .miniaturizable, .resizable],
      backing: .buffered,
      defer: false
    )
    self.window.isReleasedWhenClosed = false
    super.init()
    self.window.contentViewController = self.contentController
    self.window.delegate = self
    self.clickMonitor = NSEvent.addLocalMonitorForEvents(matching: .leftMouseUp) {
      [weak self] event in
      guard let self, !self.isClosed, event.window === self.window else {
        return event
      }
      self.dispatchClickIfOpen()
      return event
    }
    self.callback?.did_create(self.userData)
  }

  @MainActor
  deinit {
    self.close()
    self.releaseUserData()
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
  func windowWillClose(_ notification: Notification) {
    self.completeCloseCallbacks()
  }

  @MainActor
  func close() {
    guard !self.isClosed else {
      return
    }
    self.completeCloseCallbacks()
    self.window.close()
  }

  @MainActor
  func completeCloseCallbacks() {
    guard !self.isClosed else {
      return
    }
    self.isClosed = true
    if let clickMonitor = self.clickMonitor {
      NSEvent.removeMonitor(clickMonitor)
      self.clickMonitor = nil
    }
    let callback = self.callback
    self.callback = nil
    callback?.will_close(self.userData)
    callback?.did_close(self.userData)
  }

  @MainActor
  func dispatchClickIfOpen() {
    guard !self.isClosed else {
      return
    }
    self.callback?.did_click(self.userData)
  }

  @MainActor
  func releaseUserData() {
    let release = self.releaseCallback
    self.releaseCallback = nil
    let userData = self.userData
    self.userData = nil
    release?(userData)
  }

  @MainActor
  func setSidebar(_ controller: ZintlSidebarViewController) {
    self.clearSidebar()

    let splitViewController = NSSplitViewController()
    let sidebarItem = NSSplitViewItem(sidebarWithViewController: controller)
    sidebarItem.minimumThickness = 180
    sidebarItem.maximumThickness = 360
    sidebarItem.canCollapse = true
    splitViewController.addSplitViewItem(sidebarItem)
    splitViewController.addSplitViewItem(
      NSSplitViewItem(viewController: self.contentController)
    )

    self.sidebarController = controller
    self.splitViewController = splitViewController
    self.window.styleMask.insert(.fullSizeContentView)
    self.window.titlebarAppearsTransparent = true
    if #available(macOS 11.0, *) {
      self.window.toolbarStyle = .unified
    }
    self.window.contentViewController = splitViewController
    self.installSidebarToolbar()
  }

  @MainActor
  func clearSidebar() {
    guard let splitViewController = self.splitViewController else {
      return
    }
    for item in splitViewController.splitViewItems.reversed() {
      splitViewController.removeSplitViewItem(item)
    }
    self.window.contentViewController = self.contentController
    self.sidebarController?.releaseUserData()
    self.splitViewController = nil
    self.sidebarController = nil
    if self.ownsSidebarToolbar {
      self.window.toolbar = nil
      self.ownsSidebarToolbar = false
    } else if self.insertedSidebarToolbarItem,
      let index = self.window.toolbar?.items.firstIndex(where: {
        $0.itemIdentifier == .toggleSidebar
      })
    {
      self.window.toolbar?.removeItem(at: index)
    }
    if self.insertedSidebarTrackingSeparator,
      let index = self.window.toolbar?.items.firstIndex(where: {
        if #available(macOS 11.0, *) {
          return $0.itemIdentifier == .sidebarTrackingSeparator
        }
        return false
      })
    {
      self.window.toolbar?.removeItem(at: index)
    }
    self.insertedSidebarToolbarItem = false
    self.insertedSidebarTrackingSeparator = false
    self.window.styleMask.remove(.fullSizeContentView)
    self.window.titlebarAppearsTransparent = false
    if #available(macOS 11.0, *) {
      self.window.toolbarStyle = .automatic
    }
  }

  @MainActor
  private func installSidebarToolbar() {
    if self.window.toolbar == nil {
      let toolbar = NSToolbar(identifier: "zintl.sidebar.toolbar")
      toolbar.delegate = self
      toolbar.displayMode = .iconOnly
      self.window.toolbar = toolbar
      toolbar.insertItem(withItemIdentifier: .toggleSidebar, at: 0)
      if #available(macOS 11.0, *) {
        toolbar.insertItem(withItemIdentifier: .sidebarTrackingSeparator, at: 1)
      }
      self.ownsSidebarToolbar = true
    } else {
      guard let toolbar = self.window.toolbar else {
        return
      }
      if !toolbar.items.contains(where: { $0.itemIdentifier == .toggleSidebar }) {
        toolbar.insertItem(withItemIdentifier: .toggleSidebar, at: 0)
        self.insertedSidebarToolbarItem = true
      }
      if #available(macOS 11.0, *),
        !toolbar.items.contains(where: { $0.itemIdentifier == .sidebarTrackingSeparator })
      {
        toolbar.insertItem(
          withItemIdentifier: .sidebarTrackingSeparator,
          at: min(1, toolbar.items.count)
        )
        self.insertedSidebarTrackingSeparator = true
      }
    }
  }

  @MainActor
  func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
    var identifiers: [NSToolbarItem.Identifier] = [.toggleSidebar]
    if #available(macOS 11.0, *) {
      identifiers.append(.sidebarTrackingSeparator)
    }
    identifiers.append(.flexibleSpace)
    return identifiers
  }

  @MainActor
  func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
    var identifiers: [NSToolbarItem.Identifier] = [.toggleSidebar]
    if #available(macOS 11.0, *) {
      identifiers.append(.sidebarTrackingSeparator)
    }
    identifiers.append(contentsOf: [.flexibleSpace, .space])
    return identifiers
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

}

@MainActor
class ZintlWgpuSurface {
  var view: NSView
  var metalLayer: CAMetalLayer
  weak var window: NSWindow?

  @MainActor
  init(window: ZintlWindow, rect: ZintlRect) {
    self.window = window.window
    self.view = NSView(frame: ZintlWgpuSurface.nsRect(from: rect))
    self.metalLayer = CAMetalLayer()
    self.view.wantsLayer = true
    self.view.layer = self.metalLayer
    self.updateDrawableSize()
    window.contentController.view.addSubview(self.view)
  }

  @MainActor
  deinit {
    self.view.removeFromSuperview()
  }

  @MainActor
  func setRect(_ rect: ZintlRect) {
    self.view.frame = ZintlWgpuSurface.nsRect(from: rect)
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
func zintlAppkitCreateWindow(
  userData: UnsafeRawPointer?,
  callback: UnsafePointer<WindowCallback>?
) -> UnsafeMutableRawPointer {
  let wnd = ZintlWindow(userData: userData, callback: callback?.pointee)
  let ptr = Unmanaged.passRetained(wnd).toOpaque()
  return ptr
}

@MainActor
@_cdecl("zintlappkit_show_window")
func zintlAppkitShowWindow(ptr: UnsafeMutableRawPointer) {
  let wnd = Unmanaged<ZintlWindow>.fromOpaque(ptr).takeUnretainedValue()
  NSApp.activate(ignoringOtherApps: true)
  wnd.window.makeKeyAndOrderFront(nil)
}

@MainActor
@_cdecl("zintlappkit_window_set_bounds")
func zintlAppkitWindowSetBounds(ptr: UnsafeRawPointer, bounds: ZintlRect) {
  let wnd = Unmanaged<ZintlWindow>.fromOpaque(ptr).takeUnretainedValue()
  wnd.setBounds(bounds)
}

@MainActor
@_cdecl("zintlappkit_window_set_size")
func zintlAppkitWindowSetSize(ptr: UnsafeRawPointer, width: Double, height: Double) {
  let wnd = Unmanaged<ZintlWindow>.fromOpaque(ptr).takeUnretainedValue()
  wnd.setSize(width: width, height: height)
}

@MainActor
@_cdecl("zintlappkit_window_set_position")
func zintlAppkitWindowSetPosition(ptr: UnsafeRawPointer, x: Double, y: Double) {
  let wnd = Unmanaged<ZintlWindow>.fromOpaque(ptr).takeUnretainedValue()
  wnd.setPosition(x: x, y: y)
}

@MainActor
@_cdecl("zintlappkit_set_commands")
func zintlAppkitSetCommands(
  commandsJson: ZintlString,
  userData: UnsafeRawPointer?,
  callback: ZintlCommandCallback?,
  release: ZintlCommandRelease?
) {
  let json = zintlString(commandsJson)
  guard let data = json.data(using: .utf8) else {
    release?(userData)
    return
  }
  do {
    let commands = try JSONDecoder().decode(ZintlCommandSet.self, from: data)
    guard let delegate = ZintlAppkitSupportState.shared.state?.delegate else {
      release?(userData)
      return
    }
    delegate.setCommands(
      commands: commands, userData: userData, callback: callback, release: release)
  } catch {
    release?(userData)
  }
}

@MainActor
@_cdecl("zintlappkit_destroy_window")
func zintlAppkitDestroyWindow(ptr: UnsafeRawPointer) {
  let window = Unmanaged<ZintlWindow>.fromOpaque(ptr)
  let value = window.takeUnretainedValue()
  value.close()
  value.releaseUserData()
  window.release()
}

@MainActor
@_cdecl("zintlappkit_create_wgpu_surface")
func zintlAppkitCreateWgpuSurface(window: UnsafeRawPointer, rect: ZintlRect)
  -> UnsafeMutableRawPointer
{
  let wnd = Unmanaged<ZintlWindow>.fromOpaque(window).takeUnretainedValue()
  let surface = ZintlWgpuSurface(window: wnd, rect: rect)
  return Unmanaged.passRetained(surface).toOpaque()
}

@MainActor
@_cdecl("zintlappkit_destroy_wgpu_surface")
func zintlAppkitDestroyWgpuSurface(surface: UnsafeRawPointer) {
  Unmanaged<ZintlWgpuSurface>.fromOpaque(surface).release()
}

@MainActor
@_cdecl("zintlappkit_wgpu_surface_set_rect")
func zintlAppkitWgpuSurfaceSetRect(surface: UnsafeRawPointer, rect: ZintlRect) {
  let surface = Unmanaged<ZintlWgpuSurface>.fromOpaque(surface).takeUnretainedValue()
  surface.setRect(rect)
}

@MainActor
@_cdecl("zintlappkit_wgpu_surface_drawable_size")
func zintlAppkitWgpuSurfaceDrawableSize(
  surface: UnsafeRawPointer,
  outWidth: UnsafeMutablePointer<UInt32>,
  outHeight: UnsafeMutablePointer<UInt32>
) {
  let surface = Unmanaged<ZintlWgpuSurface>.fromOpaque(surface).takeUnretainedValue()
  let size = surface.drawableSize()
  outWidth.pointee = size.0
  outHeight.pointee = size.1
}

@MainActor
@_cdecl("zintlappkit_wgpu_surface_metal_layer")
func zintlAppkitWgpuSurfaceMetalLayer(surface: UnsafeRawPointer) -> UnsafeMutableRawPointer {
  let surface = Unmanaged<ZintlWgpuSurface>.fromOpaque(surface).takeUnretainedValue()
  return Unmanaged.passUnretained(surface.metalLayer).toOpaque()
}

@MainActor
@_cdecl("zintlappkit_wgpu_surface_view")
func zintlAppkitWgpuSurfaceView(surface: UnsafeRawPointer) -> UnsafeMutableRawPointer {
  let surface = Unmanaged<ZintlWgpuSurface>.fromOpaque(surface).takeUnretainedValue()
  return Unmanaged.passUnretained(surface.view).toOpaque()
}
