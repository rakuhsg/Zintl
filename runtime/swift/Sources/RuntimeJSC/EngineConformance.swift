import Foundation

/// Minimum reusable engine surface. Future JSC/V8 adapters run the same
/// conformance cases without exposing engine-native values.
public protocol JavaScriptEngineAdapter: Sendable {
  func evaluate(_ source: String) async throws -> JavaScriptEvaluationResult
  func shutdown() async
}

extension JavaScriptCoreAdapter: JavaScriptEngineAdapter {}

public struct EngineConformanceReport: Sendable, Equatable {
  public let synchronousEvaluation: Bool
  public let promiseMicrotasks: Bool
  public let structuredRejection: Bool
  public let ambientAuthorityAbsent: Bool

  public var passed: Bool {
    synchronousEvaluation && promiseMicrotasks && structuredRejection && ambientAuthorityAbsent
  }
}

public enum EngineConformanceHarness {
  /// Runs engine-independent evaluation, Promise, error, and ambient-authority cases.
  public static func run(on adapter: any JavaScriptEngineAdapter) async throws
    -> EngineConformanceReport
  {
    let synchronous = try await adapter.evaluate("40 + 2")
    let microtasks = try await adapter.evaluate(
      "Promise.resolve(1).then(value => value + 1).then(value => value * 3)"
    )
    let rejection = try await adapter.evaluate(
      "Promise.reject(Object.assign(new Error('denied'), { code: 'PermissionDenied' }))"
        + ".catch(error => [error.name, error.code])"
    )
    let ambient = try await adapter.evaluate(
      "[typeof process, typeof require, typeof Deno, typeof fetch]"
    )
    return EngineConformanceReport(
      synchronousEvaluation: synchronous.json == #"{"type":"value","value":42}"#,
      promiseMicrotasks: microtasks.json == #"{"type":"value","value":6}"#,
      structuredRejection: rejection.json
        == #"{"type":"value","value":["Error","PermissionDenied"]}"#,
      ambientAuthorityAbsent: ambient.json
        == #"{"type":"value","value":["undefined","undefined","undefined","undefined"]}"#
    )
  }
}
