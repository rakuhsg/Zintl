import XCTest

@MainActor
class ZintlUITestCase: XCTestCase {
  private(set) var app: XCUIApplication!

  override func setUpWithError() throws {
    continueAfterFailure = false
  }

  func launch(
    scenario: String,
    file: StaticString = #filePath,
    line: UInt = #line
  ) throws {
    let path = ProcessInfo.processInfo.environment["ZINTL_UI_TEST_APP"] ?? defaultAppPath

    app = XCUIApplication(url: URL(fileURLWithPath: path))
    app.launchArguments = ["--scenario", scenario]
    app.launch()
    let launchedApp = app!
    addTeardownBlock { @MainActor in
      launchedApp.terminate()
    }

    XCTAssertTrue(
      app.windows["main-window"].waitForExistence(timeout: 5),
      "The main window did not appear",
      file: file,
      line: line
    )
  }

  private var defaultAppPath: String {
    URL(fileURLWithPath: #filePath)
      .deletingLastPathComponent()
      .deletingLastPathComponent()
      .deletingLastPathComponent()
      .appendingPathComponent("target/ZintlUITestApp.app")
      .path
  }
}
