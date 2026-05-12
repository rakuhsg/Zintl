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

@MainActor
@_cdecl("zintlappkit_init")
func zintlAppkitInit(ud: UnsafeRawPointer, appcbPtr: UnsafePointer<AppCallback>)
{
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
class RWindow {
    var window: NSWindow

    @MainActor
    init() {
        self.window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 480, height: 300),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
    }

    @MainActor
    deinit {
        self.window.close()
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
@_cdecl("zintlappkit_destroy_window")
func zintlAppkitDestroyWindow(ptr: UnsafeMutableRawPointer) {
    let _ = Unmanaged<RWindow>.fromOpaque(ptr).takeRetainedValue()
}

@MainActor
@_cdecl("zintlappkit_create_wgpu_surface")
func zintlAppkitCreateWgpuSurface(window: UnsafeRawPointer, rect: ZintlRect) -> UnsafeMutableRawPointer {
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
