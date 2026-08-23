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
}
