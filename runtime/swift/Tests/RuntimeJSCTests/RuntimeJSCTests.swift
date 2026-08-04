import Dispatch
import Foundation
import Testing

@testable import RuntimeEmbed
@testable import RuntimeJSC

private final class TestExecutor: JavaScriptSerialExecutor, @unchecked Sendable {
  let queue = DispatchQueue(label: "runtime.test.js-executor")

  init() {
    queue.setSpecific(key: key, value: 1)
  }

  func enqueue(_ operation: @escaping @Sendable () -> Void) {
    queue.async(execute: operation)
  }

  func preconditionIsCurrent() {
    dispatchPrecondition(condition: .onQueue(queue))
  }

  private let key = DispatchSpecificKey<Int>()
}

private actor OperationGate {
  func wait() async {
    if released { return }
    await withCheckedContinuation { continuation in
      self.continuation = continuation
    }
  }

  func release() {
    released = true
    continuation?.resume()
    continuation = nil
  }

  private var continuation: CheckedContinuation<Void, Never>?
  private var released = false
}

private actor OpaquePermissionCodec: PermissionCodec {
  func seal(authenticatedEnvelope: Data) async throws -> Data {
    let blob = Data("opaque-\(nextID)".utf8)
    nextID += 1
    envelopes[blob] = authenticatedEnvelope
    return blob
  }

  func open(opaqueBlob: Data) async throws -> Data {
    guard let envelope = envelopes[opaqueBlob] else {
      throw PermissionPersistenceError.invalidEnvelope
    }
    return envelope
  }

  private var nextID = 1
  private var envelopes: [Data: Data] = [:]
}

private actor PersistentScopeState {
  func update(locator: String, identity: Data) {
    self.locator = locator
    self.identity = identity
  }

  func resolve(_ opaqueLocator: Data) throws -> ResolvedPermissionScope {
    guard opaqueLocator == Data("test-bookmark".utf8), let locator, let identity else {
      throw PermissionPersistenceError.scopeMismatch
    }
    return ResolvedPermissionScope(locator: locator, stableIdentity: identity)
  }

  private var locator: String?
  private var identity: Data?
}

private func javascriptLiteral(_ value: String) throws -> String {
  String(decoding: try JSONEncoder().encode(value), as: UTF8.self)
}

private func makeTestDirectory(_ label: String) throws -> URL {
  let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
    "zintl-swift-fs-\(label)-\(UUID().uuidString)",
    isDirectory: true
  )
  try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
  if directory.path.hasPrefix("/var/") {
    return URL(fileURLWithPath: "/private\(directory.path)", isDirectory: true)
  }
  return directory
}

@Test("zero op limits are rejected")
// Verifies public op configuration cannot select an unbounded zero default.
func rejectsZeroLimits() {
  #expect(throws: EmbeddedRuntimeError.self) {
    _ = try OpLimits(maxInputBytes: 0, maxOutputBytes: 1)
  }
}

@Test("embedding configuration rejects invalid lifecycle and audit limits")
// Verifies public configuration cannot select zero, excessive, or effectively unbounded limits.
func rejectsInvalidEmbeddingConfiguration() {
  #expect(throws: EmbeddedRuntimeError.invalidLimits) {
    _ = try EmbeddedRuntimeConfiguration(permissionTimeoutNanoseconds: 0)
  }
  #expect(throws: EmbeddedRuntimeError.invalidLimits) {
    _ = try EmbeddedRuntimeConfiguration(maximumAuditEvents: 65_537)
  }
}

@Test("embedding API accepts a non-main serial executor")
// Verifies the public skeleton does not require MainActor or DispatchQueue.main.
func acceptsHostExecutor() {
  _ = EmbeddedRuntime(executor: TestExecutor())
}

@Test("JavaScriptCore passes the reusable engine conformance harness")
// Verifies evaluation, Promise, rejection, and ambient-authority cases reusable by future engines.
func javascriptCoreEngineConformance() async throws {
  let adapter = try await JavaScriptCoreAdapter.create(
    runtimeID: 99,
    executor: TestExecutor(),
    dispatcher: { _, _, _ in
      .failure(HostDispatchFailure(name: "Error", message: "unused", code: "Internal"))
    }
  )
  let report = try await EngineConformanceHarness.run(on: adapter)
  #expect(report.passed)
  await adapter.shutdown()
}

@Test("custom async op is concise and permission guarded")
// Verifies a registered async op runs only after an allow decision.
func customAsyncOpRunsAfterPermission() async throws {
  let runtime = EmbeddedRuntime(
    executor: TestExecutor(),
    permissionResolver: { _ in .allow(scope: Data(), rights: 1, quota: 1) }
  )
  try runtime.registerOp(
    name: "com.example.image.decode",
    version: 1,
    limits: try OpLimits(maxInputBytes: 4, maxOutputBytes: 4),
    permission: "com.example.image.decode"
  ) { _, input in
    input
  }
  let result = try await runtime.invokeRegisteredOpForTesting(
    name: "com.example.image.decode",
    version: 1,
    requestID: 1,
    input: Data([1, 2])
  )
  #expect(result == Data([1, 2]))
}

@Test("missing permission resolver denies custom op")
// Verifies callback absence cannot become implicit custom-op authority.
func missingResolverDenies() async throws {
  let runtime = EmbeddedRuntime(executor: TestExecutor())
  try runtime.registerOp(
    name: "com.example.image.decode",
    version: 1,
    limits: try OpLimits(maxInputBytes: 4, maxOutputBytes: 4),
    permission: "com.example.image.decode"
  ) { _, input in
    input
  }
  await #expect(throws: EmbeddedRuntimeError.permissionDenied) {
    try await runtime.invokeRegisteredOpForTesting(
      name: "com.example.image.decode",
      version: 1,
      requestID: 1,
      input: Data()
    )
  }
}

@Test("permission decisions cannot grant empty rights or quota")
// Verifies malformed trusted decisions fail closed before the host handler executes.
func malformedPermissionDecisionDenies() async throws {
  let runtime = EmbeddedRuntime(
    executor: TestExecutor(),
    permissionResolver: { _ in .allow(scope: Data(), rights: 0, quota: 0) }
  )
  try runtime.registerOp(
    name: "com.example.denied.invalid-grant",
    version: 1,
    limits: try OpLimits(maxInputBytes: 1, maxOutputBytes: 1),
    permission: "com.example.denied.invalid-grant"
  ) { _, input in input }
  await #expect(throws: EmbeddedRuntimeError.permissionDenied) {
    try await runtime.invokeRegisteredOpForTesting(
      name: "com.example.denied.invalid-grant",
      version: 1,
      requestID: 1,
      input: Data()
    )
  }
}

@Test("registry freezes after configuration")
// Verifies runtime-start registry mutation is rejected.
func frozenRegistryRejectsMutation() throws {
  let runtime = EmbeddedRuntime(executor: TestExecutor())
  runtime.freezeRegistry()
  #expect(throws: EmbeddedRuntimeError.registryFrozen) {
    try runtime.registerOp(
      name: "com.example.image.decode",
      version: 1,
      limits: try OpLimits(maxInputBytes: 1, maxOutputBytes: 1),
      permission: "com.example.image.decode"
    ) { _, input in input }
  }
}

@Test("JSC Promise invokes registered async op end to end")
// Verifies Promise dispatch, host execution, settlement, and microtasks on a non-main executor.
func jscPromiseInvokesCustomOp() async throws {
  let runtime = EmbeddedRuntime(
    executor: TestExecutor(),
    permissionResolver: { _ in .allow(scope: Data(), rights: 1, quota: 1) }
  )
  try runtime.registerOp(
    name: "com.example.bytes.echo",
    version: 1,
    limits: try OpLimits(maxInputBytes: 4, maxOutputBytes: 4),
    permission: "com.example.bytes.echo"
  ) { _, input in
    dispatchPrecondition(condition: .notOnQueue(.main))
    return input
  }
  let adapter = try await runtime.makeJavaScriptCoreAdapter(runtimeID: 1)
  let result = try await adapter.evaluate(
    "Zintl.invoke('com.example.bytes.echo', new Uint8Array([1, 2])).then(v => v[0] + v[1])"
  )
  #expect(result.json == #"{"type":"value","value":3}"#)
  await adapter.shutdown()
}

@Test("directory access is denied when permission callback is absent")
// Verifies an untrusted locator alone never causes filesystem authority or an open attempt.
func missingDirectoryPermissionResolverDenies() async throws {
  let runtime = EmbeddedRuntime(executor: TestExecutor())
  let adapter = try await runtime.makeJavaScriptCoreAdapter(runtimeID: 1)
  let locator = "/zintl-test-must-not-exist-\(UUID().uuidString)"
  let result = try await adapter.evaluate(
    "Zintl.requestDirectory(\(try javascriptLiteral(locator)), { read: true }).catch(error => error.code)"
  )
  #expect(result.json == #"{"type":"value","value":"PermissionDenied"}"#)
  #expect(!FileManager.default.fileExists(atPath: locator))
  await adapter.shutdown()
}

@Test("read-only directory grant stays scoped and opaque")
// Verifies approved read/stat, denied write/traversal, opaque branding, and close end to end.
func readOnlyDirectoryGrantEndToEnd() async throws {
  let directory = try makeTestDirectory("readonly")
  defer { try? FileManager.default.removeItem(at: directory) }
  try Data([1, 2, 3]).write(to: directory.appendingPathComponent("data.bin"))
  let path = directory.path
  let runtime = EmbeddedRuntime(
    executor: TestExecutor(),
    permissionResolver: { request in
      #expect(request.requestedDirectory == path)
      #expect(request.requestedRights == 9)
      #expect(request.kind == "zintl.permission.fs.directory")
      return .allow(
        scope: request.requestedScope,
        rights: request.requestedRights,
        quota: 64 * 1_024
      )
    }
  )
  let adapter = try await runtime.makeJavaScriptCoreAdapter(runtimeID: 1)
  let result = try await adapter.evaluate(
    """
    (async () => {
      const directory = await Zintl.requestDirectory(
        \(try javascriptLiteral(path)), { read: true, metadata: true });
      const file = await directory.openRelative(
        "data.bin", { read: true, metadata: true });
      const bytes = await file.read({ maxBytes: 16 });
      const metadata = await file.stat();
      const write = await file.write(new Uint8Array([9]))
        .then(() => "unexpected", error => error.code);
      const traversal = await directory.openRelative("../outside", { read: true })
        .then(() => "unexpected", error => error.code);
      const borrowed = await file.read.call(directory, { maxBytes: 1 })
        .then(() => "unexpected", error => error.code);
      const visibleKeys = [Object.keys(directory), Object.keys(file)];
      await file.close();
      const closed = await file.read({ maxBytes: 1 })
        .then(() => "unexpected", error => error.code);
      await directory.close();
      return [
        Array.from(bytes), metadata.kind, metadata.size, write, traversal, borrowed,
        visibleKeys, closed
      ];
    })()
    """
  )
  #expect(
    result.json
      == #"{"type":"value","value":[[1,2,3],"file",3,"PermissionDenied","InvalidArgument","InvalidResource",[[],[]],"InvalidResource"]}"#
  )
  #expect(
    !FileManager.default.fileExists(atPath: directory.appendingPathComponent("created.bin").path))
  await adapter.shutdown()
}

@Test("directory permission deny and timeout mint no authority")
// Verifies explicit deny and callback deadline both reject without creating a capability.
func directoryPermissionDenyAndTimeout() async throws {
  let directory = try makeTestDirectory("deny-timeout")
  defer { try? FileManager.default.removeItem(at: directory) }
  let locator = try javascriptLiteral(directory.path)
  let deniedRuntime = EmbeddedRuntime(
    executor: TestExecutor(),
    permissionResolver: { _ in .deny }
  )
  let deniedAdapter = try await deniedRuntime.makeJavaScriptCoreAdapter(runtimeID: 1)
  let denied = try await deniedAdapter.evaluate(
    "Zintl.requestDirectory(\(locator), { read: true }).catch(error => error.code)"
  )
  #expect(denied.json == #"{"type":"value","value":"PermissionDenied"}"#)
  await deniedAdapter.shutdown()

  let timeoutRuntime = EmbeddedRuntime(
    executor: TestExecutor(),
    permissionResolver: { _ in
      try await Task.sleep(for: .seconds(30))
      return .deny
    },
    permissionTimeoutNanoseconds: 1_000_000
  )
  let timeoutAdapter = try await timeoutRuntime.makeJavaScriptCoreAdapter(runtimeID: 2)
  let timedOut = try await timeoutAdapter.evaluate(
    "Zintl.requestDirectory(\(locator), { read: true }).catch(error => error.code)"
  )
  #expect(timedOut.json == #"{"type":"value","value":"TimedOut"}"#)
  await timeoutAdapter.shutdown()
}

@Test("permission export and import authenticate, attenuate, and reopen directory identity")
// Verifies opaque cross-runtime restore plus tamper, replay, expiry, audience, rights, and scope attacks.
func permissionExportImportEndToEnd() async throws {
  let directory = try makeTestDirectory("persistence")
  let replacement = try makeTestDirectory("persistence-replacement")
  defer {
    try? FileManager.default.removeItem(at: directory)
    try? FileManager.default.removeItem(at: replacement)
  }
  try Data([4, 5, 6]).write(to: directory.appendingPathComponent("data.bin"))
  let codec = OpaquePermissionCodec()
  let scope = PersistentScopeState()
  let persistence = try PermissionPersistenceConfiguration(
    issuer: "com.example.zintl-tests",
    audience: "com.example.application",
    codec: codec,
    encodeScopeLocator: { _ in Data("test-bookmark".utf8) },
    resolveScopeLocator: { locator in try await scope.resolve(locator) }
  )
  let sourceRuntime = EmbeddedRuntime(
    executor: TestExecutor(),
    permissionResolver: { request in
      .allow(scope: request.requestedScope, rights: request.requestedRights, quota: 64)
    },
    permissionPersistence: persistence
  )
  let sourceAdapter = try await sourceRuntime.makeJavaScriptCoreAdapter(runtimeID: 10)
  let sourcePermission = try await sourceAdapter.requestDirectoryPermission(
    locator: directory.path,
    rights: 9
  )
  let identity = try await sourceAdapter.directoryIdentity(sourcePermission)
  await scope.update(locator: directory.path, identity: identity)
  let blob = try await sourceRuntime.exportPermission(
    sourcePermission,
    from: sourceAdapter,
    expiresAt: 100,
    nonce: Data(repeating: 7, count: 16)
  )
  #expect(!blob.contains(directory.path.data(using: .utf8)!))

  let destinationRuntime = EmbeddedRuntime(
    executor: TestExecutor(),
    permissionPersistence: persistence
  )
  let destinationAdapter = try await destinationRuntime.makeJavaScriptCoreAdapter(runtimeID: 11)
  let imported = try await destinationRuntime.importPermission(
    blob,
    into: destinationAdapter,
    requestedRights: 1,
    requestedQuota: 32,
    now: 10
  )
  try await destinationAdapter.installDirectoryPermission(imported, globalName: "savedDirectory")
  let result = try await destinationAdapter.evaluate(
    """
    Promise.all([
      savedDirectory.readRelative("data.bin", { maxBytes: 8 }).then(Array.from),
      savedDirectory.writeRelative("denied", new Uint8Array([1]), { create: true })
        .then(() => "unexpected", error => error.code),
      Object.keys(savedDirectory)
    ])
    """
  )
  #expect(
    result.json
      == #"{"type":"value","value":[[4,5,6],"PermissionDenied",[]]}"#
  )
  await #expect(throws: PermissionPersistenceError.replay) {
    _ = try await destinationRuntime.importPermission(
      blob,
      into: destinationAdapter,
      requestedRights: 1,
      requestedQuota: 1,
      now: 10
    )
  }
  var tampered = blob
  tampered[0] ^= 1
  await #expect(throws: PermissionPersistenceError.invalidEnvelope) {
    _ = try await destinationRuntime.importPermission(
      tampered,
      into: destinationAdapter,
      requestedRights: 1,
      requestedQuota: 1,
      now: 10
    )
  }

  let secondBlob = try await sourceRuntime.exportPermission(
    sourcePermission,
    from: sourceAdapter,
    expiresAt: 100,
    nonce: Data(repeating: 8, count: 16)
  )
  await #expect(throws: PermissionPersistenceError.expired) {
    _ = try await destinationRuntime.importPermission(
      secondBlob,
      into: destinationAdapter,
      requestedRights: 1,
      requestedQuota: 1,
      now: 100
    )
  }
  await #expect(throws: PermissionPersistenceError.escalation) {
    _ = try await destinationRuntime.importPermission(
      secondBlob,
      into: destinationAdapter,
      requestedRights: 2,
      requestedQuota: 1,
      now: 10
    )
  }
  await scope.update(locator: replacement.path, identity: identity)
  await #expect(throws: PermissionPersistenceError.scopeMismatch) {
    _ = try await destinationRuntime.importPermission(
      secondBlob,
      into: destinationAdapter,
      requestedRights: 1,
      requestedQuota: 1,
      now: 10
    )
  }
  await scope.update(locator: directory.path, identity: identity)
  #expect(
    try await destinationRuntime.importPermission(
      secondBlob,
      into: destinationAdapter,
      requestedRights: 1,
      requestedQuota: 1,
      now: 10
    ).rights == 1
  )

  let wrongAudiencePersistence = try PermissionPersistenceConfiguration(
    issuer: "com.example.zintl-tests",
    audience: "com.example.other",
    codec: codec,
    encodeScopeLocator: { _ in Data("test-bookmark".utf8) },
    resolveScopeLocator: { locator in try await scope.resolve(locator) }
  )
  let wrongAudienceRuntime = EmbeddedRuntime(
    executor: TestExecutor(),
    permissionPersistence: wrongAudiencePersistence
  )
  let wrongAudienceAdapter = try await wrongAudienceRuntime.makeJavaScriptCoreAdapter(runtimeID: 12)
  await #expect(throws: PermissionPersistenceError.wrongAudience) {
    _ = try await wrongAudienceRuntime.importPermission(
      blob,
      into: wrongAudienceAdapter,
      requestedRights: 1,
      requestedQuota: 1,
      now: 10
    )
  }
  await wrongAudienceAdapter.shutdown()
  await destinationAdapter.shutdown()
  await sourceAdapter.shutdown()
}

@Test("shutdown cancels a pending directory permission exactly once")
// Verifies callback cancellation cannot mint filesystem authority after adapter shutdown.
func shutdownCancelsPendingDirectoryPermission() async throws {
  let directory = try makeTestDirectory("permission-shutdown")
  defer { try? FileManager.default.removeItem(at: directory) }
  let (started, startedContinuation) = AsyncStream<Void>.makeStream()
  let gate = OperationGate()
  let runtime = EmbeddedRuntime(
    executor: TestExecutor(),
    permissionResolver: { request in
      startedContinuation.yield()
      await gate.wait()
      return .allow(
        scope: request.requestedScope,
        rights: request.requestedRights,
        quota: 1_024
      )
    }
  )
  let adapter = try await runtime.makeJavaScriptCoreAdapter(runtimeID: 3)
  let locator = try javascriptLiteral(directory.path)
  let evaluation = Task {
    try await adapter.evaluate(
      "Zintl.requestDirectory(\(locator), { read: true }).then(() => true)"
    )
  }
  var iterator = started.makeAsyncIterator()
  _ = await iterator.next()
  await adapter.shutdown()
  await #expect(throws: JavaScriptException.self) {
    _ = try await evaluation.value
  }
  await gate.release()
}

@Test("completion drain yields across bounded batches")
// Verifies more than one drain budget of completions settles without losing wakeups.
func boundedCompletionDrainReschedules() async throws {
  let adapter = try await JavaScriptCoreAdapter.create(
    runtimeID: 1,
    executor: TestExecutor(),
    dispatcher: { _, input, _ in .success(input) }
  )
  let result = try await adapter.evaluate(
    """
    Promise.all(Array.from({ length: 130 }, (_, value) =>
      Zintl.invoke('com.example.echo', new Uint8Array([value & 255]))
    )).then(values => values.reduce((sum, value) => sum + value[0], 0))
    """
  )
  #expect(result.json == #"{"type":"value","value":8385}"#)
  await adapter.shutdown()
}

@Test("host rejection crosses the FFI completion queue")
// Verifies structured permission failures survive Rust completion encoding and Promise rejection.
func ffiCompletionPreservesRejection() async throws {
  let runtime = EmbeddedRuntime(executor: TestExecutor())
  try runtime.registerOp(
    name: "com.example.denied",
    version: 1,
    limits: try OpLimits(maxInputBytes: 1, maxOutputBytes: 1),
    permission: "com.example.denied"
  ) { _, input in input }
  let adapter = try await runtime.makeJavaScriptCoreAdapter(runtimeID: 1)
  let result = try await adapter.evaluate(
    "Zintl.invoke('com.example.denied', new Uint8Array()).catch(error => error.code)"
  )
  #expect(result.json == #"{"type":"value","value":"PermissionDenied"}"#)
  await adapter.shutdown()
}

@Test("sleep uses the Rust monotonic timer completion pipeline")
// Verifies a JS timer settles through the bounded Rust queue on the non-main JS executor.
func javascriptSleepCompletes() async throws {
  let adapter = try await JavaScriptCoreAdapter.create(
    runtimeID: 1,
    executor: TestExecutor(),
    dispatcher: { _, _, _ in
      .failure(HostDispatchFailure(name: "Error", message: "unused", code: "Internal"))
    }
  )
  let result = try await adapter.evaluate("Zintl.sleep(1).then(() => 42)")
  #expect(result.json == #"{"type":"value","value":42}"#)
  let invalid = try await adapter.evaluate("Zintl.sleep(-1).catch(error => error.code)")
  #expect(invalid.json == #"{"type":"value","value":"InvalidArgument"}"#)
  await adapter.shutdown()
}

@Test("shutdown cancels a pending Rust timer exactly once")
// Verifies timer wake-task cancellation and Rust cancellation settle one pending Promise safely.
func shutdownCancelsJavaScriptSleep() async throws {
  let adapter = try await JavaScriptCoreAdapter.create(
    runtimeID: 1,
    executor: TestExecutor(),
    dispatcher: { _, _, _ in
      .failure(HostDispatchFailure(name: "Error", message: "unused", code: "Internal"))
    }
  )
  let evaluation = Task { try await adapter.evaluate("Zintl.sleep(30000).then(() => true)") }
  try await Task.sleep(for: .milliseconds(1))
  await adapter.shutdown()
  await #expect(throws: JavaScriptException.self) {
    _ = try await evaluation.value
  }
}

@Test("private-slot capability rejects forgery and receiver spoofing")
// Verifies plain objects, borrowed methods, and cross-runtime brands cannot forge authority.
func hostObjectBrandIsUnforgeable() async throws {
  let adapter = try await JavaScriptCoreAdapter.create(
    runtimeID: 1,
    executor: TestExecutor(),
    dispatcher: { _, _, _ in
      .failure(
        HostDispatchFailure(name: "Error", message: "unused", code: "Internal"))
    }
  )
  try await adapter.installHostObject(globalName: "capability", kind: .capability, objectID: 1)
  try await adapter.installHostObject(globalName: "resource", kind: .resource, objectID: 3)
  #if DEBUG
    try await adapter.installHostObjectForTesting(
      globalName: "foreignCapability", objectRuntimeID: 2, kind: .capability, objectID: 2)
  #endif
  let result = try await adapter.evaluate(
    """
    [
      capability.assertBrand(),
      resource.assertBrand(),
      (() => { try { capability.assertBrand.call({}); return false; }
               catch (e) { return e.code === "InvalidCapability"; } })(),
      (() => { try { capability.assertBrand.call(foreignCapability); return false; }
               catch (e) { return e.code === "InvalidCapability"; } })()
    ]
    """
  )
  #expect(result.json == #"{"type":"value","value":[true,true,true,true]}"#)
  await adapter.shutdown()
}

@Test("bootstrap hides native bindings and ambient OS globals")
// Verifies untrusted script cannot access bridge callbacks or ambient host authority.
func bootstrapHidesNativeBindings() async throws {
  let adapter = try await JavaScriptCoreAdapter.create(
    runtimeID: 1,
    executor: TestExecutor(),
    dispatcher: { _, _, _ in
      .failure(
        HostDispatchFailure(name: "Error", message: "unused", code: "Internal"))
    }
  )
  let result = try await adapter.evaluate(
    """
    [typeof __zintlNativeSubmit, typeof __zintlEvaluationDone,
     typeof __zintlCheckCapability, typeof __zintlCheckResource,
     typeof __zintlRequestDirectory, typeof __zintlFsRead,
     typeof __zintlFsWrite, typeof __zintlFsStat, typeof __zintlFsClose,
     typeof __zintlFsOpen, typeof __zintlFileRead,
     typeof __zintlFileWrite, typeof __zintlFileStat,
     typeof process, typeof require, typeof Deno, typeof fetch]
    """
  )
  #expect(
    result.json
      == #"{"type":"value","value":["undefined","undefined","undefined","undefined","undefined","undefined","undefined","undefined","undefined","undefined","undefined","undefined","undefined","undefined","undefined","undefined","undefined"]}"#
  )
  await adapter.shutdown()
}

@Test("unknown builtin names cannot reach the host dispatcher")
// Verifies the implementation namespace stays reserved after runtime start.
func unknownBuiltinFailsClosed() async throws {
  let adapter = try await JavaScriptCoreAdapter.create(
    runtimeID: 1,
    executor: TestExecutor(),
    dispatcher: { _, input, _ in .success(input) }
  )
  let result = try await adapter.evaluate(
    "Zintl.invoke('zintl.builtin.forged', new Uint8Array()).catch(error => error.code)"
  )
  #expect(result.json == #"{"type":"value","value":"NotSupported"}"#)
  await adapter.shutdown()
}

@Test("host-object globals have a hard count limit")
// Verifies trusted installation cannot grow retained JSC host objects without a bound.
func hostObjectInstallationIsBounded() async throws {
  let adapter = try await JavaScriptCoreAdapter.create(
    runtimeID: 1,
    executor: TestExecutor(),
    dispatcher: { _, _, _ in
      .failure(HostDispatchFailure(name: "Error", message: "unused", code: "Internal"))
    }
  )
  for index in 0..<1_024 {
    try await adapter.installHostObject(
      globalName: "resource\(index)",
      kind: .resource,
      objectID: UInt64(index + 1)
    )
  }
  await #expect(throws: JavaScriptException.self) {
    try await adapter.installHostObject(
      globalName: "resourceOverflow",
      kind: .resource,
      objectID: 1_025
    )
  }
  await adapter.shutdown()
}

@Test("shutdown rejects pending Promise and cancels host task")
// Verifies context shutdown leaves no pending Promise or cooperative host operation.
func shutdownSettlesPendingPromise() async throws {
  let (started, startedContinuation) = AsyncStream<Void>.makeStream()
  let runtime = EmbeddedRuntime(
    executor: TestExecutor(),
    permissionResolver: { _ in .allow(scope: Data(), rights: 1, quota: 1) }
  )
  try runtime.registerOp(
    name: "com.example.slow.operation",
    version: 1,
    limits: try OpLimits(maxInputBytes: 1, maxOutputBytes: 1),
    permission: "com.example.slow.operation"
  ) { _, _ in
    startedContinuation.yield()
    try await Task.sleep(for: .seconds(30))
    return Data()
  }
  let adapter = try await runtime.makeJavaScriptCoreAdapter(runtimeID: 1)
  let evaluation = Task {
    try await adapter.evaluate(
      "Zintl.invoke('com.example.slow.operation', new Uint8Array()).then(() => true)")
  }
  var iterator = started.makeAsyncIterator()
  _ = await iterator.next()
  await adapter.shutdown()
  await #expect(throws: JavaScriptException.self) {
    _ = try await evaluation.value
  }
}

@Test("EmbeddedRuntime owns tracked adapter shutdown and bounded redacted audit")
// Verifies build/start/shutdown transitions, pending cancellation, restart rejection, and audit bounds.
func embeddedRuntimeLifecycleAndAudit() async throws {
  let configuration = try EmbeddedRuntimeConfiguration(maximumAuditEvents: 3)
  let gate = OperationGate()
  let (started, startedContinuation) = AsyncStream<Void>.makeStream()
  let runtime = EmbeddedRuntime.build(
    configuration: configuration,
    executor: TestExecutor(),
    permissionResolver: { _ in .allow(scope: Data(), rights: 1, quota: 1) }
  )
  try runtime.registerOp(
    name: "com.example.lifecycle.slow",
    version: 1,
    limits: try OpLimits(maxInputBytes: 1, maxOutputBytes: 1),
    permission: "com.example.lifecycle.permission"
  ) { _, _ in
    startedContinuation.yield()
    await gate.wait()
    return Data()
  }
  #expect(runtime.lifecycle == .configured)
  let adapter = try await runtime.startJavaScriptCore(runtimeID: 20)
  #expect(runtime.lifecycle == .running)
  await #expect(throws: EmbeddedRuntimeError.duplicateRuntimeID) {
    _ = try await runtime.startJavaScriptCore(runtimeID: 20)
  }
  let evaluation = Task {
    try await adapter.evaluate(
      "Zintl.invoke('com.example.lifecycle.slow', new Uint8Array()).then(() => true)"
    )
  }
  var iterator = started.makeAsyncIterator()
  _ = await iterator.next()
  await runtime.shutdown()
  #expect(runtime.lifecycle == .terminated)
  await #expect(throws: JavaScriptException.self) {
    _ = try await evaluation.value
  }
  await #expect(throws: EmbeddedRuntimeError.runtimeUnavailable) {
    _ = try await runtime.startJavaScriptCore(runtimeID: 21)
  }
  await gate.release()

  let audit = runtime.auditSnapshot()
  #expect(audit.count <= 3)
  #expect(audit.last?.category == .lifecycle)
  #expect(audit.last?.outcome == .shutdown)
  let renderedAudit = String(reflecting: audit)
  #expect(!renderedAudit.contains("Uint8Array"))
  #expect(!renderedAudit.contains("permission-shutdown"))
  #expect(!renderedAudit.contains("opaque-"))
}

@Test("host timeout does not wait for a non-cooperative callback")
// Verifies timeout wins once while an ignored cancellation cannot block adapter shutdown.
func timeoutBoundsNonCooperativeHostOperation() async throws {
  let gate = OperationGate()
  let runtime = EmbeddedRuntime(
    executor: TestExecutor(),
    permissionResolver: { _ in .allow(scope: Data(), rights: 1, quota: 1) }
  )
  try runtime.registerOp(
    name: "com.example.noncooperative.operation",
    version: 1,
    limits: try OpLimits(
      maxInputBytes: 1,
      maxOutputBytes: 1,
      timeoutNanoseconds: 1_000_000
    ),
    permission: "com.example.noncooperative.operation"
  ) { _, _ in
    await gate.wait()
    return Data()
  }
  let adapter = try await runtime.makeJavaScriptCoreAdapter(runtimeID: 1)
  let result = try await adapter.evaluate(
    "Zintl.invoke('com.example.noncooperative.operation', new Uint8Array()).catch(error => error.code)"
  )
  #expect(result.json == #"{"type":"value","value":"TimedOut"}"#)
  await adapter.shutdown()
  await gate.release()
}
