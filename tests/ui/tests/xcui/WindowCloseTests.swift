import XCTest

@MainActor
final class WindowCloseTests: ZintlUITestCase {
  /// Verifies closing the window makes App::run return successfully and exits the app.
  func testClosingWindowExitsCleanly() throws {
    let marker = FileManager.default.temporaryDirectory
      .appendingPathComponent("zintl-ui-clean-exit-\(UUID().uuidString)")
    addTeardownBlock {
      try? FileManager.default.removeItem(at: marker)
    }

    try launch(
      scenario: "window-close",
      environment: ["ZINTL_UI_TEST_EXIT_MARKER": marker.path]
    )

    let window = app.windows["main-window"]
    let closeButton = window.buttons[XCUIIdentifierCloseWindow]
    XCTAssertTrue(closeButton.waitForExistence(timeout: 2))
    closeButton.click()

    XCTAssertTrue(app.wait(for: .notRunning, timeout: 5))
    XCTAssertTrue(
      FileManager.default.fileExists(atPath: marker.path),
      "The app did not return successfully after its window closed"
    )
  }
}
