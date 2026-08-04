import CRuntimeFFI
import Dispatch
import Foundation
import JavaScriptCore
import RuntimeJSCShim

/// Serial execution boundary supplied by the embedder.
/// RuntimeJSC never assumes this executor is the main queue.
public protocol JavaScriptSerialExecutor: Sendable {
  func enqueue(_ operation: @escaping @Sendable () -> Void)
  func preconditionIsCurrent()
}

/// A ready-to-use non-main serial queue executor.
public final class DispatchQueueJavaScriptExecutor: JavaScriptSerialExecutor, @unchecked Sendable {
  public init(label: String, qos: DispatchQoS = .userInitiated) {
    queue = DispatchQueue(label: label, qos: qos)
    queue.setSpecific(key: key, value: identity)
  }

  public func enqueue(_ operation: @escaping @Sendable () -> Void) {
    queue.async(execute: operation)
  }

  public func preconditionIsCurrent() {
    dispatchPrecondition(condition: .onQueue(queue))
  }

  private let queue: DispatchQueue
  private let key = DispatchSpecificKey<UInt64>()
  private let identity: UInt64 = 1
}

/// Engine-neutral bytes submitted to the Rust op protocol.
public struct EncodedOpRequest: Sendable {
  public let requestID: UInt64
  public let opID: UInt32
  public let payload: Data

  public init(requestID: UInt64, opID: UInt32, payload: Data) {
    self.requestID = requestID
    self.opID = opID
    self.payload = payload
  }
}

public struct HostDispatchFailure: Error, Sendable, Equatable, Codable {
  public let name: String
  public let message: String
  public let code: String

  public init(name: String, message: String, code: String) {
    self.name = name
    self.message = message
    self.code = code
  }
}

public typealias HostOperationDispatcher =
  @Sendable (_ name: String, _ input: Data, _ requestID: UInt64) async -> Result<
    Data, HostDispatchFailure
  >

public struct DirectoryPermissionRequest: Sendable, Equatable {
  public let requestID: UInt64
  public let locator: String
  public let rights: UInt64

  public init(requestID: UInt64, locator: String, rights: UInt64) {
    self.requestID = requestID
    self.locator = locator
    self.rights = rights
  }
}

public struct ApprovedDirectoryGrant: Sendable, Equatable, Codable {
  public let locator: String
  public let rights: UInt64
  public let quota: UInt64

  public init(locator: String, rights: UInt64, quota: UInt64) {
    self.locator = locator
    self.rights = rights
    self.quota = quota
  }
}

public typealias DirectoryGrantResolver =
  @Sendable (DirectoryPermissionRequest) async throws -> ApprovedDirectoryGrant

public enum DirectoryPermissionError: Error, Sendable {
  case denied
  case timedOut
  case cancelled
}

/// Trusted Swift handle for one runtime-local directory capability. Its native
/// object identity is private and is never serialized or exposed to JavaScript.
public struct DirectoryPermissionHandle: Sendable {
  public let locator: String
  public let rights: UInt64
  public let quota: UInt64

  fileprivate let runtimeID: UInt64
  fileprivate let objectID: UInt64
}

public struct JavaScriptEvaluationResult: Sendable, Equatable {
  public let json: String
}

public struct JavaScriptException: Error, Sendable, Equatable {
  public let name: String
  public let message: String
  public let stack: String?
  public let code: String
}

public enum HostObjectKind: UInt32, Sendable {
  case capability = 1
  case resource = 2
  case directory = 3
  case file = 4
}

private enum PromiseResultKind {
  case bytes
  case directory
  case file
}

private struct PendingPromise {
  let resolve: JSValue
  let reject: JSValue
  let task: Task<Void, Never>
  let resultKind: PromiseResultKind
}

private struct RuntimeHandle: @unchecked Sendable {
  let pointer: OpaquePointer
}

private struct DecodedCompletion {
  let requestID: UInt64
  let status: UInt32
  let payload: Data
}

private final class AdapterState: @unchecked Sendable {
  var context: JSContext?
  var runtime: RuntimeHandle?
  var evaluateFunction: JSValue?
  var decorateHostObjectFunction: JSValue?
  var makeErrorFunction: JSValue?
  var nextRequestID: UInt64 = 1
  var nextEvaluationID: UInt64 = 1
  var promises: [UInt64: PendingPromise] = [:]
  var evaluations: [UInt64: CheckedContinuation<JavaScriptEvaluationResult, Error>] = [:]
  var hostObjectGlobals: Set<String> = []
  var shuttingDown = false
}

private func completionNotifier(userData: UnsafeMutableRawPointer?) {
  guard let userData else { return }
  let adapter = Unmanaged<JavaScriptCoreAdapter>.fromOpaque(userData).takeUnretainedValue()
  adapter.scheduleCompletionDrain()
}

/// JSC adapter whose context, values, callbacks, and Promise registry are
/// confined to one embedder-provided serial executor.
public final class JavaScriptCoreAdapter: @unchecked Sendable {
  public static func create(
    runtimeID: UInt64,
    executor: any JavaScriptSerialExecutor,
    directoryResolver: DirectoryGrantResolver? = nil,
    dispatcher: @escaping HostOperationDispatcher
  ) async throws -> JavaScriptCoreAdapter {
    guard runtimeID != 0 else {
      throw JavaScriptException(
        name: "TypeError", message: "Invalid runtime identity", stack: nil, code: "InvalidArgument"
      )
    }
    let adapter = JavaScriptCoreAdapter(
      runtimeID: runtimeID,
      executor: executor,
      dispatcher: dispatcher,
      directoryResolver: directoryResolver
    )
    try await adapter.initialize()
    return adapter
  }

  public func evaluate(_ source: String) async throws -> JavaScriptEvaluationResult {
    try await withCheckedThrowingContinuation { continuation in
      executor.enqueue { [self] in
        executor.preconditionIsCurrent()
        guard !state.shuttingDown, let function = state.evaluateFunction else {
          continuation.resume(throwing: shutdownException())
          return
        }
        guard source.utf8.count <= maxScriptBytes else {
          continuation.resume(
            throwing: JavaScriptException(
              name: "RangeError", message: "Script exceeds configured limit", stack: nil,
              code: "QuotaExceeded"))
          return
        }
        guard state.evaluations.count < maxInflightEvaluations else {
          continuation.resume(
            throwing: JavaScriptException(
              name: "QuotaExceeded", message: "Evaluation limit exceeded", stack: nil,
              code: "QuotaExceeded"))
          return
        }
        let evaluationID = state.nextEvaluationID
        guard evaluationID < 9_007_199_254_740_991 else {
          continuation.resume(throwing: internalException("Evaluation identity exhausted"))
          return
        }
        state.nextEvaluationID += 1
        state.evaluations[evaluationID] = continuation
        _ = function.call(withArguments: [Double(evaluationID), source])
        if let exception = state.context?.exception {
          state.context?.exception = nil
          state.evaluations.removeValue(forKey: evaluationID)?.resume(
            throwing: structuredException(exception, code: "JavaScript"))
        }
      }
    }
  }

  /// Requests and opens a directory capability for trusted embedding code.
  /// The async callback and blocking open never occupy the JavaScript executor.
  public func requestDirectoryPermission(
    locator: String,
    rights: UInt64
  ) async throws -> DirectoryPermissionHandle {
    guard !locator.isEmpty, locator.utf8.count <= maxFilesystemPathBytes,
      rights > 0, rights & ~maxFilesystemRights == 0,
      let resolver = directoryResolver
    else {
      throw permissionDeniedException()
    }
    let requestID = try await onExecutorThrowing { [self] in
      guard !state.shuttingDown, state.runtime != nil else { throw shutdownException() }
      let requestID = state.nextRequestID
      guard requestID < UInt64.max else {
        throw internalException("Request identity exhausted")
      }
      state.nextRequestID += 1
      return requestID
    }
    let grant: ApprovedDirectoryGrant
    do {
      grant = try await resolver(
        DirectoryPermissionRequest(requestID: requestID, locator: locator, rights: rights))
    } catch {
      throw permissionDeniedException()
    }
    guard grant.locator == locator, grant.rights == rights, grant.quota > 0 else {
      throw permissionDeniedException()
    }
    let objectID = try await openDirectoryForHost(
      locator: locator,
      rights: rights,
      authenticatedIdentity: nil
    )
    return DirectoryPermissionHandle(
      locator: locator,
      rights: rights,
      quota: grant.quota,
      runtimeID: runtimeID,
      objectID: objectID
    )
  }

  /// Returns fixed-width identity for inclusion in an authenticated envelope.
  public func directoryIdentity(_ permission: DirectoryPermissionHandle) async throws -> Data {
    try validate(permission)
    return try await onExecutorThrowing { [self] in
      guard !state.shuttingDown, let runtime = state.runtime else { throw shutdownException() }
      var identity = Data(count: 16)
      let status = identity.withUnsafeMutableBytes { buffer in
        rt_runtime_fs_directory_identity(
          runtime.pointer,
          permission.objectID,
          buffer.bindMemory(to: UInt8.self).baseAddress,
          buffer.count
        )
      }
      guard status.status == RT_OK else { throw Self.exceptionForOperationStatus(status) }
      return identity
    }
  }

  /// Reopens a previously authenticated directory and mints fresh local authority.
  public func importDirectoryPermission(
    locator: String,
    rights: UInt64,
    quota: UInt64,
    authenticatedIdentity: Data
  ) async throws -> DirectoryPermissionHandle {
    guard quota > 0, authenticatedIdentity.count == 16 else {
      throw permissionDeniedException()
    }
    let objectID = try await openDirectoryForHost(
      locator: locator,
      rights: rights,
      authenticatedIdentity: authenticatedIdentity
    )
    return DirectoryPermissionHandle(
      locator: locator,
      rights: rights,
      quota: quota,
      runtimeID: runtimeID,
      objectID: objectID
    )
  }

  /// Installs an imported/granted directory as an opaque private-slot host object.
  public func installDirectoryPermission(
    _ permission: DirectoryPermissionHandle,
    globalName: String
  ) async throws {
    try validate(permission)
    try await installHostObject(
      globalName: globalName,
      kind: .directory,
      objectID: permission.objectID
    )
  }

  /// Installs a private-slot host object for capability/resource adapters.
  public func installHostObject(
    globalName: String,
    kind: HostObjectKind,
    objectID: UInt64
  ) async throws {
    try await installHostObject(
      globalName: globalName, objectRuntimeID: runtimeID, kind: kind, objectID: objectID)
  }

  #if DEBUG
    func installHostObjectForTesting(
      globalName: String,
      objectRuntimeID: UInt64,
      kind: HostObjectKind,
      objectID: UInt64
    ) async throws {
      try await installHostObject(
        globalName: globalName,
        objectRuntimeID: objectRuntimeID,
        kind: kind,
        objectID: objectID
      )
    }
  #endif

  /// Cancels pending dispatches, rejects Promises once, and releases the context
  /// on its owning executor.
  public func shutdown() async {
    let task = shutdownLock.withLock { () -> Task<Void, Never> in
      if let shutdownTask { return shutdownTask }
      let task = Task { [self] in await performShutdown() }
      shutdownTask = task
      return task
    }
    await task.value
  }

  private func performShutdown() async {
    let tasks = await onExecutor { [self] in
      guard !state.shuttingDown else { return [Task<Void, Never>]() }
      state.shuttingDown = true
      let runtime = state.runtime
      for (requestID, pending) in state.promises {
        pending.task.cancel()
        if let runtime {
          _ = rt_runtime_cancel(runtime.pointer, requestID)
        }
      }
      return state.promises.values.map(\.task)
    }
    for task in tasks {
      await task.value
    }
    await onExecutor { [self] in
      if let runtime = state.runtime {
        _ = rt_runtime_shutdown(runtime.pointer)
      }
    }
    while await onExecutor({ [self] in
      drainCompletionBatch(
        maximumCompletions: maxDrainCompletions,
        maximumBytes: maxDrainBytes
      )
    }) {
      await Task.yield()
    }
    await onExecutor { [self] in
      for pending in state.promises.values {
        reject(
          pending.reject,
          failure: HostDispatchFailure(
            name: "Error", message: "Runtime is shutting down", code: "RuntimeShuttingDown"))
      }
      state.promises.removeAll()
      let evaluations = state.evaluations
      state.evaluations.removeAll()
      for evaluation in evaluations.values {
        evaluation.resume(throwing: shutdownException())
      }
      checkpointMicrotasks()
      state.evaluateFunction = nil
      state.decorateHostObjectFunction = nil
      state.makeErrorFunction = nil
      state.context = nil
      state.hostObjectGlobals.removeAll()
      if let runtime = state.runtime {
        _ = rt_runtime_set_notifier(runtime.pointer, nil, nil)
        state.runtime = nil
        rt_runtime_free(runtime.pointer)
      }
    }
  }

  private func onExecutor<T: Sendable>(
    _ operation: @escaping @Sendable () -> T
  ) async -> T {
    await withCheckedContinuation { continuation in
      executor.enqueue { [executor] in
        executor.preconditionIsCurrent()
        continuation.resume(returning: operation())
      }
    }
  }

  private func onExecutorThrowing<T: Sendable>(
    _ operation: @escaping @Sendable () throws -> T
  ) async throws -> T {
    try await withCheckedThrowingContinuation { continuation in
      executor.enqueue { [executor] in
        executor.preconditionIsCurrent()
        continuation.resume(with: Result { try operation() })
      }
    }
  }

  private func validate(_ permission: DirectoryPermissionHandle) throws {
    guard permission.runtimeID == runtimeID, permission.objectID != 0 else {
      throw JavaScriptException(
        name: "TypeError", message: "Invalid directory permission", stack: nil,
        code: "InvalidResource")
    }
  }

  private func openDirectoryForHost(
    locator: String,
    rights: UInt64,
    authenticatedIdentity: Data?
  ) async throws -> UInt64 {
    guard !locator.isEmpty, locator.utf8.count <= maxFilesystemPathBytes,
      rights > 0, rights & ~maxFilesystemRights == 0
    else {
      throw permissionDeniedException()
    }
    let locatorBytes = Data(locator.utf8)
    return try await onExecutorThrowing { [self] in
      guard !state.shuttingDown, let runtime = state.runtime else { throw shutdownException() }
      var objectID: UInt64 = 0
      let status = locatorBytes.withUnsafeBytes { locatorBuffer in
        if let authenticatedIdentity {
          return authenticatedIdentity.withUnsafeBytes { identityBuffer in
            rt_runtime_fs_open_imported_directory(
              runtime.pointer,
              locatorBuffer.bindMemory(to: UInt8.self).baseAddress,
              locatorBuffer.count,
              rights,
              identityBuffer.bindMemory(to: UInt8.self).baseAddress,
              identityBuffer.count,
              &objectID
            )
          }
        }
        return rt_runtime_fs_open_approved_directory(
          runtime.pointer,
          locatorBuffer.bindMemory(to: UInt8.self).baseAddress,
          locatorBuffer.count,
          rights,
          &objectID
        )
      }
      guard status.status == RT_OK else { throw Self.exceptionForOperationStatus(status) }
      return objectID
    }
  }

  private init(
    runtimeID: UInt64,
    executor: any JavaScriptSerialExecutor,
    dispatcher: @escaping HostOperationDispatcher,
    directoryResolver: DirectoryGrantResolver?
  ) {
    self.runtimeID = runtimeID
    self.executor = executor
    self.dispatcher = dispatcher
    self.directoryResolver = directoryResolver
  }

  private func initialize() async throws {
    try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
      executor.enqueue { [self] in
        executor.preconditionIsCurrent()
        var configuration = rt_config_t(
          abi_version: UInt32(RT_ABI_VERSION),
          flags: 0,
          max_inflight_requests: UInt32(maxInflightRequests),
          max_completion_bytes: UInt32(maxOpBytes)
        )
        var runtimePointer: OpaquePointer?
        guard rt_runtime_new(&configuration, &runtimePointer).status == RT_OK,
          let runtimePointer
        else {
          continuation.resume(throwing: internalException("Unable to create runtime core"))
          return
        }
        let runtime = RuntimeHandle(pointer: runtimePointer)
        state.runtime = runtime
        let userData = Unmanaged.passUnretained(self).toOpaque()
        guard
          rt_runtime_set_notifier(runtime.pointer, completionNotifier, userData).status == RT_OK,
          rt_runtime_start(runtime.pointer).status == RT_OK
        else {
          rt_runtime_free(runtime.pointer)
          state.runtime = nil
          continuation.resume(throwing: internalException("Unable to start runtime core"))
          return
        }
        guard let context = JSContext() else {
          rt_runtime_set_notifier(runtime.pointer, nil, nil)
          rt_runtime_free(runtime.pointer)
          state.runtime = nil
          continuation.resume(throwing: internalException("Unable to create JSC context"))
          return
        }
        state.context = context
        installNativeCallbacks(in: context)
        guard let bridge = context.evaluateScript(Self.bootstrap), context.exception == nil else {
          let exception =
            context.exception.map { structuredException($0, code: "Bootstrap") }
            ?? internalException("Bootstrap failed")
          context.exception = nil
          state.context = nil
          rt_runtime_set_notifier(runtime.pointer, nil, nil)
          rt_runtime_free(runtime.pointer)
          state.runtime = nil
          continuation.resume(throwing: exception)
          return
        }
        state.evaluateFunction = bridge.forProperty("evaluate")
        state.decorateHostObjectFunction = bridge.forProperty("decorateHostObject")
        state.makeErrorFunction = bridge.forProperty("makeError")
        continuation.resume()
      }
    }
  }

  private func installNativeCallbacks(in context: JSContext) {
    typealias SubmitBlock = @convention(block) (NSString, JSValue, JSValue, JSValue) -> Void
    let submit: SubmitBlock = { [weak self] name, input, resolve, reject in
      self?.submit(name: name as String, input: input, resolve: resolve, reject: reject)
    }
    typealias EvaluationBlock = @convention(block) (Double, Bool, NSString) -> Void
    let evaluationDone: EvaluationBlock = { [weak self] identifier, succeeded, json in
      self?.finishEvaluation(identifier: identifier, succeeded: succeeded, json: json as String)
    }
    typealias BrandBlock = @convention(block) (JSValue) -> Bool
    let checkCapability: BrandBlock = { [weak self] value in
      self?.checkHostObject(value, kind: .capability) ?? false
    }
    let checkResource: BrandBlock = { [weak self] value in
      guard let self else { return false }
      return self.checkHostObject(value, kind: .resource)
        || self.checkHostObject(value, kind: .directory)
        || self.checkHostObject(value, kind: .file)
    }
    typealias DirectoryRequestBlock =
      @convention(block) (NSString, Double, JSValue, JSValue) -> Void
    let requestDirectory: DirectoryRequestBlock = { [weak self] locator, rights, resolve, reject in
      self?.submitDirectoryPermission(
        locator: locator as String,
        rights: rights,
        resolve: resolve,
        reject: reject
      )
    }
    typealias FsReadBlock =
      @convention(block) (JSValue, NSString, Double, JSValue, JSValue) -> Void
    let fsRead: FsReadBlock = { [weak self] receiver, path, maxBytes, resolve, reject in
      self?.submitFilesystemRead(
        receiver: receiver,
        path: path as String,
        maxBytes: maxBytes,
        resolve: resolve,
        reject: reject
      )
    }
    typealias FsWriteBlock =
      @convention(block) (JSValue, NSString, JSValue, Double, JSValue, JSValue) -> Void
    let fsWrite: FsWriteBlock = { [weak self] receiver, path, input, flags, resolve, reject in
      self?.submitFilesystemWrite(
        receiver: receiver,
        path: path as String,
        input: input,
        flags: flags,
        resolve: resolve,
        reject: reject
      )
    }
    typealias FsStatBlock =
      @convention(block) (JSValue, NSString, JSValue, JSValue) -> Void
    let fsStat: FsStatBlock = { [weak self] receiver, path, resolve, reject in
      self?.submitFilesystemStat(
        receiver: receiver,
        path: path as String,
        resolve: resolve,
        reject: reject
      )
    }
    typealias FsCloseBlock = @convention(block) (JSValue, JSValue, JSValue) -> Void
    let fsClose: FsCloseBlock = { [weak self] receiver, resolve, reject in
      self?.submitFilesystemClose(receiver: receiver, resolve: resolve, reject: reject)
    }
    typealias FsOpenBlock =
      @convention(block) (JSValue, NSString, Double, Double, JSValue, JSValue) -> Void
    let fsOpen: FsOpenBlock = { [weak self] receiver, path, rights, flags, resolve, reject in
      self?.submitFilesystemOpen(
        receiver: receiver,
        path: path as String,
        rights: rights,
        flags: flags,
        resolve: resolve,
        reject: reject
      )
    }
    typealias FileReadBlock =
      @convention(block) (JSValue, Double, JSValue, JSValue) -> Void
    let fileRead: FileReadBlock = { [weak self] receiver, maxBytes, resolve, reject in
      self?.submitFileRead(
        receiver: receiver, maxBytes: maxBytes, resolve: resolve, reject: reject)
    }
    typealias FileWriteBlock =
      @convention(block) (JSValue, JSValue, JSValue, JSValue) -> Void
    let fileWrite: FileWriteBlock = { [weak self] receiver, input, resolve, reject in
      self?.submitFileWrite(receiver: receiver, input: input, resolve: resolve, reject: reject)
    }
    typealias FileStatBlock = @convention(block) (JSValue, JSValue, JSValue) -> Void
    let fileStat: FileStatBlock = { [weak self] receiver, resolve, reject in
      self?.submitFileStat(receiver: receiver, resolve: resolve, reject: reject)
    }
    context.setObject(submit, forKeyedSubscript: "__zintlNativeSubmit" as NSString)
    context.setObject(evaluationDone, forKeyedSubscript: "__zintlEvaluationDone" as NSString)
    context.setObject(checkCapability, forKeyedSubscript: "__zintlCheckCapability" as NSString)
    context.setObject(checkResource, forKeyedSubscript: "__zintlCheckResource" as NSString)
    context.setObject(
      requestDirectory,
      forKeyedSubscript: "__zintlRequestDirectory" as NSString
    )
    context.setObject(fsRead, forKeyedSubscript: "__zintlFsRead" as NSString)
    context.setObject(fsWrite, forKeyedSubscript: "__zintlFsWrite" as NSString)
    context.setObject(fsStat, forKeyedSubscript: "__zintlFsStat" as NSString)
    context.setObject(fsClose, forKeyedSubscript: "__zintlFsClose" as NSString)
    context.setObject(fsOpen, forKeyedSubscript: "__zintlFsOpen" as NSString)
    context.setObject(fileRead, forKeyedSubscript: "__zintlFileRead" as NSString)
    context.setObject(fileWrite, forKeyedSubscript: "__zintlFileWrite" as NSString)
    context.setObject(fileStat, forKeyedSubscript: "__zintlFileStat" as NSString)
  }

  private func submitDirectoryPermission(
    locator: String,
    rights: Double,
    resolve: JSValue,
    reject: JSValue
  ) {
    executor.preconditionIsCurrent()
    guard !state.shuttingDown, let runtime = state.runtime else {
      self.reject(
        reject,
        failure: HostDispatchFailure(
          name: "Error", message: "Runtime is shutting down", code: "RuntimeShuttingDown"))
      return
    }
    guard locator.utf8.count <= maxFilesystemPathBytes,
      rights.isFinite, rights >= 1, rights <= Double(maxFilesystemRights),
      rights.rounded(.down) == rights,
      UInt64(rights) & ~maxFilesystemRights == 0,
      let resolver = directoryResolver
    else {
      self.reject(
        reject,
        failure: HostDispatchFailure(
          name: "PermissionDenied", message: "Directory permission denied",
          code: "PermissionDenied"))
      return
    }
    let requestID = state.nextRequestID
    guard requestID < UInt64.max else {
      self.reject(reject, failure: internalFailure("Request identity exhausted"))
      return
    }
    state.nextRequestID += 1
    let requestedRights = UInt64(rights)
    let locatorBytes = Data(locator.utf8)
    let submitStatus = locatorBytes.withUnsafeBytes { buffer in
      rt_runtime_submit(
        runtime.pointer,
        requestID,
        directoryRequestOpID,
        buffer.bindMemory(to: UInt8.self).baseAddress,
        buffer.count
      )
    }
    guard submitStatus.status == RT_OK else {
      self.reject(reject, failure: failureForRuntimeStatus(submitStatus.status))
      return
    }
    let task = Task { [runtime] in
      let result: Result<Data, HostDispatchFailure>
      do {
        let request = DirectoryPermissionRequest(
          requestID: requestID,
          locator: locator,
          rights: requestedRights
        )
        let grant = try await resolver(request)
        guard grant.locator == locator,
          grant.rights == requestedRights,
          grant.quota > 0
        else {
          throw HostDispatchFailure(
            name: "PermissionDenied", message: "Invalid directory grant",
            code: "PermissionDenied")
        }
        try Task.checkCancellation()
        var objectID: UInt64 = 0
        let status = locatorBytes.withUnsafeBytes { buffer in
          rt_runtime_fs_open_approved_directory(
            runtime.pointer,
            buffer.bindMemory(to: UInt8.self).baseAddress,
            buffer.count,
            grant.rights,
            &objectID
          )
        }
        guard status.status == RT_OK else {
          throw Self.failureForOperationStatus(status)
        }
        result = .success(Data(Self.bigEndianBytes(objectID)))
      } catch is CancellationError {
        result = .failure(
          HostDispatchFailure(name: "Error", message: "Request cancelled", code: "Cancelled"))
      } catch let failure as HostDispatchFailure {
        result = .failure(failure)
      } catch let error as DirectoryPermissionError {
        switch error {
        case .denied:
          result = .failure(
            HostDispatchFailure(
              name: "PermissionDenied", message: "Directory permission denied",
              code: "PermissionDenied"))
        case .timedOut:
          result = .failure(
            HostDispatchFailure(
              name: "TimeoutError", message: "Directory permission timed out", code: "TimedOut"))
        case .cancelled:
          result = .failure(
            HostDispatchFailure(
              name: "Error", message: "Directory permission cancelled", code: "Cancelled"))
        }
      } catch {
        result = .failure(
          HostDispatchFailure(
            name: "PermissionDenied", message: "Directory permission denied",
            code: "PermissionDenied"))
      }
      Self.completeRuntime(runtime, requestID: requestID, result: result)
    }
    state.promises[requestID] = PendingPromise(
      resolve: resolve,
      reject: reject,
      task: task,
      resultKind: .directory
    )
  }

  private func submitFilesystemRead(
    receiver: JSValue,
    path: String,
    maxBytes: Double,
    resolve: JSValue,
    reject: JSValue
  ) {
    guard maxBytes.isFinite, maxBytes >= 1, maxBytes <= Double(maxOpBytes),
      maxBytes.rounded(.down) == maxBytes
    else {
      self.reject(
        reject,
        failure: HostDispatchFailure(
          name: "QuotaExceeded", message: "Invalid read limit", code: "QuotaExceeded"))
      return
    }
    submitFilesystemOperation(
      receiver: receiver,
      path: path,
      resolve: resolve,
      reject: reject,
      operation: { runtime, requestID, objectID, pathBuffer in
        rt_runtime_fs_read_file(
          runtime.pointer,
          requestID,
          objectID,
          pathBuffer.bindMemory(to: UInt8.self).baseAddress,
          pathBuffer.count,
          UInt32(maxBytes)
        )
      }
    )
  }

  private func submitFilesystemWrite(
    receiver: JSValue,
    path: String,
    input: JSValue,
    flags: Double,
    resolve: JSValue,
    reject: JSValue
  ) {
    guard flags.isFinite, flags >= 0, flags <= 3, flags.rounded(.down) == flags,
      let bytes = decodeBytes(input)
    else {
      self.reject(
        reject,
        failure: HostDispatchFailure(
          name: "TypeError", message: "Invalid write input", code: "InvalidArgument"))
      return
    }
    let data = Data(bytes)
    submitFilesystemOperation(
      receiver: receiver,
      path: path,
      resolve: resolve,
      reject: reject,
      operation: { runtime, requestID, objectID, pathBuffer in
        data.withUnsafeBytes { dataBuffer in
          rt_runtime_fs_write_file(
            runtime.pointer,
            requestID,
            objectID,
            pathBuffer.bindMemory(to: UInt8.self).baseAddress,
            pathBuffer.count,
            dataBuffer.bindMemory(to: UInt8.self).baseAddress,
            dataBuffer.count,
            UInt32(flags) & 1,
            (UInt32(flags) >> 1) & 1
          )
        }
      }
    )
  }

  private func submitFilesystemStat(
    receiver: JSValue,
    path: String,
    resolve: JSValue,
    reject: JSValue
  ) {
    submitFilesystemOperation(
      receiver: receiver,
      path: path,
      resolve: resolve,
      reject: reject,
      operation: { runtime, requestID, objectID, pathBuffer in
        rt_runtime_fs_metadata(
          runtime.pointer,
          requestID,
          objectID,
          pathBuffer.bindMemory(to: UInt8.self).baseAddress,
          pathBuffer.count
        )
      }
    )
  }

  private func submitFilesystemOpen(
    receiver: JSValue,
    path: String,
    rights: Double,
    flags: Double,
    resolve: JSValue,
    reject: JSValue
  ) {
    executor.preconditionIsCurrent()
    guard !state.shuttingDown, let runtime = state.runtime,
      let directoryObjectID = readHostObject(receiver, kind: .directory),
      !path.isEmpty, path.utf8.count <= maxFilesystemPathBytes,
      rights.isFinite, rights >= 1, rights <= Double(maxFileRights),
      rights.rounded(.down) == rights, UInt64(rights) & ~maxFileRights == 0,
      flags.isFinite, flags >= 0, flags <= 3, flags.rounded(.down) == flags
    else {
      self.reject(
        reject,
        failure: HostDispatchFailure(
          name: "TypeError", message: "Invalid file open request", code: "InvalidArgument"))
      return
    }
    let requestID = state.nextRequestID
    guard requestID < UInt64.max else {
      self.reject(reject, failure: internalFailure("Request identity exhausted"))
      return
    }
    state.nextRequestID += 1
    let pathBytes = Data(path.utf8)
    let submitStatus = pathBytes.withUnsafeBytes { buffer in
      rt_runtime_submit(
        runtime.pointer,
        requestID,
        filesystemOpenOpID,
        buffer.bindMemory(to: UInt8.self).baseAddress,
        buffer.count
      )
    }
    guard submitStatus.status == RT_OK else {
      self.reject(reject, failure: failureForRuntimeStatus(submitStatus.status))
      return
    }
    let requestedRights = UInt64(rights)
    let openFlags = UInt32(flags)
    let task = Task.detached { [runtime] in
      var objectID: UInt64 = 0
      let status = pathBytes.withUnsafeBytes { buffer in
        rt_runtime_fs_open_relative(
          runtime.pointer,
          directoryObjectID,
          buffer.bindMemory(to: UInt8.self).baseAddress,
          buffer.count,
          requestedRights,
          openFlags & 1,
          (openFlags >> 1) & 1,
          &objectID
        )
      }
      let result: Result<Data, HostDispatchFailure>
      if Task.isCancelled {
        result = .failure(
          HostDispatchFailure(name: "Error", message: "Request cancelled", code: "Cancelled"))
      } else if status.status == RT_OK {
        result = .success(Data(Self.bigEndianBytes(objectID)))
      } else {
        result = .failure(Self.failureForOperationStatus(status))
      }
      Self.completeRuntime(runtime, requestID: requestID, result: result)
    }
    state.promises[requestID] = PendingPromise(
      resolve: resolve,
      reject: reject,
      task: task,
      resultKind: .file
    )
  }

  private func submitFileRead(
    receiver: JSValue,
    maxBytes: Double,
    resolve: JSValue,
    reject: JSValue
  ) {
    guard maxBytes.isFinite, maxBytes >= 1, maxBytes <= Double(maxOpBytes),
      maxBytes.rounded(.down) == maxBytes
    else {
      self.reject(
        reject,
        failure: HostDispatchFailure(
          name: "QuotaExceeded", message: "Invalid read limit", code: "QuotaExceeded"))
      return
    }
    submitFileOperation(receiver: receiver, resolve: resolve, reject: reject) {
      runtime, requestID, objectID in
      rt_runtime_fs_read(runtime.pointer, requestID, objectID, UInt32(maxBytes))
    }
  }

  private func submitFileWrite(
    receiver: JSValue,
    input: JSValue,
    resolve: JSValue,
    reject: JSValue
  ) {
    guard let bytes = decodeBytes(input) else {
      self.reject(
        reject,
        failure: HostDispatchFailure(
          name: "TypeError", message: "Invalid file write input", code: "InvalidArgument"))
      return
    }
    let data = Data(bytes)
    submitFileOperation(receiver: receiver, resolve: resolve, reject: reject) {
      runtime, requestID, objectID in
      data.withUnsafeBytes { buffer in
        rt_runtime_fs_write(
          runtime.pointer,
          requestID,
          objectID,
          buffer.bindMemory(to: UInt8.self).baseAddress,
          buffer.count
        )
      }
    }
  }

  private func submitFileStat(receiver: JSValue, resolve: JSValue, reject: JSValue) {
    submitFileOperation(receiver: receiver, resolve: resolve, reject: reject) {
      runtime, requestID, objectID in
      rt_runtime_fs_stat(runtime.pointer, requestID, objectID)
    }
  }

  private func submitFileOperation(
    receiver: JSValue,
    resolve: JSValue,
    reject: JSValue,
    operation: (RuntimeHandle, UInt64, UInt64) -> rt_status_t
  ) {
    executor.preconditionIsCurrent()
    guard !state.shuttingDown, let runtime = state.runtime,
      let objectID = readHostObject(receiver, kind: .file)
    else {
      self.reject(
        reject,
        failure: HostDispatchFailure(
          name: "TypeError", message: "Invalid file receiver", code: "InvalidResource"))
      return
    }
    let requestID = state.nextRequestID
    guard requestID < UInt64.max else {
      self.reject(reject, failure: internalFailure("Request identity exhausted"))
      return
    }
    state.nextRequestID += 1
    let status = operation(runtime, requestID, objectID)
    guard status.status == RT_OK else {
      self.reject(reject, failure: failureForRuntimeStatus(status.status))
      return
    }
    state.promises[requestID] = PendingPromise(
      resolve: resolve,
      reject: reject,
      task: Task {},
      resultKind: .bytes
    )
  }

  private func submitFilesystemClose(
    receiver: JSValue,
    resolve: JSValue,
    reject: JSValue
  ) {
    executor.preconditionIsCurrent()
    guard !state.shuttingDown, let runtime = state.runtime,
      let objectID = readHostObject(receiver, kind: .directory)
        ?? readHostObject(receiver, kind: .file)
    else {
      self.reject(
        reject,
        failure: HostDispatchFailure(
          name: "TypeError", message: "Invalid filesystem receiver", code: "InvalidResource"))
      return
    }
    let requestID = state.nextRequestID
    guard requestID < UInt64.max else {
      self.reject(reject, failure: internalFailure("Request identity exhausted"))
      return
    }
    state.nextRequestID += 1
    let status = rt_runtime_fs_close(runtime.pointer, requestID, objectID)
    guard status.status == RT_OK else {
      self.reject(reject, failure: failureForRuntimeStatus(status.status))
      return
    }
    state.promises[requestID] = PendingPromise(
      resolve: resolve,
      reject: reject,
      task: Task {},
      resultKind: .bytes
    )
  }

  private func submitFilesystemOperation(
    receiver: JSValue,
    path: String,
    resolve: JSValue,
    reject: JSValue,
    operation: (RuntimeHandle, UInt64, UInt64, UnsafeRawBufferPointer) -> rt_status_t
  ) {
    executor.preconditionIsCurrent()
    guard !state.shuttingDown, let runtime = state.runtime,
      let objectID = readHostObject(receiver, kind: .directory),
      !path.isEmpty, path.utf8.count <= maxFilesystemPathBytes
    else {
      self.reject(
        reject,
        failure: HostDispatchFailure(
          name: "TypeError", message: "Invalid filesystem request", code: "InvalidArgument"))
      return
    }
    let requestID = state.nextRequestID
    guard requestID < UInt64.max else {
      self.reject(reject, failure: internalFailure("Request identity exhausted"))
      return
    }
    state.nextRequestID += 1
    let pathBytes = Data(path.utf8)
    let status = pathBytes.withUnsafeBytes { buffer in
      operation(runtime, requestID, objectID, buffer)
    }
    guard status.status == RT_OK else {
      self.reject(reject, failure: failureForRuntimeStatus(status.status))
      return
    }
    state.promises[requestID] = PendingPromise(
      resolve: resolve,
      reject: reject,
      task: Task {},
      resultKind: .bytes
    )
  }

  private func submit(name: String, input: JSValue, resolve: JSValue, reject: JSValue) {
    executor.preconditionIsCurrent()
    guard !state.shuttingDown else {
      self.reject(
        reject,
        failure: HostDispatchFailure(
          name: "Error", message: "Runtime is shutting down", code: "RuntimeShuttingDown"))
      return
    }
    guard !name.hasPrefix("zintl.builtin.") || name == Self.timerSleepOperation else {
      self.reject(
        reject,
        failure: HostDispatchFailure(
          name: "NotSupported", message: "Unknown builtin operation", code: "NotSupported"))
      return
    }
    guard let bytes = decodeBytes(input) else {
      self.reject(
        reject,
        failure: HostDispatchFailure(
          name: "TypeError", message: "Expected byte input", code: "InvalidArgument"))
      return
    }
    guard let runtime = state.runtime else {
      self.reject(
        reject,
        failure: HostDispatchFailure(
          name: "Error", message: "Runtime core is unavailable", code: "RuntimeShuttingDown"))
      return
    }
    let requestID = state.nextRequestID
    guard requestID < UInt64.max else {
      self.reject(
        reject,
        failure: HostDispatchFailure(
          name: "Error", message: "Request identity exhausted", code: "Internal"))
      return
    }
    state.nextRequestID += 1
    let inputData = Data(bytes)
    if name == Self.timerSleepOperation {
      submitTimer(
        requestID: requestID,
        input: inputData,
        resolve: resolve,
        reject: reject,
        runtime: runtime
      )
      return
    }
    let submitStatus = inputData.withUnsafeBytes { buffer in
      rt_runtime_submit(
        runtime.pointer,
        requestID,
        hostDispatchOpID,
        buffer.bindMemory(to: UInt8.self).baseAddress,
        buffer.count
      )
    }
    guard submitStatus.status == RT_OK else {
      self.reject(reject, failure: failureForRuntimeStatus(submitStatus.status))
      return
    }
    let task = Task { [dispatcher, runtime] in
      let result = await dispatcher(name, inputData, requestID)
      Self.completeRuntime(runtime, requestID: requestID, result: result)
    }
    state.promises[requestID] = PendingPromise(
      resolve: resolve,
      reject: reject,
      task: task,
      resultKind: .bytes
    )
  }

  private func submitTimer(
    requestID: UInt64,
    input: Data,
    resolve: JSValue,
    reject: JSValue,
    runtime: RuntimeHandle
  ) {
    executor.preconditionIsCurrent()
    guard input.count == MemoryLayout<UInt32>.size,
      let milliseconds = readInteger(UInt32.self, from: input, at: 0),
      milliseconds <= maxTimerMilliseconds
    else {
      self.reject(
        reject,
        failure: HostDispatchFailure(
          name: "RangeError", message: "Invalid timer delay", code: "InvalidArgument"))
      return
    }
    let (delayNanoseconds, delayOverflow) = UInt64(milliseconds).multipliedReportingOverflow(
      by: 1_000_000)
    let now = DispatchTime.now().uptimeNanoseconds
    let (deadline, deadlineOverflow) = now.addingReportingOverflow(delayNanoseconds)
    guard !delayOverflow, !deadlineOverflow else {
      self.reject(
        reject,
        failure: HostDispatchFailure(
          name: "RangeError", message: "Timer deadline overflow", code: "InvalidArgument"))
      return
    }
    let submitStatus = rt_runtime_submit_timer(
      runtime.pointer,
      requestID,
      timerSleepOpID,
      deadline
    )
    guard submitStatus.status == RT_OK else {
      self.reject(reject, failure: failureForRuntimeStatus(submitStatus.status))
      return
    }
    let drainLimit = UInt32(maxDrainCompletions)
    let task = Task.detached { [runtime] in
      do {
        try await Task.sleep(for: .milliseconds(Int64(milliseconds)))
      } catch {
        return
      }
      if Task.isCancelled { return }
      var fired: UInt32 = 0
      _ = rt_runtime_fire_due_timers(
        runtime.pointer,
        DispatchTime.now().uptimeNanoseconds,
        drainLimit,
        &fired
      )
    }
    state.promises[requestID] = PendingPromise(
      resolve: resolve,
      reject: reject,
      task: task,
      resultKind: .bytes
    )
  }

  private func settle(_ completion: DecodedCompletion) {
    executor.preconditionIsCurrent()
    guard let pending = state.promises.removeValue(forKey: completion.requestID) else {
      return
    }
    if completion.status == 0 {
      switch pending.resultKind {
      case .bytes:
        _ = pending.resolve.call(withArguments: [Array(completion.payload)])
      case .directory:
        guard completion.payload.count == MemoryLayout<UInt64>.size,
          let objectID = readInteger(UInt64.self, from: completion.payload, at: 0),
          let object = makeHostObject(
            objectRuntimeID: runtimeID,
            kind: .directory,
            objectID: objectID
          )
        else {
          reject(
            pending.reject,
            failure: HostDispatchFailure(
              name: "Error", message: "Invalid directory result", code: "Protocol"))
          return
        }
        _ = pending.resolve.call(withArguments: [object])
      case .file:
        guard completion.payload.count == MemoryLayout<UInt64>.size,
          let objectID = readInteger(UInt64.self, from: completion.payload, at: 0),
          let object = makeHostObject(
            objectRuntimeID: runtimeID,
            kind: .file,
            objectID: objectID
          )
        else {
          reject(
            pending.reject,
            failure: HostDispatchFailure(
              name: "Error", message: "Invalid file result", code: "Protocol"))
          return
        }
        _ = pending.resolve.call(withArguments: [object])
      }
    } else {
      let failure =
        (try? JSONDecoder().decode(HostDispatchFailure.self, from: completion.payload))
        ?? failureForCompletionStatus(completion.status)
      reject(pending.reject, failure: failure)
    }
  }

  fileprivate func scheduleCompletionDrain() {
    drainLock.lock()
    if drainScheduled {
      drainLock.unlock()
      return
    }
    drainScheduled = true
    drainLock.unlock()
    executor.enqueue { [self] in
      executor.preconditionIsCurrent()
      drainLock.lock()
      drainScheduled = false
      drainLock.unlock()
      let hasMore = drainCompletionBatch(
        maximumCompletions: maxDrainCompletions,
        maximumBytes: maxDrainBytes
      )
      if hasMore {
        scheduleCompletionDrain()
      }
    }
  }

  private func drainCompletionBatch(
    maximumCompletions: Int,
    maximumBytes: Int
  ) -> Bool {
    executor.preconditionIsCurrent()
    guard let runtime = state.runtime else { return false }
    var pumped: UInt32 = 0
    let pumpStatus = rt_runtime_pump_filesystem(
      runtime.pointer,
      UInt32(maximumCompletions),
      &pumped
    )
    let filesystemHasMore = pumpStatus.status == RT_OK && pumped == UInt32(maximumCompletions)
    var drained = 0
    var drainedBytes = 0
    while drained < maximumCompletions {
      var required = 0
      let sizeStatus = rt_runtime_next_completion(runtime.pointer, nil, 0, &required)
      if sizeStatus.status == RT_EMPTY {
        break
      }
      guard sizeStatus.status == RT_BUFFER_TOO_SMALL, required > 0,
        required <= maxCompletionEnvelopeBytes
      else {
        break
      }
      if drained > 0, drainedBytes + required > maximumBytes {
        return true
      }
      var encoded = Data(count: required)
      let readStatus = encoded.withUnsafeMutableBytes { buffer in
        rt_runtime_next_completion(
          runtime.pointer,
          buffer.bindMemory(to: UInt8.self).baseAddress,
          buffer.count,
          &required
        )
      }
      guard readStatus.status == RT_OK, let completion = decodeCompletion(encoded) else {
        break
      }
      drained += 1
      drainedBytes += encoded.count
      settle(completion)
    }
    if drained > 0 {
      checkpointMicrotasks()
    }
    var required = 0
    return filesystemHasMore
      || rt_runtime_next_completion(runtime.pointer, nil, 0, &required).status
        == RT_BUFFER_TOO_SMALL
  }

  private func decodeCompletion(_ encoded: Data) -> DecodedCompletion? {
    guard encoded.count >= 28, encoded.prefix(4) == Data([0x5A, 0x52, 0x54, 0x31]),
      readInteger(UInt16.self, from: encoded, at: 4) == 1,
      readInteger(UInt16.self, from: encoded, at: 6) == 2,
      let requestID = readInteger(UInt64.self, from: encoded, at: 8), requestID != 0,
      let status = readInteger(UInt32.self, from: encoded, at: 16), status <= 14,
      readInteger(UInt32.self, from: encoded, at: 20) == 0,
      let payloadLength = readInteger(UInt32.self, from: encoded, at: 24),
      Int(payloadLength) == encoded.count - 28
    else {
      return nil
    }
    return DecodedCompletion(
      requestID: requestID,
      status: status,
      payload: encoded.subdata(in: 28..<encoded.count)
    )
  }

  private func readInteger<T: FixedWidthInteger>(
    _ type: T.Type,
    from data: Data,
    at offset: Int
  ) -> T? {
    guard offset >= 0, offset <= data.count, MemoryLayout<T>.size <= data.count - offset else {
      return nil
    }
    return data.withUnsafeBytes { buffer in
      T(bigEndian: buffer.loadUnaligned(fromByteOffset: offset, as: type))
    }
  }

  private static func completeRuntime(
    _ runtime: RuntimeHandle,
    requestID: UInt64,
    result: Result<Data, HostDispatchFailure>
  ) {
    let status: UInt32
    let payload: Data
    switch result {
    case .success(let output) where output.count <= maximumFFIPayloadBytes:
      status = 0
      payload = output
    case .success:
      let failure = HostDispatchFailure(
        name: "QuotaExceeded", message: "Operation output exceeded runtime limit",
        code: "QuotaExceeded")
      status = 10
      payload = (try? JSONEncoder().encode(failure)) ?? Data()
    case .failure(let failure):
      status = completionStatus(for: failure.code)
      payload = (try? JSONEncoder().encode(failure)) ?? Data()
    }
    payload.withUnsafeBytes { buffer in
      _ = rt_runtime_complete_host_op(
        runtime.pointer,
        requestID,
        status,
        buffer.bindMemory(to: UInt8.self).baseAddress,
        buffer.count
      )
    }
  }

  private static func completionStatus(for code: String) -> UInt32 {
    switch code {
    case "PermissionDenied": 1
    case "InvalidArgument": 2
    case "InvalidCapability": 3
    case "InvalidResource": 4
    case "ResourceClosed": 5
    case "ResourceBusy": 6
    case "NotSupported": 7
    case "Cancelled": 8
    case "TimedOut": 9
    case "QuotaExceeded": 10
    case "Io": 11
    case "Protocol": 12
    case "RuntimeShuttingDown": 13
    default: 14
    }
  }

  private static func failureForOperationStatus(_ status: rt_status_t) -> HostDispatchFailure {
    if status.status == RT_INVALID_ARGUMENT {
      return HostDispatchFailure(
        name: "TypeError", message: "Invalid operation request", code: "InvalidArgument")
    }
    if status.status == RT_RUNTIME_SHUTTING_DOWN {
      return HostDispatchFailure(
        name: "Error", message: "Runtime is shutting down", code: "RuntimeShuttingDown")
    }
    guard status.status == RT_OPERATION_FAILED else {
      return HostDispatchFailure(
        name: "Error", message: "Operation failed", code: "Internal")
    }
    let code: String
    switch status.detail {
    case 1: code = "PermissionDenied"
    case 2: code = "InvalidArgument"
    case 3: code = "InvalidCapability"
    case 4: code = "InvalidResource"
    case 5: code = "ResourceClosed"
    case 7: code = "NotSupported"
    case 8: code = "Cancelled"
    case 9: code = "TimedOut"
    case 10: code = "QuotaExceeded"
    case 11: code = "Io"
    case 12: code = "Protocol"
    case 13: code = "RuntimeShuttingDown"
    default: code = "Internal"
    }
    return HostDispatchFailure(name: "Error", message: "Operation failed", code: code)
  }

  private static func exceptionForOperationStatus(_ status: rt_status_t) -> JavaScriptException {
    let failure = failureForOperationStatus(status)
    return JavaScriptException(
      name: failure.name,
      message: failure.message,
      stack: nil,
      code: failure.code
    )
  }

  private static func bigEndianBytes(_ value: UInt64) -> [UInt8] {
    let value = value.bigEndian
    return withUnsafeBytes(of: value) { Array($0) }
  }

  private func failureForRuntimeStatus(_ status: UInt32) -> HostDispatchFailure {
    switch status {
    case UInt32(RT_INVALID_ARGUMENT):
      HostDispatchFailure(
        name: "TypeError", message: "Invalid runtime request", code: "InvalidArgument")
    case UInt32(RT_UNKNOWN_REQUEST):
      HostDispatchFailure(
        name: "InvalidResource", message: "Invalid runtime resource", code: "InvalidResource")
    case UInt32(RT_QUOTA_EXCEEDED):
      HostDispatchFailure(
        name: "QuotaExceeded", message: "Runtime request limit exceeded", code: "QuotaExceeded")
    case UInt32(RT_RUNTIME_SHUTTING_DOWN):
      HostDispatchFailure(
        name: "Error", message: "Runtime is shutting down", code: "RuntimeShuttingDown")
    default:
      HostDispatchFailure(name: "Error", message: "Runtime request failed", code: "Internal")
    }
  }

  private func failureForCompletionStatus(_ status: UInt32) -> HostDispatchFailure {
    let code: String
    switch status {
    case 1: code = "PermissionDenied"
    case 2: code = "InvalidArgument"
    case 3: code = "InvalidCapability"
    case 4: code = "InvalidResource"
    case 5: code = "ResourceClosed"
    case 6: code = "ResourceBusy"
    case 7: code = "NotSupported"
    case 8: code = "Cancelled"
    case 9: code = "TimedOut"
    case 10: code = "QuotaExceeded"
    case 11: code = "Io"
    case 12: code = "Protocol"
    case 13: code = "RuntimeShuttingDown"
    default: code = "Internal"
    }
    return HostDispatchFailure(name: "Error", message: "Host operation failed", code: code)
  }

  private func reject(_ function: JSValue, failure: HostDispatchFailure) {
    let error = state.makeErrorFunction?.call(withArguments: [
      failure.name, failure.message, failure.code,
    ])
    _ = function.call(withArguments: [error as Any])
  }

  private func finishEvaluation(identifier: Double, succeeded: Bool, json: String) {
    executor.preconditionIsCurrent()
    guard identifier.isFinite, identifier >= 1, identifier.rounded(.down) == identifier else {
      return
    }
    let evaluationID = UInt64(identifier)
    guard let continuation = state.evaluations.removeValue(forKey: evaluationID) else {
      return
    }
    if succeeded {
      continuation.resume(returning: JavaScriptEvaluationResult(json: json))
    } else {
      continuation.resume(
        throwing: JavaScriptException(
          name: "Error", message: json, stack: nil, code: "JavaScript"))
    }
  }

  private func decodeBytes(_ value: JSValue) -> [UInt8]? {
    guard value.isArray, let values = value.toArray() as? [NSNumber], values.count <= maxOpBytes
    else {
      return nil
    }
    var output = [UInt8]()
    output.reserveCapacity(values.count)
    for number in values {
      let integer = number.intValue
      guard integer >= 0, integer <= 255, number.doubleValue == Double(integer) else {
        return nil
      }
      output.append(UInt8(integer))
    }
    return output
  }

  private func checkHostObject(_ value: JSValue, kind: HostObjectKind) -> Bool {
    executor.preconditionIsCurrent()
    guard let context = state.context else { return false }
    var objectID: UInt64 = 0
    return rtjsc_host_object_read(
      context.jsGlobalContextRef,
      value.jsValueRef,
      runtimeID,
      kind.rawValue,
      &objectID
    ) == 0
  }

  private func readHostObject(_ value: JSValue, kind: HostObjectKind) -> UInt64? {
    executor.preconditionIsCurrent()
    guard let context = state.context else { return nil }
    var objectID: UInt64 = 0
    guard
      rtjsc_host_object_read(
        context.jsGlobalContextRef,
        value.jsValueRef,
        runtimeID,
        kind.rawValue,
        &objectID
      ) == 0,
      objectID != 0
    else {
      return nil
    }
    return objectID
  }

  private func installHostObject(
    globalName: String,
    objectRuntimeID: UInt64,
    kind: HostObjectKind,
    objectID: UInt64
  ) async throws {
    try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
      executor.enqueue { [self] in
        executor.preconditionIsCurrent()
        guard !state.shuttingDown,
          !globalName.isEmpty, globalName.utf8.count <= maxHostObjectNameBytes,
          objectRuntimeID != 0, objectID != 0,
          state.hostObjectGlobals.contains(globalName)
            || state.hostObjectGlobals.count < maxHostObjects,
          let context = state.context,
          let decorated = makeHostObject(
            objectRuntimeID: objectRuntimeID,
            kind: kind,
            objectID: objectID
          )
        else {
          continuation.resume(throwing: internalException("Unable to create host object"))
          return
        }
        context.setObject(decorated, forKeyedSubscript: globalName as NSString)
        state.hostObjectGlobals.insert(globalName)
        continuation.resume()
      }
    }
  }

  private func makeHostObject(
    objectRuntimeID: UInt64,
    kind: HostObjectKind,
    objectID: UInt64
  ) -> JSValue? {
    executor.preconditionIsCurrent()
    guard let context = state.context,
      let decorator = state.decorateHostObjectFunction,
      let object = rtjsc_host_object_make(
        context.jsGlobalContextRef, objectRuntimeID, objectID, kind.rawValue),
      let value = JSValue(jsValueRef: object, in: context)
    else {
      return nil
    }
    return decorator.call(withArguments: [value, kind.rawValue])
  }

  private func checkpointMicrotasks() {
    executor.preconditionIsCurrent()
    _ = state.context?.evaluateScript("void 0")
    state.context?.exception = nil
  }

  private func structuredException(_ value: JSValue, code: String) -> JavaScriptException {
    JavaScriptException(
      name: value.forProperty("name")?.toString() ?? "Error",
      message: value.forProperty("message")?.toString() ?? "JavaScript evaluation failed",
      stack: value.forProperty("stack")?.toString(),
      code: code
    )
  }

  private func shutdownException() -> JavaScriptException {
    JavaScriptException(
      name: "Error", message: "Runtime is shutting down", stack: nil,
      code: "RuntimeShuttingDown")
  }

  private func internalException(_ message: String) -> JavaScriptException {
    JavaScriptException(name: "Error", message: message, stack: nil, code: "Internal")
  }

  private func permissionDeniedException() -> JavaScriptException {
    JavaScriptException(
      name: "PermissionDenied",
      message: "Directory permission denied",
      stack: nil,
      code: "PermissionDenied"
    )
  }

  private func internalFailure(_ message: String) -> HostDispatchFailure {
    HostDispatchFailure(name: "Error", message: message, code: "Internal")
  }

  private let runtimeID: UInt64
  private let executor: any JavaScriptSerialExecutor
  private let dispatcher: HostOperationDispatcher
  private let directoryResolver: DirectoryGrantResolver?
  private let state = AdapterState()
  private let drainLock = NSLock()
  private var drainScheduled = false
  private let shutdownLock = NSLock()
  private var shutdownTask: Task<Void, Never>?
  private let maxInflightRequests = 1_024
  private let maxInflightEvaluations = 1_024
  private let maxHostObjects = 1_024
  private let maxHostObjectNameBytes = 192
  private let maxFilesystemPathBytes = 4_096
  private let maxFilesystemRights: UInt64 = 0x3F
  private let maxFileRights: UInt64 = 0x0B
  private let maxScriptBytes = 4 * 1024 * 1024
  private let maxOpBytes = 4 * 1024 * 1024
  private let maxCompletionEnvelopeBytes = 4 * 1024 * 1024 + 28
  private let maxDrainCompletions = 64
  private let maxDrainBytes = 1024 * 1024
  private let maxTimerMilliseconds: UInt32 = 86_400_000
  private let hostDispatchOpID: UInt32 = 1
  private let timerSleepOpID: UInt32 = 2
  private let directoryRequestOpID: UInt32 = 16
  private let filesystemOpenOpID: UInt32 = 25
  private static let maximumFFIPayloadBytes = 4 * 1024 * 1024
  private static let timerSleepOperation = "zintl.builtin.timer.sleep"

  private static let bootstrap = #"""
    (() => {
      "use strict";
      const submit = globalThis.__zintlNativeSubmit;
      const evaluationDone = globalThis.__zintlEvaluationDone;
      const checkCapability = globalThis.__zintlCheckCapability;
      const checkResource = globalThis.__zintlCheckResource;
      const requestDirectoryNative = globalThis.__zintlRequestDirectory;
      const fsRead = globalThis.__zintlFsRead;
      const fsWrite = globalThis.__zintlFsWrite;
      const fsStat = globalThis.__zintlFsStat;
      const fsClose = globalThis.__zintlFsClose;
      const fsOpen = globalThis.__zintlFsOpen;
      const fileRead = globalThis.__zintlFileRead;
      const fileWrite = globalThis.__zintlFileWrite;
      const fileStat = globalThis.__zintlFileStat;
      delete globalThis.__zintlNativeSubmit;
      delete globalThis.__zintlEvaluationDone;
      delete globalThis.__zintlCheckCapability;
      delete globalThis.__zintlCheckResource;
      delete globalThis.__zintlRequestDirectory;
      delete globalThis.__zintlFsRead;
      delete globalThis.__zintlFsWrite;
      delete globalThis.__zintlFsStat;
      delete globalThis.__zintlFsClose;
      delete globalThis.__zintlFsOpen;
      delete globalThis.__zintlFileRead;
      delete globalThis.__zintlFileWrite;
      delete globalThis.__zintlFileStat;

      const makeError = (name, message, code) => {
        const error = new Error(String(message));
        Object.defineProperty(error, "name", { value: String(name) });
        Object.defineProperty(error, "code", { value: String(code) });
        return error;
      };
      const invoke = (name, input) => {
        if (typeof name !== "string" || !(input instanceof Uint8Array)) {
          return Promise.reject(makeError("TypeError", "Invalid operation input", "InvalidArgument"));
        }
        return new Promise((resolve, reject) => {
          submit(name, Array.from(input), resolve, reject);
        }).then((bytes) => new Uint8Array(bytes));
      };
      const sleep = (milliseconds) => {
        if (!Number.isSafeInteger(milliseconds) || milliseconds < 0 || milliseconds > 86400000) {
          return Promise.reject(makeError("RangeError", "Invalid timer delay", "InvalidArgument"));
        }
        const input = new Uint8Array(4);
        input[0] = (milliseconds >>> 24) & 255;
        input[1] = (milliseconds >>> 16) & 255;
        input[2] = (milliseconds >>> 8) & 255;
        input[3] = milliseconds & 255;
        return invoke("zintl.builtin.timer.sleep", input).then(() => undefined);
      };
      const rightNames = Object.freeze({
        read: 1, write: 2, create: 4, metadata: 8, enumerate: 16, truncate: 32
      });
      const requestDirectory = (locator, options) => {
        if (typeof locator !== "string" || locator.length === 0 || locator.length > 4096 ||
            options === null || typeof options !== "object") {
          return Promise.reject(makeError("TypeError", "Invalid directory request", "InvalidArgument"));
        }
        let rights = 0;
        for (const key of Object.keys(options)) {
          if (!(key in rightNames) || typeof options[key] !== "boolean") {
            return Promise.reject(makeError("TypeError", "Invalid directory rights", "InvalidArgument"));
          }
          if (options[key]) { rights |= rightNames[key]; }
        }
        if (rights === 0) {
          return Promise.reject(makeError("TypeError", "Empty directory rights", "InvalidArgument"));
        }
        return new Promise((resolve, reject) => {
          requestDirectoryNative(locator, rights, resolve, reject);
        });
      };
      const api = Object.freeze({ invoke, sleep, requestDirectory });
      Object.defineProperty(globalThis, "Zintl", {
        value: api, writable: false, enumerable: true, configurable: false
      });
      const decorateHostObject = (object, kind) => {
        const check = kind === 1 ? checkCapability :
          (kind === 2 || kind === 3 || kind === 4) ? checkResource : null;
        const invalidCode = kind === 1 ? "InvalidCapability" : "InvalidResource";
        if (check === null) { throw makeError("TypeError", "Invalid host kind", "Internal"); }
        Object.defineProperty(object, "assertBrand", {
          value: function () {
            if (!check(this)) {
              throw makeError("TypeError", "Invalid host object receiver", invalidCode);
            }
            return true;
          },
          writable: false, enumerable: false, configurable: false
        });
        if (kind === 3) {
          Object.defineProperties(object, {
            openRelative: {
              value: function (path, options) {
                if (!check(this) || typeof path !== "string" || path.length === 0 ||
                    options === null || typeof options !== "object") {
                  return Promise.reject(makeError("TypeError", "Invalid file open", "InvalidResource"));
                }
                let rights = 0;
                for (const key of Object.keys(options)) {
                  if (key === "create" || key === "truncate") {
                    if (typeof options[key] !== "boolean") {
                      return Promise.reject(makeError("TypeError", "Invalid file flags", "InvalidArgument"));
                    }
                  } else if ((key === "read" || key === "write" || key === "metadata") &&
                             typeof options[key] === "boolean") {
                    if (options[key]) { rights |= rightNames[key]; }
                  } else {
                    return Promise.reject(makeError("TypeError", "Invalid file rights", "InvalidArgument"));
                  }
                }
                if (rights === 0) {
                  return Promise.reject(makeError("TypeError", "Empty file rights", "InvalidArgument"));
                }
                const flags = (options.create === true ? 1 : 0) |
                  (options.truncate === true ? 2 : 0);
                return new Promise((resolve, reject) =>
                  fsOpen(this, path, rights, flags, resolve, reject));
              }, writable: false, enumerable: false, configurable: false
            },
            readRelative: {
              value: function (path, options = {}) {
                if (!check(this) || typeof path !== "string" || options === null ||
                    typeof options !== "object") {
                  return Promise.reject(makeError("TypeError", "Invalid directory read", "InvalidResource"));
                }
                const maxBytes = options.maxBytes ?? 65536;
                return new Promise((resolve, reject) => fsRead(this, path, maxBytes, resolve, reject))
                  .then((bytes) => new Uint8Array(bytes));
              }, writable: false, enumerable: false, configurable: false
            },
            writeRelative: {
              value: function (path, bytes, options = {}) {
                if (!check(this) || typeof path !== "string" || !(bytes instanceof Uint8Array) ||
                    options === null || typeof options !== "object") {
                  return Promise.reject(makeError("TypeError", "Invalid directory write", "InvalidResource"));
                }
                const flags = (options.create === true ? 1 : 0) | (options.truncate === true ? 2 : 0);
                return new Promise((resolve, reject) =>
                  fsWrite(this, path, Array.from(bytes), flags, resolve, reject)
                ).then(() => undefined);
              }, writable: false, enumerable: false, configurable: false
            },
            statRelative: {
              value: function (path) {
                if (!check(this) || typeof path !== "string") {
                  return Promise.reject(makeError("TypeError", "Invalid directory stat", "InvalidResource"));
                }
                return new Promise((resolve, reject) => fsStat(this, path, resolve, reject))
                  .then((bytes) => {
                    if (!Array.isArray(bytes) || bytes.length !== 9) {
                      throw makeError("Error", "Invalid metadata result", "Protocol");
                    }
                    let size = 0;
                    for (let index = 1; index < 9; index++) { size = size * 256 + bytes[index]; }
                    return Object.freeze({ kind: bytes[0] === 1 ? "file" : "directory", size });
                  });
              }, writable: false, enumerable: false, configurable: false
            },
            close: {
              value: function () {
                if (!check(this)) {
                  return Promise.reject(makeError("TypeError", "Invalid directory receiver", "InvalidResource"));
                }
                return new Promise((resolve, reject) => fsClose(this, resolve, reject))
                  .then(() => undefined);
              }, writable: false, enumerable: false, configurable: false
            }
          });
        }
        if (kind === 4) {
          const decodeMetadata = (bytes) => {
            if (!Array.isArray(bytes) || bytes.length !== 9) {
              throw makeError("Error", "Invalid metadata result", "Protocol");
            }
            let size = 0;
            for (let index = 1; index < 9; index++) { size = size * 256 + bytes[index]; }
            return Object.freeze({ kind: bytes[0] === 1 ? "file" : "directory", size });
          };
          Object.defineProperties(object, {
            read: {
              value: function (options = {}) {
                if (!check(this) || options === null || typeof options !== "object") {
                  return Promise.reject(makeError("TypeError", "Invalid file read", "InvalidResource"));
                }
                const maxBytes = options.maxBytes ?? 65536;
                return new Promise((resolve, reject) => fileRead(this, maxBytes, resolve, reject))
                  .then((bytes) => new Uint8Array(bytes));
              }, writable: false, enumerable: false, configurable: false
            },
            write: {
              value: function (bytes) {
                if (!check(this) || !(bytes instanceof Uint8Array)) {
                  return Promise.reject(makeError("TypeError", "Invalid file write", "InvalidResource"));
                }
                return new Promise((resolve, reject) =>
                  fileWrite(this, Array.from(bytes), resolve, reject)).then(() => undefined);
              }, writable: false, enumerable: false, configurable: false
            },
            stat: {
              value: function () {
                if (!check(this)) {
                  return Promise.reject(makeError("TypeError", "Invalid file stat", "InvalidResource"));
                }
                return new Promise((resolve, reject) => fileStat(this, resolve, reject))
                  .then(decodeMetadata);
              }, writable: false, enumerable: false, configurable: false
            },
            close: {
              value: function () {
                if (!check(this)) {
                  return Promise.reject(makeError("TypeError", "Invalid file receiver", "InvalidResource"));
                }
                return new Promise((resolve, reject) => fsClose(this, resolve, reject))
                  .then(() => undefined);
              }, writable: false, enumerable: false, configurable: false
            }
          });
        }
        return Object.freeze(object);
      };
      const encode = (value) => {
        if (value instanceof Uint8Array) {
          return JSON.stringify({ type: "bytes", value: Array.from(value) });
        }
        return JSON.stringify({ type: "value", value: value === undefined ? null : value });
      };
      const safeError = (error) => {
        try { return `${String(error?.name ?? "Error")}: ${String(error?.message ?? error)}`; }
        catch (_) { return "Error: JavaScript evaluation failed"; }
      };
      const evaluate = (identifier, source) => {
        let result;
        try { result = (0, eval)(source); }
        catch (error) { evaluationDone(identifier, false, safeError(error)); return; }
        Promise.resolve(result).then(
          (value) => evaluationDone(identifier, true, encode(value)),
          (error) => evaluationDone(identifier, false, safeError(error))
        );
      };
      return Object.freeze({ evaluate, decorateHostObject, makeError });
    })()
    """#
}
