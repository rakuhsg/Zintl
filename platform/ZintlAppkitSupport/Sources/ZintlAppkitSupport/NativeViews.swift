import AppKit
import ZintlAppkitSupportTypes

@MainActor
final class ZintlControlActionTarget: NSObject {
  // These values are immutable in practice after initialization. Swift 6
  // deinits are nonisolated, so the release callback needs unchecked access.
  nonisolated(unsafe) private var userData: UnsafeRawPointer?
  nonisolated(unsafe) private var action: ZintlControlAction?
  nonisolated(unsafe) private var release: ZintlControlRelease?

  init(
    userData: UnsafeRawPointer?,
    action: ZintlControlAction?,
    release: ZintlControlRelease?
  ) {
    self.userData = userData
    self.action = action
    self.release = release
  }

  deinit {
    self.releaseUserData()
  }

  @objc func handleControlAction(_ sender: Any?) {
    self.action?(self.userData)
  }

  nonisolated func releaseUserData() {
    let release = self.release
    self.release = nil
    let userData = self.userData
    self.userData = nil
    self.action = nil
    release?(userData)
  }
}

@MainActor
final class ZintlButton: NSButton {
  var actionTarget: ZintlControlActionTarget?
}

private func zintlView(_ pointer: UnsafeRawPointer) -> NSView {
  Unmanaged<NSView>.fromOpaque(pointer).takeUnretainedValue()
}

private func zintlButton(_ pointer: UnsafeRawPointer) -> ZintlButton {
  Unmanaged<ZintlButton>.fromOpaque(pointer).takeUnretainedValue()
}

private func zintlTextField(_ pointer: UnsafeRawPointer) -> NSTextField {
  Unmanaged<NSTextField>.fromOpaque(pointer).takeUnretainedValue()
}

private func zintlRect(_ rect: ZintlRect) -> NSRect {
  NSRect(x: rect.x, y: rect.y, width: rect.width, height: rect.height)
}

@MainActor
@_cdecl("zintlappkit_window_content_view")
func zintlAppkitWindowContentView(window: UnsafeRawPointer) -> UnsafeMutableRawPointer? {
  let window = Unmanaged<ZintlWindow>.fromOpaque(window).takeUnretainedValue()
  guard let contentView = window.window.contentView else {
    return nil
  }
  return Unmanaged.passUnretained(contentView).toOpaque()
}

@MainActor
@_cdecl("zintlappkit_create_view")
func zintlAppkitCreateView(frame: ZintlRect) -> UnsafeMutableRawPointer {
  Unmanaged.passRetained(NSView(frame: zintlRect(frame))).toOpaque()
}

@MainActor
@_cdecl("zintlappkit_release_view")
func zintlAppkitReleaseView(view: UnsafeRawPointer) {
  Unmanaged<NSView>.fromOpaque(view).release()
}

@MainActor
@_cdecl("zintlappkit_view_add_subview")
func zintlAppkitViewAddSubview(parent: UnsafeRawPointer, child: UnsafeRawPointer) {
  zintlView(parent).addSubview(zintlView(child))
}

@MainActor
@_cdecl("zintlappkit_view_remove_from_superview")
func zintlAppkitViewRemoveFromSuperview(view: UnsafeRawPointer) {
  zintlView(view).removeFromSuperview()
}

@MainActor
@_cdecl("zintlappkit_view_set_frame")
func zintlAppkitViewSetFrame(view: UnsafeRawPointer, frame: ZintlRect) {
  zintlView(view).frame = zintlRect(frame)
}

@MainActor
@_cdecl("zintlappkit_view_set_translates_autoresizing_mask_into_constraints")
func zintlAppkitViewSetTranslatesAutoresizingMaskIntoConstraints(
  view: UnsafeRawPointer,
  enabled: Bool
) {
  zintlView(view).translatesAutoresizingMaskIntoConstraints = enabled
}

@MainActor
@_cdecl("zintlappkit_create_button")
func zintlAppkitCreateButton(title: UnsafePointer<CChar>) -> UnsafeMutableRawPointer {
  let button = ZintlButton(title: String(cString: title), target: nil, action: nil)
  return Unmanaged.passRetained(button).toOpaque()
}

@MainActor
@_cdecl("zintlappkit_button_set_title")
func zintlAppkitButtonSetTitle(button: UnsafeRawPointer, title: UnsafePointer<CChar>) {
  zintlButton(button).title = String(cString: title)
}

@MainActor
@_cdecl("zintlappkit_button_set_action")
func zintlAppkitButtonSetAction(
  button: UnsafeRawPointer,
  userData: UnsafeRawPointer?,
  action: ZintlControlAction?,
  release: ZintlControlRelease?
) {
  let button = zintlButton(button)
  button.actionTarget?.releaseUserData()
  let target = ZintlControlActionTarget(
    userData: userData,
    action: action,
    release: release
  )
  button.actionTarget = target
  button.target = target
  button.action = #selector(ZintlControlActionTarget.handleControlAction(_:))
}

@MainActor
@_cdecl("zintlappkit_button_clear_action")
func zintlAppkitButtonClearAction(button: UnsafeRawPointer) {
  let button = zintlButton(button)
  button.actionTarget?.releaseUserData()
  button.target = nil
  button.action = nil
  button.actionTarget = nil
}

@MainActor
@_cdecl("zintlappkit_create_text_field")
func zintlAppkitCreateTextField(
  value: UnsafePointer<CChar>,
  label: Bool
) -> UnsafeMutableRawPointer {
  let value = String(cString: value)
  let textField =
    label
    ? NSTextField(labelWithString: value)
    : NSTextField(string: value)
  return Unmanaged.passRetained(textField).toOpaque()
}

@MainActor
@_cdecl("zintlappkit_text_field_set_string_value")
func zintlAppkitTextFieldSetStringValue(
  textField: UnsafeRawPointer,
  value: UnsafePointer<CChar>
) {
  zintlTextField(textField).stringValue = String(cString: value)
}

@MainActor
@_cdecl("zintlappkit_text_field_set_placeholder_string")
func zintlAppkitTextFieldSetPlaceholderString(
  textField: UnsafeRawPointer,
  value: UnsafePointer<CChar>?
) {
  zintlTextField(textField).placeholderString = value.map(String.init(cString:))
}

@MainActor
@_cdecl("zintlappkit_text_field_set_editable")
func zintlAppkitTextFieldSetEditable(textField: UnsafeRawPointer, editable: Bool) {
  zintlTextField(textField).isEditable = editable
}

@MainActor
@_cdecl("zintlappkit_text_field_set_selectable")
func zintlAppkitTextFieldSetSelectable(textField: UnsafeRawPointer, selectable: Bool) {
  zintlTextField(textField).isSelectable = selectable
}
