import AppKit
import Testing
import ZintlAppkitSupportTypes

@testable import ZintlAppkitSupport

@MainActor
private final class WindowCallbackProbe {
  var didCreate = 0
  var willClose = 0
  var didClose = 0
}

private func withProbe(
  _ userData: UnsafeRawPointer?,
  body: (WindowCallbackProbe) -> Void
) {
  guard let userData else {
    return
  }
  body(Unmanaged<WindowCallbackProbe>.fromOpaque(userData).takeUnretainedValue())
}

@MainActor
@Test func nativeOwnershipAndLifecycle() {
  let probe = WindowCallbackProbe()
  let retainedProbe = Unmanaged.passRetained(probe)
  var windowCallbacks = WindowCallback(
    did_create: { userData in
      withProbe(userData) { $0.didCreate += 1 }
    },
    will_close: { userData in
      withProbe(userData) { $0.willClose += 1 }
    },
    did_close: { userData in
      withProbe(userData) { $0.didClose += 1 }
    }
  )
  let window = withUnsafePointer(to: &windowCallbacks) {
    zintlAppkitCreateWindow(userData: retainedProbe.toOpaque(), callback: $0)
  }

  #expect(probe.didCreate == 1)
  zintlAppkitDestroyWindow(ptr: window)
  #expect(probe.willClose == 1)
  #expect(probe.didClose == 1)

  retainedProbe.release()

  var callbacks = AppCallback(
    on_launch: { _ in },
    perform: { _ in },
    will_terminate: { _ in }
  )
  let userData = UnsafeRawPointer(bitPattern: 1)!

  withUnsafePointer(to: &callbacks) {
    zintlAppkitInit(ud: userData, appcbPtr: $0)
  }

  #expect(NSApp.delegate != nil)
  #expect(ZintlAppkitSupportState.shared.state != nil)

  zintlAppkitDestroy()

  #expect(NSApp.delegate == nil)
  #expect(ZintlAppkitSupportState.shared.state == nil)
}
