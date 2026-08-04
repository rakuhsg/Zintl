import Foundation
import XCTest

@testable import RuntimeJSCFFI

final class RuntimeJSCFFITests: XCTestCase {
  // Verifies the private C ABI evaluates a Promise and preserves non-consuming size queries.
  func testPromiseEvaluationEvent() throws {
    var engine: UnsafeMutableRawPointer?
    XCTAssertEqual(zjscEngineNew(8, 8, 4096, 65536, nil, nil, &engine), 0)
    defer { zjscEngineFree(engine) }
    XCTAssertEqual(zjscEngineStart(engine), 0)
    let source = Data("Promise.resolve(42)".utf8)
    XCTAssertEqual(
      source.withUnsafeBytes { bytes in
        zjscEngineSubmit(
          engine,
          1,
          bytes.bindMemory(to: UInt8.self).baseAddress,
          bytes.count
        )
      },
      0
    )
    let deadline = Date().addingTimeInterval(2)
    var required = 0
    while Date() < deadline {
      let status = zjscEngineNextEvent(engine, nil, 0, &required)
      if status == 2 { break }
      XCTAssertEqual(status, 1)
      Thread.sleep(forTimeInterval: 0.001)
    }
    XCTAssertGreaterThan(required, 20)
    var event = [UInt8](repeating: 0, count: required)
    XCTAssertEqual(zjscEngineNextEvent(engine, &event, event.count, &required), 0)
    XCTAssertEqual(Array(event.prefix(4)), Array("ZJE1".utf8))
    XCTAssertEqual(zjscEngineShutdown(engine), 0)
  }

  // Verifies an oversized evaluation result produces a bounded terminal error event.
  func testEvaluationQueueOverflowTerminatesEvaluation() throws {
    var engine: UnsafeMutableRawPointer?
    XCTAssertEqual(zjscEngineNew(2, 2, 4096, 32, nil, nil, &engine), 0)
    defer { zjscEngineFree(engine) }
    XCTAssertEqual(zjscEngineStart(engine), 0)
    let source = Data("'x'.repeat(1000)".utf8)
    XCTAssertEqual(
      source.withUnsafeBytes { bytes in
        zjscEngineSubmit(
          engine,
          1,
          bytes.bindMemory(to: UInt8.self).baseAddress,
          bytes.count
        )
      },
      0
    )
    let deadline = Date().addingTimeInterval(2)
    var required = 0
    while Date() < deadline {
      let status = zjscEngineNextEvent(engine, nil, 0, &required)
      if status == 2 { break }
      XCTAssertEqual(status, 1)
      Thread.sleep(forTimeInterval: 0.001)
    }
    var event = [UInt8](repeating: 0, count: required)
    XCTAssertEqual(zjscEngineNextEvent(engine, &event, event.count, &required), 0)
    XCTAssertEqual(Array(event.prefix(4)), Array("ZJE1".utf8))
    XCTAssertEqual(Array(event[6...7]), [0, 2])
    XCTAssertTrue(String(decoding: event, as: UTF8.self).contains("QuotaExceeded"))
  }

  // Verifies evaluation identities cannot alias through JavaScript number conversion.
  func testUnsafeIntegerEvaluationIdentityIsRejected() throws {
    var engine: UnsafeMutableRawPointer?
    XCTAssertEqual(zjscEngineNew(2, 2, 4096, 4096, nil, nil, &engine), 0)
    defer { zjscEngineFree(engine) }
    XCTAssertEqual(zjscEngineStart(engine), 0)
    let source = Data("42".utf8)
    XCTAssertEqual(
      source.withUnsafeBytes { bytes in
        zjscEngineSubmit(
          engine,
          9_007_199_254_740_992,
          bytes.bindMemory(to: UInt8.self).baseAddress,
          bytes.count
        )
      },
      3
    )
  }
}
