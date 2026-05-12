import AppKit
import CoreFoundation
import ZintlAppkitSupportTypes

struct State {
    var source: CFRunLoopSource;
    var loop: CFRunLoop;
}

class RAppDelegate: NSObject, NSApplicationDelegate {
    var ud: UnsafeRawPointer;
    var cb: AppCallback;
    
    init(ud: UnsafeRawPointer, cb: AppCallback) {
        self.ud = ud;
        self.cb = cb;
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let mainMenu = NSMenu();

        let appMenuItem = NSMenuItem();
        mainMenu.addItem(appMenuItem);
        let appMenu = NSMenu();
        appMenuItem.submenu = appMenu;
        appMenu.addItem(withTitle: "Quit", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q");

        NSApp.mainMenu = mainMenu;
        
        self.cb.on_launch(self.ud);
    }
    
    func applicationWillTerminate(_ notification: Notification) {
        self.cb.will_terminate(self.ud);
    }
}

@MainActor
class ZintlAppkitSupportState {
    static let shared = ZintlAppkitSupportState();
    
    var state: State?;
}

typealias PerformFn = (@convention(c) (UnsafeRawPointer?) -> Void)?;

class ZintlUdWrapper {
    var cb: PerformFn;
    var ud: UnsafeRawPointer;
    
    init(_ ud: UnsafeRawPointer, cb: PerformFn) {
        self.ud = ud;
        self.cb = cb;
    }
}


func performRustFn(rp: UnsafeMutableRawPointer?) {
    if rp != nil {
        let wrapper = Unmanaged<ZintlUdWrapper>.fromOpaque(rp!).takeRetainedValue();
        wrapper.cb!(wrapper.ud);
    }
}

@MainActor
@_cdecl("zintlappkit_init")
func zintlAppkitInit(ud: UnsafeRawPointer, appcbPtr: UnsafePointer<AppCallback>) {
    let appcb = appcbPtr.pointee;
    
    // init appkit app
    let app = NSApplication.shared;
    app.setActivationPolicy(.regular);
    
    let loop = RunLoop.current.getCFRunLoop();
    
    assert(ZintlAppkitSupportState.shared.state == nil, "ZintlAppkitSupportState is already initialized");
    
    var source_cx = CFRunLoopSourceContext();
    source_cx.info = Unmanaged.passRetained(ZintlUdWrapper(ud, cb: appcb.perform)).toOpaque();
    source_cx.perform = performRustFn;
    let source = CFRunLoopSourceCreate(nil, 1, &source_cx)!;
    CFRunLoopAddSource(loop, source, .commonModes)
    
    let delegate = RAppDelegate(ud: ud, cb: appcb);
    
    app.delegate = delegate;
    
    ZintlAppkitSupportState.shared.state = State(source: source, loop: loop)
}

@_cdecl("zintlappkit_schedule")
func zintlAppkitSchedule() {
    Task { @MainActor in
        guard let state = ZintlAppkitSupportState.shared.state else {
            assertionFailure("ZintlAppkitSupportState is not initialized")
            return
        }
        
        CFRunLoopSourceSignal(state.source);
        CFRunLoopWakeUp(state.loop);
    }
}

@MainActor
@_cdecl("zintlappkit_run")
func zintlAppkitRun() {
    guard let state = ZintlAppkitSupportState.shared.state else {
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
    var window: NSWindow;
    
    @MainActor
    init() {
        self.window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 480, height: 300),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        );
    }
    
    @MainActor
    deinit {
        self.window.close();
    }
}

@MainActor
@_cdecl("zintlappkit_create_window")
func zintlAppkitCreateWindow() -> UnsafeMutableRawPointer {
    let wnd = RWindow();
    let ptr = Unmanaged.passRetained(wnd).toOpaque();
    return ptr;
}

@MainActor
@_cdecl("zintlappkit_show_window")
func zintlAppkitShowWindow(ptr: UnsafeMutableRawPointer) {
    let wnd = Unmanaged<RWindow>.fromOpaque(ptr).takeUnretainedValue();
    wnd.window.makeKeyAndOrderFront(nil);
}

@MainActor
@_cdecl("zintlappkit_destroy_window")
func zintlAppkitDestroyWindow(ptr: UnsafeMutableRawPointer) {
    let _ = Unmanaged<RWindow>.fromOpaque(ptr).takeRetainedValue();
}
