import XCTest

@MainActor
final class FullSizeSidebarTests: ZintlUITestCase {
  /// Verifies Store-backed sidebar selection does not resize a full-size content window.
  func testChangingSidebarSelectionPreservesWindowSize() throws {
    try launch(scenario: "full-size-sidebar")

    let window = app.windows["main-window"]
    let selection = app.staticTexts["selected-item"]
    let initialSize = window.frame.size
    XCTAssertTrue(selection.waitForExistence(timeout: 2))
    XCTAssertEqual(selection.value as? String, "Selected: home")

    let settings = app.staticTexts["Settings"]
    XCTAssertTrue(settings.waitForExistence(timeout: 2))
    settings.click()
    XCTAssertEqual(selection.value as? String, "Selected: settings")
    XCTAssertEqual(window.frame.width, initialSize.width, accuracy: 1)
    XCTAssertEqual(window.frame.height, initialSize.height, accuracy: 1)

    let library = app.staticTexts["Library"]
    XCTAssertTrue(library.waitForExistence(timeout: 2))
    library.click()
    XCTAssertEqual(selection.value as? String, "Selected: library")
    XCTAssertEqual(window.frame.width, initialSize.width, accuracy: 1)
    XCTAssertEqual(window.frame.height, initialSize.height, accuracy: 1)
  }
}
