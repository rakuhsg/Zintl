import XCTest

@MainActor
final class PerformMainTests: ZintlUITestCase {
  /// Verifies Context tasks are delivered and executed by the AppKit main loop.
  func testTaskRunsOnMainThread() throws {
    try launch(scenario: "perform-main")

    let schedule = app.buttons["schedule-main-task"]
    XCTAssertTrue(schedule.waitForExistence(timeout: 2))
    schedule.click()

    let observe = app.buttons["observe-main-task"]
    XCTAssertTrue(observe.waitForExistence(timeout: 2))
    observe.click()

    let status = app.staticTexts["main-task-status"]
    XCTAssertTrue(status.waitForExistence(timeout: 2))
    XCTAssertEqual(status.value as? String, "Performed")
  }
}
