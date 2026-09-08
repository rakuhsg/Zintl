import XCTest

@MainActor
final class FullSizeSidebarTests: ZintlUITestCase {
  /// Verifies sidebar reconstruction preserves both declared and user-resized window sizes.
  ///
  /// Each click changes a bound Store, which rebuilds the SidebarState and replaces the native
  /// split-view controller. The replacement must preserve the current full-size content window
  /// frame instead of allowing AppKit to fit the window to the replacement controller.
  func testChangingSidebarSelectionPreservesWindowSize() throws {
    try launch(scenario: "full-size-sidebar")

    let window = app.windows["main-window"]
    let selection = app.staticTexts["selected-item"]
    let initialSize = window.frame.size
    XCTAssertTrue(selection.waitForExistence(timeout: 2))
    XCTAssertEqual(selection.value as? String, "Selected: home")

    // These first two selections exercise the launch-size contract. Waiting for the bound label
    // proves that each click reached Rust, updated the Store, and completed native synchronization
    // before the frame is measured; otherwise a size check could race the Sidebar replacement.
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

    // Resize through the native bottom-right handle and first prove that the frame really changed.
    // This guard prevents the final preservation assertions from passing when UI automation failed
    // to resize the window and merely compared the original size with itself.
    let resizeHandle = window.coordinate(
      withNormalizedOffset: CGVector(dx: 0.98, dy: 0.98)
    )
    resizeHandle.press(
      forDuration: 0.2,
      thenDragTo: resizeHandle.withOffset(CGVector(dx: 160, dy: 100))
    )
    let resizedSize = window.frame.size
    XCTAssertNotEqual(resizedSize, initialSize)

    // Selecting Home performs one more Store update and native Sidebar replacement after the user
    // resize. The frame must remain at the user-selected size rather than returning to Window::new's
    // declared 640x400 bounds or adopting AppKit's split-view fitting size.
    let home = app.staticTexts["Home"]
    XCTAssertTrue(home.waitForExistence(timeout: 2))
    home.click()
    XCTAssertEqual(selection.value as? String, "Selected: home")
    XCTAssertEqual(window.frame.width, resizedSize.width, accuracy: 1)
    XCTAssertEqual(window.frame.height, resizedSize.height, accuracy: 1)
  }
}
