import CryptoKit
import Foundation
import RuntimeEmbed

public actor DemoPermissionCodec: PermissionCodec {
  public init(keyURL: URL) throws {
    self.keyURL = keyURL
    if FileManager.default.fileExists(atPath: keyURL.path) {
      key = SymmetricKey(data: try Data(contentsOf: keyURL))
    } else {
      let newKey = SymmetricKey(size: .bits256)
      let data = newKey.withUnsafeBytes { Data($0) }
      try FileManager.default.createDirectory(
        at: keyURL.deletingLastPathComponent(),
        withIntermediateDirectories: true
      )
      try data.write(to: keyURL, options: .atomic)
      try FileManager.default.setAttributes(
        [.posixPermissions: 0o600],
        ofItemAtPath: keyURL.path
      )
      key = newKey
    }
  }

  public func seal(authenticatedEnvelope: Data) async throws -> Data {
    let box = try AES.GCM.seal(authenticatedEnvelope, using: key)
    guard let combined = box.combined else { throw DemoPersistenceError.codec }
    return combined
  }

  public func open(opaqueBlob: Data) async throws -> Data {
    try AES.GCM.open(AES.GCM.SealedBox(combined: opaqueBlob), using: key)
  }

  private let keyURL: URL
  private let key: SymmetricKey
}

public enum DemoPersistenceError: Error, Equatable {
  case codec
  case staleBookmark
  case invalidDirectoryIdentity
}

public enum DemoScopeLocator {
  public static func encode(path: String) throws -> Data {
    try URL(fileURLWithPath: path, isDirectory: true).bookmarkData(
      options: [.withSecurityScope],
      includingResourceValuesForKeys: nil,
      relativeTo: nil
    )
  }

  public static func resolve(bookmark: Data) throws -> ResolvedPermissionScope {
    var stale = false
    let url = try URL(
      resolvingBookmarkData: bookmark,
      options: [.withSecurityScope],
      relativeTo: nil,
      bookmarkDataIsStale: &stale
    )
    guard !stale else { throw DemoPersistenceError.staleBookmark }
    return ResolvedPermissionScope(
      locator: url.path,
      stableIdentity: try directoryIdentity(at: url)
    )
  }

  public static func directoryIdentity(at url: URL) throws -> Data {
    let attributes = try FileManager.default.attributesOfItem(atPath: url.path)
    guard let device = (attributes[.systemNumber] as? NSNumber)?.uint64Value,
      let file = (attributes[.systemFileNumber] as? NSNumber)?.uint64Value,
      device != 0, file != 0
    else {
      throw DemoPersistenceError.invalidDirectoryIdentity
    }
    var output = Data()
    output.appendBigEndian(device)
    output.appendBigEndian(file)
    return output
  }
}

extension Data {
  fileprivate mutating func appendBigEndian(_ value: UInt64) {
    for shift in stride(from: 56, through: 0, by: -8) {
      append(UInt8(truncatingIfNeeded: value >> UInt64(shift)))
    }
  }
}
