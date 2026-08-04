import AppKit
import Foundation
import RuntimeEmbed
import RuntimeJSC

public actor DemoRuntimeHost {
  public init(dialog: PermissionDialogCoordinator, storageDirectory: URL? = nil) throws {
    self.dialog = dialog
    let base =
      try storageDirectory
      ?? FileManager.default.url(
        for: .applicationSupportDirectory,
        in: .userDomainMask,
        appropriateFor: nil,
        create: true
      ).appendingPathComponent("PermissionReplDemo", isDirectory: true)
    try FileManager.default.createDirectory(at: base, withIntermediateDirectories: true)
    blobURL = base.appendingPathComponent("permission.blob")
    codec = try DemoPermissionCodec(keyURL: base.appendingPathComponent("permission.key"))
  }

  public func start() async throws -> DemoRuntimeStatus {
    guard runtime == nil else { return .alreadyRunning }
    let persistence = try PermissionPersistenceConfiguration(
      issuer: "dev.zintl.PermissionReplDemo",
      audience: "dev.zintl.PermissionReplDemo",
      codec: codec,
      encodeScopeLocator: { path in try DemoScopeLocator.encode(path: path) },
      resolveScopeLocator: { bookmark in try DemoScopeLocator.resolve(bookmark: bookmark) }
    )
    let runtime = EmbeddedRuntime(
      executor: DispatchQueueJavaScriptExecutor(label: "dev.zintl.demo.javascript"),
      permissionResolver: { [dialog] request in
        if request.requestedDirectory != nil {
          return await dialog.resolve(request)
        }
        if request.kind == "dev.zintl.demo.reverse" {
          return .allow(scope: request.requestedScope, rights: 1, quota: 64 * 1_024)
        }
        return .deny
      },
      permissionPersistence: persistence
    )
    try runtime.registerOp(
      name: "dev.zintl.demo.reverse",
      version: 1,
      limits: try OpLimits(maxInputBytes: 64 * 1_024, maxOutputBytes: 64 * 1_024),
      permission: "dev.zintl.demo.reverse"
    ) { _, input in
      dispatchPrecondition(condition: .notOnQueue(.main))
      try Task.checkCancellation()
      return Data(input.reversed())
    }
    let adapter = try await runtime.startJavaScriptCore(runtimeID: nextRuntimeID)
    nextRuntimeID += 1
    self.runtime = runtime
    self.adapter = adapter
    guard FileManager.default.fileExists(atPath: blobURL.path) else {
      return .readyWithoutSavedPermission
    }
    do {
      let blob = try Data(contentsOf: blobURL)
      let imported = try await runtime.importPermission(
        blob,
        into: adapter,
        requestedRights: 1,
        requestedQuota: 4 * 1_024 * 1_024,
        now: Self.now()
      )
      try await adapter.installDirectoryPermission(imported, globalName: "savedDirectory")
      return .readyWithImportedPermission
    } catch {
      return .readyWithInvalidSavedPermission
    }
  }

  public func evaluate(_ source: String) async throws -> JavaScriptEvaluationResult {
    guard let adapter, let runtime else { throw DemoHostError.notRunning }
    let result: Result<JavaScriptEvaluationResult, Error>
    do {
      result = .success(try await adapter.evaluate(source))
    } catch {
      result = .failure(error)
    }
    try await persistRequestedPermission(adapter: adapter, runtime: runtime)
    return try result.get()
  }

  private func persistRequestedPermission(
    adapter: JavaScriptCoreAdapter,
    runtime: EmbeddedRuntime
  ) async throws {
    if let request = await dialog.consumeSaveRequest(),
      let locator = request.requestedDirectory
    {
      let permission = try await adapter.requestDirectoryPermission(
        locator: locator,
        rights: request.requestedRights
      )
      let blob = try await runtime.exportPermission(
        permission,
        from: adapter,
        expiresAt: Self.now() + 30 * 24 * 60 * 60
      )
      try blob.write(to: blobURL, options: [.atomic, .completeFileProtection])
    }
  }

  public func cancelAndRecreate() async throws -> DemoRuntimeStatus {
    await shutdown()
    return try await start()
  }

  public func shutdown() async {
    await dialog.cancelAll()
    if let runtime { await runtime.shutdown() }
    adapter = nil
    runtime = nil
  }

  public func recreate() async throws -> DemoRuntimeStatus {
    await shutdown()
    return try await start()
  }

  private static func now() -> UInt64 {
    UInt64(max(0, Date().timeIntervalSince1970))
  }

  private let dialog: PermissionDialogCoordinator
  private let codec: DemoPermissionCodec
  private let blobURL: URL
  private var runtime: EmbeddedRuntime?
  private var adapter: JavaScriptCoreAdapter?
  private var nextRuntimeID: UInt64 = 1
}

public enum DemoHostError: Error, Equatable {
  case notRunning
}

public enum DemoRuntimeStatus: Sendable, Equatable {
  case alreadyRunning
  case readyWithoutSavedPermission
  case readyWithImportedPermission
  case readyWithInvalidSavedPermission
}
