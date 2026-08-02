import AppKit

private func zintlLayoutView(_ pointer: UnsafeRawPointer) -> NSView {
  Unmanaged<NSView>.fromOpaque(pointer).takeUnretainedValue()
}

private func zintlConstraint(_ pointer: UnsafeRawPointer) -> NSLayoutConstraint {
  Unmanaged<NSLayoutConstraint>.fromOpaque(pointer).takeUnretainedValue()
}

@MainActor
@_cdecl("zintlappkit_layout_constraint_create")
func zintlAppkitLayoutConstraintCreate(
  firstView: UnsafeRawPointer,
  firstAttribute: Int32,
  relation: Int32,
  secondView: UnsafeRawPointer?,
  secondAttribute: Int32,
  multiplier: Double,
  constant: Double
) -> UnsafeMutableRawPointer {
  guard
    let firstAttribute = NSLayoutConstraint.Attribute(rawValue: Int(firstAttribute)),
    let relation = NSLayoutConstraint.Relation(rawValue: Int(relation)),
    let secondAttribute = NSLayoutConstraint.Attribute(rawValue: Int(secondAttribute))
  else {
    preconditionFailure("Invalid Auto Layout attribute or relation")
  }
  let constraint = NSLayoutConstraint(
    item: zintlLayoutView(firstView),
    attribute: firstAttribute,
    relatedBy: relation,
    toItem: secondView.map(zintlLayoutView),
    attribute: secondAttribute,
    multiplier: multiplier,
    constant: constant
  )
  return Unmanaged.passRetained(constraint).toOpaque()
}

@MainActor
@_cdecl("zintlappkit_layout_constraint_set_active")
func zintlAppkitLayoutConstraintSetActive(constraint: UnsafeRawPointer, active: Bool) {
  zintlConstraint(constraint).isActive = active
}

@MainActor
@_cdecl("zintlappkit_layout_constraint_set_priority")
func zintlAppkitLayoutConstraintSetPriority(constraint: UnsafeRawPointer, priority: Float) {
  zintlConstraint(constraint).priority = NSLayoutConstraint.Priority(rawValue: priority)
}

@MainActor
@_cdecl("zintlappkit_release_layout_constraint")
func zintlAppkitReleaseLayoutConstraint(constraint: UnsafeRawPointer) {
  Unmanaged<NSLayoutConstraint>.fromOpaque(constraint).release()
}
