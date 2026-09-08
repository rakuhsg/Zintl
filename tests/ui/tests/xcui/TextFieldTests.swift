import XCTest

@MainActor
final class TextFieldTests: ZintlUITestCase {
  /// Verifies native text input updates the Store-backed label.
  func testInputUpdatesStoredValue() throws {
    try launch(scenario: "text-field")

    let input = app.textFields["text-input"]
    XCTAssertTrue(input.waitForExistence(timeout: 2))
    input.click()
    input.typeText("typed value")

    let output = app.staticTexts["stored-value"]
    XCTAssertTrue(output.waitForExistence(timeout: 2))
    XCTAssertEqual(output.value as? String, #"Stored value: "typed value""#)
  }

  /// Verifies multiline input accepts a line break and updates its bound Store.
  func testMultilineInputUpdatesStoredValue() throws {
    try launch(scenario: "text-field")

    let input = app.textFields["multiline-input"]
    XCTAssertTrue(input.waitForExistence(timeout: 2))
    input.click()
    input.typeText("first line")
    input.typeKey(.return, modifierFlags: .option)
    input.typeText("second line")

    XCTAssertEqual(input.value as? String, "first line\nsecond line")
    let output = app.staticTexts["stored-multiline-value"]
    XCTAssertTrue(output.waitForExistence(timeout: 2))
    XCTAssertEqual(
      output.value as? String,
      #"Stored multiline value: "first line\nsecond line""#
    )
  }
}
