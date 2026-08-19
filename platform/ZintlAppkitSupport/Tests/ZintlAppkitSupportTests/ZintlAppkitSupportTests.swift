import AppKit
import Testing
import ZintlAppkitSupportTypes

@testable import ZintlAppkitSupport

@MainActor
private final class WindowCallbackProbe {
  var didCreate = 0
  var willClose = 0
  var didClose = 0
  var didClick = 0
  var releases = 0
}

@MainActor
private final class CommandCallbackProbe {
  var commandIDs: [String] = []
}

@MainActor
private final class ControlActionProbe {
  var actions = 0
  var releases = 0
}

@MainActor
private final class StringValueProbe {
  var utf8: [UInt8] = []
}

@MainActor
private final class SidebarCallbackProbe {
  var selections: [String] = []
  var releases = 0
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

/// Verifies that the Swift/C string bridge round-trips UTF-8 and interior NUL bytes unchanged.
@Test func nativeStringPreservesUTF8AndInteriorNul() {
  let expected = "AppKit ↔ Rust\0string"
  let actual = withZintlString(expected, zintlString)
  #expect(actual == expected)
}

/// Verifies window callback ordering and ownership, app-state teardown, and menu-command dispatch.
@MainActor
@Test func nativeOwnershipAndLifecycle() throws {
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
    },
    did_click: { userData in
      withProbe(userData) { $0.didClick += 1 }
    },
    release: { userData in
      withProbe(userData) { $0.releases += 1 }
      guard let userData else {
        return
      }
      Unmanaged<WindowCallbackProbe>.fromOpaque(userData).release()
    }
  )
  let window = withUnsafePointer(to: &windowCallbacks) {
    zintlAppkitCreateWindow(userData: retainedProbe.toOpaque(), callback: $0)
  }

  #expect(probe.didCreate == 1)
  let nativeWindow = Unmanaged<ZintlWindow>.fromOpaque(window).takeUnretainedValue()
  #expect(!nativeWindow.window.isReleasedWhenClosed)
  withZintlString("Zintl") { zintlAppkitWindowSetTitle(ptr: window, title: $0) }
  #expect(nativeWindow.window.title == "Zintl")
  nativeWindow.dispatchClickIfOpen()
  let closeButton = try #require(nativeWindow.window.standardWindowButton(.closeButton))
  closeButton.performClick(nil)
  nativeWindow.dispatchClickIfOpen()
  nativeWindow.close()
  zintlAppkitDestroyWindow(ptr: window)
  #expect(probe.willClose == 1)
  #expect(probe.didClose == 1)
  #expect(probe.didClick == 1)
  #expect(probe.releases == 1)

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

  let commandProbe = CommandCallbackProbe()
  let retainedCommandProbe = Unmanaged.passRetained(commandProbe)
  let commandsJSON = """
    {
      "menus": [
        {
          "title": "File",
          "items": [{ "id": "file.new", "title": "New" }]
        }
      ]
    }
    """
  withZintlString(commandsJSON) { commandsJSON in
    zintlAppkitSetCommands(
      commandsJson: commandsJSON,
      userData: retainedCommandProbe.toOpaque(),
      callback: { userData, commandID in
        guard let userData else {
          return
        }
        let probe = Unmanaged<CommandCallbackProbe>.fromOpaque(userData).takeUnretainedValue()
        probe.commandIDs.append(zintlString(commandID))
      },
      release: { userData in
        guard let userData else {
          return
        }
        Unmanaged<CommandCallbackProbe>.fromOpaque(userData).release()
      }
    )
  }

  let commandItem = NSApp.mainMenu!.items.first!.submenu!.items.first!
  #expect(NSApp.sendAction(commandItem.action!, to: commandItem.target, from: commandItem))
  #expect(commandProbe.commandIDs == ["file.new"])

  zintlAppkitDestroy()

  #expect(NSApp.delegate == nil)
  #expect(ZintlAppkitSupportState.shared.state == nil)
}

/// Verifies native view ownership, button actions, Auto Layout, string values, and callback release.
@MainActor
@Test func nativeControlsAndAutoLayout() throws {
  let parent = zintlAppkitCreateView(frame: ZintlRect(x: 0, y: 0, width: 320, height: 200))
  let button = withZintlString("Save") { zintlAppkitCreateButton(title: $0) }
  let textField = withZintlString("") {
    zintlAppkitCreateTextField(value: $0, label: false)
  }

  zintlAppkitViewAddSubview(parent: parent, child: button)
  zintlAppkitViewAddSubview(parent: parent, child: textField)
  zintlAppkitViewSetTranslatesAutoresizingMaskIntoConstraints(view: button, enabled: false)
  zintlAppkitViewSetTranslatesAutoresizingMaskIntoConstraints(view: textField, enabled: false)

  let probe = ControlActionProbe()
  let retainedProbe = Unmanaged.passRetained(probe)
  zintlAppkitButtonSetAction(
    button: button,
    userData: retainedProbe.toOpaque(),
    action: { userData in
      guard let userData else {
        return
      }
      let probe = Unmanaged<ControlActionProbe>.fromOpaque(userData).takeUnretainedValue()
      probe.actions += 1
    },
    release: { userData in
      guard let userData else {
        return
      }
      let retainedProbe = Unmanaged<ControlActionProbe>.fromOpaque(userData)
      retainedProbe.takeUnretainedValue().releases += 1
      retainedProbe.release()
    }
  )

  let constraint = zintlAppkitLayoutConstraintCreate(
    firstView: button,
    firstAttribute: Int32(NSLayoutConstraint.Attribute.leading.rawValue),
    relation: Int32(NSLayoutConstraint.Relation.equal.rawValue),
    secondView: parent,
    secondAttribute: Int32(NSLayoutConstraint.Attribute.leading.rawValue),
    multiplier: 1,
    constant: 20
  )
  zintlAppkitLayoutConstraintSetActive(constraint: constraint, active: true)

  var nativeButton: NSButton? = Unmanaged<NSButton>.fromOpaque(button).takeUnretainedValue()
  var nativeTextField: NSTextField? =
    Unmanaged<NSTextField>.fromOpaque(textField).takeUnretainedValue()
  var nativeConstraint: NSLayoutConstraint? =
    Unmanaged<NSLayoutConstraint>.fromOpaque(constraint).takeUnretainedValue()
  #expect(nativeButton?.superview != nil)
  #expect(nativeConstraint?.isActive == true)
  #expect(nativeConstraint?.constant == 20)

  nativeButton?.performClick(nil)
  #expect(probe.actions == 1)

  let expectedValue = "こんにちは\0Zintl"
  nativeTextField?.stringValue = expectedValue
  let stringProbe = StringValueProbe()
  zintlAppkitTextFieldGetStringValue(
    textField: textField,
    userData: Unmanaged.passUnretained(stringProbe).toOpaque(),
    callback: { userData, value in
      guard let userData else {
        return
      }
      let probe = Unmanaged<StringValueProbe>.fromOpaque(userData).takeUnretainedValue()
      probe.utf8 = Array(zintlString(value).utf8)
    }
  )
  #expect(String(decoding: stringProbe.utf8, as: UTF8.self) == expectedValue)

  nativeButton = nil
  nativeTextField = nil
  nativeConstraint = nil

  zintlAppkitLayoutConstraintSetActive(constraint: constraint, active: false)
  zintlAppkitReleaseLayoutConstraint(constraint: constraint)
  zintlAppkitReleaseView(view: button)
  zintlAppkitReleaseView(view: textField)
  #expect(probe.releases == 0)
  zintlAppkitButtonClearAction(button: button)
  #expect(probe.releases == 1)
  zintlAppkitViewRemoveFromSuperview(view: button)
  zintlAppkitViewRemoveFromSuperview(view: textField)
  zintlAppkitReleaseView(view: parent)
}

/// Verifies sidebar layout, Source List styling, secondary selection, toolbar placement, and cleanup.
@MainActor
@Test func nativeSidebarLifecycle() throws {
  let probe = SidebarCallbackProbe()
  let retainedProbe = Unmanaged.passRetained(probe)
  var callbacks = WindowCallback(
    did_create: { _ in },
    will_close: { _ in },
    did_close: { _ in },
    did_click: { _ in },
    release: { _ in }
  )
  let window = withUnsafePointer(to: &callbacks) {
    zintlAppkitCreateWindow(userData: nil, callback: $0)
  }
  let sidebarJSON = """
    {
      "sections": [
        {
          "title": "Library",
          "items": [
            { "id": "home", "title": "Home", "systemImage": "house" },
            { "id": "recent", "title": "Recent", "systemImage": "clock" }
          ]
        }
      ],
      "selectedId": "home"
    }
    """
  let installed = withZintlString(sidebarJSON) { sidebarJSON in
    zintlAppkitWindowSetSidebar(
      window: window,
      sidebarJson: sidebarJSON,
      userData: retainedProbe.toOpaque(),
      callback: { userData, itemId in
        guard let userData else {
          return
        }
        let probe = Unmanaged<SidebarCallbackProbe>.fromOpaque(userData).takeUnretainedValue()
        probe.selections.append(zintlString(itemId))
      },
      release: { userData in
        guard let userData else {
          return
        }
        let probe = Unmanaged<SidebarCallbackProbe>.fromOpaque(userData)
        probe.takeUnretainedValue().releases += 1
        probe.release()
      }
    )
  }

  #expect(installed)
  let nativeWindow = Unmanaged<ZintlWindow>.fromOpaque(window).takeUnretainedValue()
  let splitViewController = try #require(
    nativeWindow.window.contentViewController as? NSSplitViewController)
  #expect(splitViewController.splitViewItems.count == 2)
  #expect(splitViewController.splitViewItems[0].behavior == .sidebar)
  if #available(macOS 11.0, *) {
    let sidebarScrollView = try #require(
      splitViewController.splitViewItems[0].viewController.view as? NSScrollView)
    let outlineView = try #require(sidebarScrollView.documentView as? NSOutlineView)
    #expect(outlineView.effectiveStyle == .sourceList)
    let selectedRowView = try #require(
      outlineView.rowView(atRow: outlineView.selectedRow, makeIfNecessary: true))
    selectedRowView.isEmphasized = true
    #expect(!selectedRowView.isEmphasized)
  }
  let toolbar = try #require(nativeWindow.window.toolbar)
  #expect(nativeWindow.toolbarDefaultItemIdentifiers(toolbar).contains(.toggleSidebar))
  #expect(toolbar.items.contains { $0.itemIdentifier == .toggleSidebar })
  if #available(macOS 11.0, *) {
    #expect(
      toolbar.items.map(\.itemIdentifier).starts(with: [
        .toggleSidebar, .sidebarTrackingSeparator,
      ]))
  }
  #expect(probe.selections == ["home"])

  zintlAppkitWindowClearSidebar(window: window)
  #expect(nativeWindow.window.contentViewController === nativeWindow.contentController)
  #expect(probe.releases == 1)
  zintlAppkitDestroyWindow(ptr: window)
}
