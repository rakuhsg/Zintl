import Foundation
import RuntimeJSC

private let envelopeMagic = Data("ZPEM".utf8)
private let envelopeVersion: UInt16 = 1
private let directoryPermissionKind = "zintl.permission.fs.directory"
private let maximumTextBytes = 255
private let maximumScopeLocatorBytes = 64 * 1_024
private let maximumIdentityBytes = 1_024
private let maximumEnvelopeBytes = 128 * 1_024

/// Application-provided authenticated encryption/sealing boundary. A conformer
/// must provide confidentiality and integrity and must not return partial data.
public protocol PermissionCodec: Sendable {
  func seal(authenticatedEnvelope: Data) async throws -> Data
  func open(opaqueBlob: Data) async throws -> Data
}

public struct ResolvedPermissionScope: Sendable, Equatable {
  public let locator: String
  public let stableIdentity: Data

  public init(locator: String, stableIdentity: Data) {
    self.locator = locator
    self.stableIdentity = stableIdentity
  }
}

public typealias PermissionScopeLocatorEncoder =
  @Sendable (_ localLocator: String) async throws ->
  Data
public typealias PermissionScopeLocatorResolver =
  @Sendable (_ opaqueLocator: Data) async throws ->
  ResolvedPermissionScope

public enum PermissionPersistenceError: Error, Equatable {
  case unavailable
  case invalidEnvelope
  case wrongIssuer
  case wrongAudience
  case wrongKind
  case expired
  case replay
  case replayStateFull
  case escalation
  case scopeMismatch
}

public struct PermissionPersistenceConfiguration: Sendable {
  public let issuer: String
  public let audience: String
  public let codec: any PermissionCodec
  public let encodeScopeLocator: PermissionScopeLocatorEncoder
  public let resolveScopeLocator: PermissionScopeLocatorResolver
  public let maximumReplayEntries: Int

  public init(
    issuer: String,
    audience: String,
    codec: any PermissionCodec,
    maximumReplayEntries: Int = 1_024,
    encodeScopeLocator: @escaping PermissionScopeLocatorEncoder,
    resolveScopeLocator: @escaping PermissionScopeLocatorResolver
  ) throws {
    guard Self.validText(issuer), Self.validText(audience), maximumReplayEntries > 0 else {
      throw PermissionPersistenceError.invalidEnvelope
    }
    self.issuer = issuer
    self.audience = audience
    self.codec = codec
    self.maximumReplayEntries = maximumReplayEntries
    self.encodeScopeLocator = encodeScopeLocator
    self.resolveScopeLocator = resolveScopeLocator
  }

  private static func validText(_ value: String) -> Bool {
    !value.isEmpty && value.utf8.count <= maximumTextBytes && !value.contains("\0")
  }
}

actor PermissionReplayState {
  init(maximumEntries: Int) {
    self.maximumEntries = maximumEntries
  }

  func reserve(_ nonce: Data) throws {
    guard !pending.contains(nonce), !consumed.contains(nonce) else {
      throw PermissionPersistenceError.replay
    }
    guard pending.count + consumed.count < maximumEntries else {
      throw PermissionPersistenceError.replayStateFull
    }
    pending.insert(nonce)
  }

  func commit(_ nonce: Data) {
    pending.remove(nonce)
    consumed.insert(nonce)
  }

  func release(_ nonce: Data) {
    pending.remove(nonce)
  }

  private let maximumEntries: Int
  private var pending: Set<Data> = []
  private var consumed: Set<Data> = []
}

private struct PersistentEnvelope {
  let issuer: String
  let audience: String
  let permissionKind: String
  let rights: UInt64
  let quota: UInt64
  let expiresAt: UInt64
  let nonce: Data
  let scopeLocator: Data
  let scopeIdentity: Data

  func encode() throws -> Data {
    guard Self.validText(issuer), Self.validText(audience), Self.validText(permissionKind),
      rights > 0, quota > 0, expiresAt > 0, nonce.count == 16,
      nonce.contains(where: { $0 != 0 }),
      !scopeLocator.isEmpty, scopeLocator.count <= maximumScopeLocatorBytes,
      !scopeIdentity.isEmpty, scopeIdentity.count <= maximumIdentityBytes
    else {
      throw PermissionPersistenceError.invalidEnvelope
    }
    var output = envelopeMagic
    output.appendInteger(envelopeVersion)
    try output.appendSized(Data(issuer.utf8))
    try output.appendSized(Data(audience.utf8))
    try output.appendSized(Data(permissionKind.utf8))
    output.appendInteger(rights)
    output.appendInteger(quota)
    output.appendInteger(expiresAt)
    output.append(nonce)
    try output.appendSized(scopeLocator)
    try output.appendSized(scopeIdentity)
    guard output.count <= maximumEnvelopeBytes else {
      throw PermissionPersistenceError.invalidEnvelope
    }
    return output
  }

  static func decode(_ data: Data) throws -> Self {
    guard data.count <= maximumEnvelopeBytes else {
      throw PermissionPersistenceError.invalidEnvelope
    }
    var reader = EnvelopeReader(data)
    guard try reader.read(count: 4) == envelopeMagic,
      try reader.readInteger(UInt16.self) == envelopeVersion
    else {
      throw PermissionPersistenceError.invalidEnvelope
    }
    let envelope = Self(
      issuer: try reader.readText(maximum: maximumTextBytes),
      audience: try reader.readText(maximum: maximumTextBytes),
      permissionKind: try reader.readText(maximum: maximumTextBytes),
      rights: try reader.readInteger(UInt64.self),
      quota: try reader.readInteger(UInt64.self),
      expiresAt: try reader.readInteger(UInt64.self),
      nonce: try reader.read(count: 16),
      scopeLocator: try reader.readSized(maximum: maximumScopeLocatorBytes),
      scopeIdentity: try reader.readSized(maximum: maximumIdentityBytes)
    )
    guard reader.isFinished else { throw PermissionPersistenceError.invalidEnvelope }
    _ = try envelope.encode()
    return envelope
  }

  private static func validText(_ value: String) -> Bool {
    !value.isEmpty && value.utf8.count <= maximumTextBytes && !value.contains("\0")
  }
}

extension EmbeddedRuntime {
  /// Exports a runtime-local directory permission as codec-sealed opaque bytes.
  public func exportPermission(
    _ permission: DirectoryPermissionHandle,
    from adapter: JavaScriptCoreAdapter,
    expiresAt: UInt64,
    nonce: Data? = nil
  ) async throws -> Data {
    guard let configuration = permissionPersistence else {
      throw PermissionPersistenceError.unavailable
    }
    let identity = try await adapter.directoryIdentity(permission)
    let scopeLocator = try await configuration.encodeScopeLocator(permission.locator)
    let envelope = PersistentEnvelope(
      issuer: configuration.issuer,
      audience: configuration.audience,
      permissionKind: directoryPermissionKind,
      rights: permission.rights,
      quota: permission.quota,
      expiresAt: expiresAt,
      nonce: nonce ?? Self.randomNonce(),
      scopeLocator: scopeLocator,
      scopeIdentity: identity
    )
    do {
      let blob = try await configuration.codec.seal(authenticatedEnvelope: envelope.encode())
      recordAudit(
        category: .permissionPersistence,
        outcome: .succeeded,
        operation: "permission.export"
      )
      return blob
    } catch let error as PermissionPersistenceError {
      recordAudit(
        category: .permissionPersistence, outcome: .failed, operation: "permission.export")
      throw error
    } catch {
      recordAudit(
        category: .permissionPersistence, outcome: .failed, operation: "permission.export")
      throw PermissionPersistenceError.unavailable
    }
  }

  /// Imports, attenuates, identity-checks, and reopens one directory permission.
  public func importPermission(
    _ blob: Data,
    into adapter: JavaScriptCoreAdapter,
    requestedRights: UInt64,
    requestedQuota: UInt64,
    now: UInt64
  ) async throws -> DirectoryPermissionHandle {
    guard let configuration = permissionPersistence,
      let replay = permissionReplayState,
      requestedRights > 0, requestedQuota > 0
    else {
      throw PermissionPersistenceError.unavailable
    }
    let opened: Data
    do {
      opened = try await configuration.codec.open(opaqueBlob: blob)
    } catch {
      throw PermissionPersistenceError.invalidEnvelope
    }
    let envelope = try PersistentEnvelope.decode(opened)
    guard envelope.issuer == configuration.issuer else {
      throw PermissionPersistenceError.wrongIssuer
    }
    guard envelope.audience == configuration.audience else {
      throw PermissionPersistenceError.wrongAudience
    }
    guard envelope.permissionKind == directoryPermissionKind else {
      throw PermissionPersistenceError.wrongKind
    }
    guard now < envelope.expiresAt else { throw PermissionPersistenceError.expired }
    guard envelope.rights & requestedRights == requestedRights,
      requestedQuota <= envelope.quota
    else {
      throw PermissionPersistenceError.escalation
    }
    let resolution: ResolvedPermissionScope
    do {
      resolution = try await configuration.resolveScopeLocator(envelope.scopeLocator)
    } catch {
      throw PermissionPersistenceError.scopeMismatch
    }
    guard resolution.stableIdentity == envelope.scopeIdentity else {
      throw PermissionPersistenceError.scopeMismatch
    }
    try await replay.reserve(envelope.nonce)
    do {
      let permission = try await adapter.importDirectoryPermission(
        locator: resolution.locator,
        rights: requestedRights,
        quota: requestedQuota,
        authenticatedIdentity: resolution.stableIdentity
      )
      await replay.commit(envelope.nonce)
      recordAudit(
        category: .permissionPersistence,
        outcome: .succeeded,
        operation: "permission.import"
      )
      return permission
    } catch {
      await replay.release(envelope.nonce)
      recordAudit(
        category: .permissionPersistence, outcome: .denied, operation: "permission.import")
      throw PermissionPersistenceError.scopeMismatch
    }
  }

  private static func randomNonce() -> Data {
    var uuid = UUID().uuid
    return withUnsafeBytes(of: &uuid) { Data($0) }
  }
}

private struct EnvelopeReader {
  init(_ data: Data) {
    self.data = data
  }

  mutating func read(count: Int) throws -> Data {
    guard count >= 0, offset <= data.count, count <= data.count - offset else {
      throw PermissionPersistenceError.invalidEnvelope
    }
    let value = data.subdata(in: offset..<(offset + count))
    offset += count
    return value
  }

  mutating func readInteger<T: FixedWidthInteger>(_ type: T.Type) throws -> T {
    let bytes = try read(count: MemoryLayout<T>.size)
    return bytes.reduce(T.zero) { ($0 << 8) | T($1) }
  }

  mutating func readSized(maximum: Int) throws -> Data {
    let length = try readInteger(UInt32.self)
    guard length <= UInt32(maximum) else { throw PermissionPersistenceError.invalidEnvelope }
    return try read(count: Int(length))
  }

  mutating func readText(maximum: Int) throws -> String {
    let bytes = try readSized(maximum: maximum)
    guard let text = String(data: bytes, encoding: .utf8) else {
      throw PermissionPersistenceError.invalidEnvelope
    }
    return text
  }

  var isFinished: Bool { offset == data.count }

  private let data: Data
  private var offset = 0
}

extension Data {
  fileprivate mutating func appendInteger<T: FixedWidthInteger>(_ value: T) {
    for shift in stride(from: T.bitWidth - 8, through: 0, by: -8) {
      append(UInt8(truncatingIfNeeded: value >> T(shift)))
    }
  }

  fileprivate mutating func appendSized(_ value: Data) throws {
    guard let count = UInt32(exactly: value.count) else {
      throw PermissionPersistenceError.invalidEnvelope
    }
    appendInteger(count)
    append(value)
  }
}
