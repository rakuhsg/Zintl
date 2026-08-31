import XCTest

@MainActor
final class HStackLayoutTests: ZintlUITestCase {
  /// Verifies HStack places its children horizontally with the declared gap.
  func testChildrenUseHorizontalLayoutAndSpacing() throws {
    try launch(scenario: "hstack")

    let leading = app.textFields["leading-field"]
    let trailing = app.textFields["trailing-field"]
    XCTAssertTrue(leading.waitForExistence(timeout: 2))
    XCTAssertTrue(trailing.waitForExistence(timeout: 2))

    XCTAssertGreaterThan(trailing.frame.minX, leading.frame.maxX)
    XCTAssertEqual(trailing.frame.midY, leading.frame.midY, accuracy: 1)
    XCTAssertEqual(trailing.frame.minX - leading.frame.minX, 160 + 24, accuracy: 1)
  }

  /// Verifies resizing relayouts children and later input does not restore the initial size.
  func testChildrenRelayoutWhenWindowResizes() throws {
    try launch(scenario: "hstack")

    let window = app.windows["main-window"]
    let leading = app.textFields["leading-field"]
    XCTAssertTrue(leading.waitForExistence(timeout: 2))

    let initialWindowFrame = window.frame
    let initialTopInset = leading.frame.minY - initialWindowFrame.minY
    let resizeHandle = window.coordinate(
      withNormalizedOffset: CGVector(dx: 0.98, dy: 0.98)
    )
    resizeHandle.press(
      forDuration: 0.2,
      thenDragTo: resizeHandle.withOffset(CGVector(dx: 160, dy: 100))
    )

    let resizedWindowFrame = window.frame
    XCTAssertNotEqual(resizedWindowFrame.size, initialWindowFrame.size)
    XCTAssertEqual(
      leading.frame.minY - resizedWindowFrame.minY,
      initialTopInset,
      accuracy: 1
    )

    leading.click()
    leading.typeText("Resized")
    XCTAssertEqual(window.frame.size.width, resizedWindowFrame.size.width, accuracy: 1)
    XCTAssertEqual(window.frame.size.height, resizedWindowFrame.size.height, accuracy: 1)
  }
}
