import Foundation
import RuntimeJSC

public struct OpLimits: Sendable {
  public let maxInputBytes: UInt32
  public let maxOutputBytes: UInt32
  public let timeoutNanoseconds: UInt64

  public init(
    maxInputBytes: UInt32,
    maxOutputBytes: UInt32,
    timeoutNanoseconds: UInt64 = 30_000_000_000
  ) throws {
    guard maxInputBytes > 0, maxOutputBytes > 0, timeoutNanoseconds > 0,
      timeoutNanoseconds <= 86_400_000_000_000
    else {
      throw EmbeddedRuntimeError.invalidLimits
    }
    self.maxInputBytes = maxInputBytes
    self.maxOutputBytes = maxOutputBytes
    self.timeoutNanoseconds = timeoutNanoseconds
  }
}

public enum EmbeddedRuntimeError: Error, Equatable {
  case invalidLimits
  case invalidName
  case duplicateOp
  case reservedName
  case registryFrozen
  case unknownOp
  case permissionDenied
  case inputTooLarge
  case outputTooLarge
  case cancelled
  case timedOut
  case hostFailed
  case runtimeUnavailable
  case duplicateRuntimeID
}

public struct EmbeddedRuntimeConfiguration: Sendable, Equatable {
  public let permissionTimeoutNanoseconds: UInt64
  public let maximumAuditEvents: Int

  public init(
    permissionTimeoutNanoseconds: UInt64 = 30_000_000_000,
    maximumAuditEvents: Int = 1_024
  ) throws {
    guard permissionTimeoutNanoseconds > 0,
      permissionTimeoutNanoseconds <= 86_400_000_000_000,
      maximumAuditEvents > 0,
      maximumAuditEvents <= 65_536
    else {
      throw EmbeddedRuntimeError.invalidLimits
    }
    self.permissionTimeoutNanoseconds = permissionTimeoutNanoseconds
    self.maximumAuditEvents = maximumAuditEvents
  }
}

public enum EmbeddedRuntimeLifecycle: String, Sendable, Equatable {
  case configured
  case running
  case shuttingDown
  case terminated
}

public enum AuditCategory: String, Sendable, Equatable {
  case lifecycle
  case customOperation
  case directoryPermission
  case permissionPersistence
}

public enum AuditOutcome: String, Sendable, Equatable {
  case started
  case allowed
  case denied
  case succeeded
  case failed
  case cancelled
  case timedOut
  case shutdown
}

/// Redacted audit data. Payloads, locators, handles, blobs, secrets, and native
/// error text are deliberately unrepresentable in this type.
public struct AuditEvent: Sendable, Equatable {
  public let sequence: UInt64
  public let category: AuditCategory
  public let outcome: AuditOutcome
  public let operation: String
  public let requestID: UInt64?
}

public struct PermissionRequest: Sendable {
  public let requestID: UInt64
  public let operation: String
  public let kind: String
  public let requestedScope: Data
  public let requestedRights: UInt64
  public let requestedDirectory: String?
  public let reason: String?
}

public enum PermissionDecision: Sendable {
  case deny
  case allow(scope: Data, rights: UInt64, quota: UInt64)
}

public typealias PermissionResolver =
  @Sendable (PermissionRequest) async throws -> PermissionDecision

public struct HostOpContext: Sendable {
  public let requestID: UInt64
  public let operation: String
  public let permission: String
}

public typealias HostOp = @Sendable (HostOpContext, Data) async throws -> Data

/// Explicit executor for trusted host-op work. It must not run heavy work on
/// the JavaScript serial executor or require MainActor.
public protocol HostOpExecutor: Sendable {
  func execute(_ operation: @escaping @Sendable () async throws -> Data) async throws -> Data
}

/// Default executor backed by an unstructured task, never MainActor implicitly.
public struct TaskHostOpExecutor: HostOpExecutor {
  public init() {}

  public func execute(_ operation: @escaping @Sendable () async throws -> Data) async throws -> Data
  {
    let task = Task.detached(operation: operation)
    return try await withTaskCancellationHandler {
      try await task.value
    } onCancel: {
      task.cancel()
    }
  }
}

private struct OpKey: Hashable {
  let name: String
  let version: UInt32
}

private struct RegisteredOp: Sendable {
  let limits: OpLimits
  let permission: String
  let handler: HostOp
}

/// Configuration-time registry and async embedding API backed by the Rust
/// request/completion lifecycle.
public final class EmbeddedRuntime: @unchecked Sendable {
  public convenience init(
    configuration: EmbeddedRuntimeConfiguration,
    executor: any JavaScriptSerialExecutor,
    hostOpExecutor: any HostOpExecutor = TaskHostOpExecutor(),
    permissionResolver: PermissionResolver? = nil,
    permissionPersistence: PermissionPersistenceConfiguration? = nil
  ) {
    self.init(
      executor: executor,
      hostOpExecutor: hostOpExecutor,
      permissionResolver: permissionResolver,
      permissionTimeoutNanoseconds: configuration.permissionTimeoutNanoseconds,
      permissionPersistence: permissionPersistence,
      maximumAuditEvents: configuration.maximumAuditEvents
    )
  }

  public init(
    executor: any JavaScriptSerialExecutor,
    hostOpExecutor: any HostOpExecutor = TaskHostOpExecutor(),
    permissionResolver: PermissionResolver? = nil,
    permissionTimeoutNanoseconds: UInt64 = 30_000_000_000,
    permissionPersistence: PermissionPersistenceConfiguration? = nil,
    maximumAuditEvents: Int = 1_024
  ) {
    self.executor = executor
    self.hostOpExecutor = hostOpExecutor
    self.permissionResolver = permissionResolver
    self.permissionTimeoutNanoseconds = min(
      max(permissionTimeoutNanoseconds, 1),
      86_400_000_000_000
    )
    self.permissionPersistence = permissionPersistence
    permissionReplayState = permissionPersistence.map {
      PermissionReplayState(maximumEntries: $0.maximumReplayEntries)
    }
    self.maximumAuditEvents = min(max(maximumAuditEvents, 1), 65_536)
  }

  /// Named construction entry point for configuration/build/start lifecycles.
  public static func build(
    configuration: EmbeddedRuntimeConfiguration,
    executor: any JavaScriptSerialExecutor,
    hostOpExecutor: any HostOpExecutor = TaskHostOpExecutor(),
    permissionResolver: PermissionResolver? = nil,
    permissionPersistence: PermissionPersistenceConfiguration? = nil
  ) -> EmbeddedRuntime {
    EmbeddedRuntime(
      configuration: configuration,
      executor: executor,
      hostOpExecutor: hostOpExecutor,
      permissionResolver: permissionResolver,
      permissionPersistence: permissionPersistence
    )
  }

  public var lifecycle: EmbeddedRuntimeLifecycle {
    lock.withLock { lifecycleState }
  }

  /// Returns a bounded oldest-to-newest snapshot containing redacted fields only.
  public func auditSnapshot() -> [AuditEvent] {
    lock.withLock { auditEvents }
  }

  public func registerOp(
    name: String,
    version: UInt32,
    limits: OpLimits,
    permission: String,
    handler: @escaping HostOp
  ) throws {
    guard Self.isNamespaced(name), Self.isNamespaced(permission), version > 0 else {
      throw EmbeddedRuntimeError.invalidName
    }
    guard !name.hasPrefix("zintl.builtin.") else {
      throw EmbeddedRuntimeError.reservedName
    }
    let key = OpKey(name: name, version: version)
    try lock.withLock {
      guard lifecycleState == .configured, !started else {
        throw EmbeddedRuntimeError.registryFrozen
      }
      guard operations[key] == nil else { throw EmbeddedRuntimeError.duplicateOp }
      operations[key] = RegisteredOp(limits: limits, permission: permission, handler: handler)
    }
  }

  /// Freezes registration without starting an engine adapter.
  public func freezeRegistry() {
    lock.withLock { started = true }
  }

  /// Creates the JSC adapter on the configured serial executor. Registered ops
  /// remain permission-guarded and execute on the explicit host-op executor.
  public func makeJavaScriptCoreAdapter(runtimeID: UInt64) async throws -> JavaScriptCoreAdapter {
    try lock.withLock {
      guard lifecycleState == .configured || lifecycleState == .running else {
        throw EmbeddedRuntimeError.runtimeUnavailable
      }
      guard adapters[runtimeID] == nil else { throw EmbeddedRuntimeError.duplicateRuntimeID }
      started = true
      lifecycleState = .running
    }
    recordAudit(category: .lifecycle, outcome: .started, operation: "runtime.start")
    let directoryResolver: DirectoryGrantResolver?
    if let resolver = permissionResolver {
      directoryResolver = {
        (request: DirectoryPermissionRequest) async throws
          -> ApprovedDirectoryGrant in
        do {
          let encoded = try await raceHostOperation(
            timeoutNanoseconds: self.permissionTimeoutNanoseconds
          ) {
            let scope = Data(request.locator.utf8)
            let permissionRequest = PermissionRequest(
              requestID: request.requestID,
              operation: "zintl.builtin.fs.request-directory",
              kind: "zintl.permission.fs.directory",
              requestedScope: scope,
              requestedRights: request.rights,
              requestedDirectory: request.locator,
              reason: "JavaScript requested directory access"
            )
            let decision = try await resolver(permissionRequest)
            guard case .allow(let grantedScope, let rights, let quota) = decision,
              grantedScope == scope,
              rights == request.rights,
              quota > 0
            else {
              throw EmbeddedRuntimeError.permissionDenied
            }
            return try JSONEncoder().encode(
              ApprovedDirectoryGrant(locator: request.locator, rights: rights, quota: quota))
          }
          self.recordAudit(
            category: .directoryPermission,
            outcome: .allowed,
            operation: "zintl.builtin.fs.request-directory",
            requestID: request.requestID
          )
          return try JSONDecoder().decode(ApprovedDirectoryGrant.self, from: encoded)
        } catch EmbeddedRuntimeError.timedOut {
          self.recordAudit(
            category: .directoryPermission, outcome: .timedOut,
            operation: "zintl.builtin.fs.request-directory", requestID: request.requestID)
          throw DirectoryPermissionError.timedOut
        } catch EmbeddedRuntimeError.cancelled {
          self.recordAudit(
            category: .directoryPermission, outcome: .cancelled,
            operation: "zintl.builtin.fs.request-directory", requestID: request.requestID)
          throw DirectoryPermissionError.cancelled
        } catch {
          self.recordAudit(
            category: .directoryPermission, outcome: .denied,
            operation: "zintl.builtin.fs.request-directory", requestID: request.requestID)
          throw DirectoryPermissionError.denied
        }
      }
    } else {
      directoryResolver = nil
    }
    let adapter = try await JavaScriptCoreAdapter.create(
      runtimeID: runtimeID,
      executor: executor,
      directoryResolver: directoryResolver,
      dispatcher: { [weak self] name, input, requestID in
        guard let self else {
          return .failure(
            HostDispatchFailure(
              name: "Error", message: "Runtime is unavailable", code: "RuntimeShuttingDown"))
        }
        do {
          let output = try await self.invokeRegisteredOp(
            name: name, version: 1, requestID: requestID, input: input)
          return .success(output)
        } catch let error as EmbeddedRuntimeError {
          return .failure(Self.hostFailure(error))
        } catch is CancellationError {
          return .failure(
            HostDispatchFailure(name: "Error", message: "Operation cancelled", code: "Cancelled"))
        } catch {
          return .failure(
            HostDispatchFailure(name: "Error", message: "Host operation failed", code: "Internal"))
        }
      }
    )
    let accepted = lock.withLock { () -> Bool in
      guard lifecycleState == .running, adapters[runtimeID] == nil else { return false }
      adapters[runtimeID] = adapter
      return true
    }
    guard accepted else {
      await adapter.shutdown()
      throw EmbeddedRuntimeError.runtimeUnavailable
    }
    return adapter
  }

  /// Starts and tracks a JSC runtime instance owned by this embedding lifecycle.
  public func startJavaScriptCore(runtimeID: UInt64) async throws -> JavaScriptCoreAdapter {
    try await makeJavaScriptCoreAdapter(runtimeID: runtimeID)
  }

  /// Idempotently shuts down every tracked adapter without occupying the main thread.
  public func shutdown() async {
    let shutdownAdapters = lock.withLock { () -> [JavaScriptCoreAdapter] in
      guard lifecycleState != .terminated, lifecycleState != .shuttingDown else { return [] }
      lifecycleState = .shuttingDown
      let values = Array(adapters.values)
      adapters.removeAll()
      return values
    }
    await withTaskGroup(of: Void.self) { group in
      for adapter in shutdownAdapters {
        group.addTask { await adapter.shutdown() }
      }
    }
    lock.withLock { lifecycleState = .terminated }
    recordAudit(category: .lifecycle, outcome: .shutdown, operation: "runtime.shutdown")
  }

  func invokeRegisteredOpForTesting(
    name: String,
    version: UInt32,
    requestID: UInt64,
    input: Data
  ) async throws -> Data {
    try await invokeRegisteredOp(
      name: name, version: version, requestID: requestID, input: input)
  }

  private func invokeRegisteredOp(
    name: String,
    version: UInt32,
    requestID: UInt64,
    input: Data
  ) async throws -> Data {
    recordAudit(
      category: .customOperation, outcome: .started, operation: name, requestID: requestID)
    do {
      let output = try await performRegisteredOp(
        name: name, version: version, requestID: requestID, input: input)
      recordAudit(
        category: .customOperation, outcome: .succeeded, operation: name, requestID: requestID)
      return output
    } catch {
      let outcome: AuditOutcome
      if error as? EmbeddedRuntimeError == .permissionDenied {
        outcome = .denied
      } else if error as? EmbeddedRuntimeError == .timedOut {
        outcome = .timedOut
      } else if error as? EmbeddedRuntimeError == .cancelled {
        outcome = .cancelled
      } else {
        outcome = .failed
      }
      recordAudit(
        category: .customOperation, outcome: outcome, operation: name, requestID: requestID)
      throw error
    }
  }

  private func performRegisteredOp(
    name: String,
    version: UInt32,
    requestID: UInt64,
    input: Data
  ) async throws -> Data {
    let operation = try lock.withLock {
      guard let operation = operations[OpKey(name: name, version: version)] else {
        throw EmbeddedRuntimeError.unknownOp
      }
      return operation
    }
    guard input.count <= Int(operation.limits.maxInputBytes) else {
      throw EmbeddedRuntimeError.inputTooLarge
    }
    let result = try await raceHostOperation(
      timeoutNanoseconds: operation.limits.timeoutNanoseconds
    ) {
      guard let permissionResolver = self.permissionResolver else {
        throw EmbeddedRuntimeError.permissionDenied
      }
      let request = PermissionRequest(
        requestID: requestID,
        operation: name,
        kind: operation.permission,
        requestedScope: Data(),
        requestedRights: 1,
        requestedDirectory: nil,
        reason: nil
      )
      let decision = try await permissionResolver(request)
      guard case .allow(let scope, let rights, let quota) = decision,
        scope == request.requestedScope,
        rights & request.requestedRights == request.requestedRights,
        quota > 0
      else {
        throw EmbeddedRuntimeError.permissionDenied
      }
      try Task.checkCancellation()
      let context = HostOpContext(
        requestID: requestID,
        operation: name,
        permission: operation.permission
      )
      return try await self.hostOpExecutor.execute {
        try Task.checkCancellation()
        return try await operation.handler(context, input)
      }
    }
    guard result.count <= Int(operation.limits.maxOutputBytes) else {
      throw EmbeddedRuntimeError.outputTooLarge
    }
    return result
  }

  private static func hostFailure(_ error: EmbeddedRuntimeError) -> HostDispatchFailure {
    switch error {
    case .permissionDenied:
      HostDispatchFailure(
        name: "PermissionDenied", message: "Operation is not permitted", code: "PermissionDenied")
    case .inputTooLarge, .outputTooLarge, .invalidLimits:
      HostDispatchFailure(
        name: "QuotaExceeded", message: "Operation limit exceeded", code: "QuotaExceeded")
    case .cancelled:
      HostDispatchFailure(name: "Error", message: "Operation cancelled", code: "Cancelled")
    case .timedOut:
      HostDispatchFailure(name: "TimeoutError", message: "Operation timed out", code: "TimedOut")
    case .hostFailed:
      HostDispatchFailure(name: "Error", message: "Host operation failed", code: "Internal")
    case .unknownOp:
      HostDispatchFailure(
        name: "NotSupported", message: "Operation is not available", code: "NotSupported")
    case .invalidName, .duplicateOp, .reservedName, .registryFrozen, .duplicateRuntimeID,
      .runtimeUnavailable:
      HostDispatchFailure(name: "Error", message: "Invalid runtime configuration", code: "Internal")
    }
  }

  private static func isNamespaced(_ value: String) -> Bool {
    value.count <= 192 && value.split(separator: ".").count >= 3
      && value.allSatisfy { $0.isLetter || $0.isNumber || ".-_".contains($0) }
  }

  func recordAudit(
    category: AuditCategory,
    outcome: AuditOutcome,
    operation: String,
    requestID: UInt64? = nil
  ) {
    lock.withLock {
      let sequence = nextAuditSequence
      nextAuditSequence &+= 1
      if auditEvents.count == maximumAuditEvents {
        auditEvents.removeFirst()
      }
      auditEvents.append(
        AuditEvent(
          sequence: sequence,
          category: category,
          outcome: outcome,
          operation: operation,
          requestID: requestID
        ))
    }
  }

  private let executor: any JavaScriptSerialExecutor
  private let hostOpExecutor: any HostOpExecutor
  private let permissionResolver: PermissionResolver?
  private let permissionTimeoutNanoseconds: UInt64
  private let maximumAuditEvents: Int
  let permissionPersistence: PermissionPersistenceConfiguration?
  let permissionReplayState: PermissionReplayState?
  private let lock = NSLock()
  private var started = false
  private var lifecycleState: EmbeddedRuntimeLifecycle = .configured
  private var adapters: [UInt64: JavaScriptCoreAdapter] = [:]
  private var auditEvents: [AuditEvent] = []
  private var nextAuditSequence: UInt64 = 1
  private var operations: [OpKey: RegisteredOp] = [:]
}

private final class HostOperationRace: @unchecked Sendable {
  func install(_ continuation: CheckedContinuation<Result<Data, EmbeddedRuntimeError>, Never>) {
    let completed = lock.withLock { () -> Result<Data, EmbeddedRuntimeError>? in
      if let result { return result }
      self.continuation = continuation
      return nil
    }
    if let completed {
      continuation.resume(returning: completed)
    }
  }

  func setTasks(_ tasks: [Task<Void, Never>]) {
    let alreadyCompleted = lock.withLock { () -> Bool in
      if result != nil { return true }
      self.tasks = tasks
      return false
    }
    if alreadyCompleted {
      for task in tasks {
        task.cancel()
      }
    }
  }

  func finish(_ completed: Result<Data, EmbeddedRuntimeError>) {
    let settlement = lock.withLock {
      () -> (CheckedContinuation<Result<Data, EmbeddedRuntimeError>, Never>?, [Task<Void, Never>])
      in
      guard result == nil else { return (nil, []) }
      result = completed
      let continuation = continuation
      self.continuation = nil
      let tasks = tasks
      self.tasks.removeAll()
      return (continuation, tasks)
    }
    for task in settlement.1 {
      task.cancel()
    }
    settlement.0?.resume(returning: completed)
  }

  private let lock = NSLock()
  private var continuation: CheckedContinuation<Result<Data, EmbeddedRuntimeError>, Never>?
  private var result: Result<Data, EmbeddedRuntimeError>?
  private var tasks: [Task<Void, Never>] = []
}

private func raceHostOperation(
  timeoutNanoseconds: UInt64,
  operation: @escaping @Sendable () async throws -> Data
) async throws -> Data {
  let race = HostOperationRace()
  let result = await withTaskCancellationHandler {
    await withCheckedContinuation { continuation in
      race.install(continuation)
      let operationTask = Task.detached {
        do {
          race.finish(.success(try await operation()))
        } catch is CancellationError {
          race.finish(.failure(.cancelled))
        } catch let error as EmbeddedRuntimeError {
          race.finish(.failure(error))
        } catch {
          race.finish(.failure(.hostFailed))
        }
      }
      let timeoutTask = Task.detached {
        do {
          try await Task.sleep(for: .nanoseconds(Int64(timeoutNanoseconds)))
        } catch {
          return
        }
        race.finish(.failure(.timedOut))
      }
      race.setTasks([operationTask, timeoutTask])
    }
  } onCancel: {
    race.finish(.failure(.cancelled))
  }
  return try result.get()
}
