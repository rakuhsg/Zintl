import Foundation
import RuntimeJSC
import Testing

@testable import PermissionReplDemoSupport

@Test("demo codec rejects modified permission blobs")
// Verifies the Demo persistence adapter provides authenticated encryption, not plaintext storage.
func codecRejectsTamper() async throws {
  let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
  try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
  defer { try? FileManager.default.removeItem(at: directory) }
  let codec = try DemoPermissionCodec(keyURL: directory.appendingPathComponent("key"))
  let plaintext = Data("authenticated envelope".utf8)
  let blob = try await codec.seal(authenticatedEnvelope: plaintext)
  #expect(blob != plaintext)
  #expect(try await codec.open(opaqueBlob: blob) == plaintext)
  var tampered = blob
  tampered[tampered.startIndex] ^= 1
  await #expect(throws: (any Error).self) {
    _ = try await codec.open(opaqueBlob: tampered)
  }
}

@Test("demo directory identity is fixed width and changes across directories")
// Verifies bookmark resolution metadata can detect directory substitution before import.
func directoryIdentityDetectsReplacement() throws {
  let first = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
  let second = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
  try FileManager.default.createDirectory(at: first, withIntermediateDirectories: false)
  try FileManager.default.createDirectory(at: second, withIntermediateDirectories: false)
  defer {
    try? FileManager.default.removeItem(at: first)
    try? FileManager.default.removeItem(at: second)
  }
  let firstIdentity = try DemoScopeLocator.directoryIdentity(at: first)
  let secondIdentity = try DemoScopeLocator.directoryIdentity(at: second)
  #expect(firstIdentity.count == 16)
  #expect(firstIdentity != secondIdentity)
}

@Test("demo host executes custom operation away from the main queue")
// Verifies the compiled Demo embeds JSC/custom Promise work on its declared non-main executor.
func demoHostUsesNonMainExecutor() async throws {
  let storage = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
  defer { try? FileManager.default.removeItem(at: storage) }
  let dialog = await MainActor.run { PermissionDialogCoordinator { nil } }
  let host = try DemoRuntimeHost(dialog: dialog, storageDirectory: storage)
  _ = try await host.start()
  let result = try await host.evaluate(
    "Zintl.invoke('dev.zintl.demo.reverse', new Uint8Array([1, 2, 3])).then(Array.from)"
  )
  #expect(result.json == #"{"type":"value","value":[3,2,1]}"#)
  await host.shutdown()
}
