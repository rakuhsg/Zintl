import RuntimeEmbed
import RuntimeJSC

@main
struct EmbeddingLifecycleExample {
  static func main() async throws {
    let configuration = try EmbeddedRuntimeConfiguration(
      permissionTimeoutNanoseconds: 5_000_000_000,
      maximumAuditEvents: 256
    )
    let executor = DispatchQueueJavaScriptExecutor(
      label: "com.example.zintl.javascript"
    )
    let runtime = EmbeddedRuntime.build(
      configuration: configuration,
      executor: executor
    )
    let adapter = try await runtime.startJavaScriptCore(runtimeID: 1)
    let result = try await adapter.evaluate("Promise.resolve(40 + 2)")
    print(result.json)
    await runtime.shutdown()
  }
}
