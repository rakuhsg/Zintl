import AppKit
import PermissionReplDemoSupport
import RuntimeJSC

@main
struct PermissionReplDemoApp {
  @MainActor
  static func main() {
    let application = NSApplication.shared
    application.setActivationPolicy(.regular)
    let delegate = AppDelegate()
    application.delegate = delegate
    application.run()
    _ = delegate
  }
}

@MainActor
private final class AppDelegate: NSObject, NSApplicationDelegate {
  func applicationDidFinishLaunching(_ notification: Notification) {
    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 920, height: 720),
      styleMask: [.titled, .closable, .miniaturizable, .resizable],
      backing: .buffered,
      defer: false
    )
    window.title = "Zintl Permission REPL Demo"
    window.center()
    let dialog = PermissionDialogCoordinator { [weak window] in window }
    do {
      let host = try DemoRuntimeHost(dialog: dialog)
      let controller = ReplViewController(host: host, dialog: dialog)
      window.contentViewController = controller
      window.delegate = controller
      self.window = window
      self.controller = controller
      window.makeKeyAndOrderFront(nil)
      NSApp.activate(ignoringOtherApps: true)
      Task { await controller.startRuntime() }
    } catch {
      NSApp.presentError(error)
      NSApp.terminate(nil)
    }
  }

  func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }

  func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
    guard !terminationPending else { return .terminateLater }
    guard let controller else { return .terminateNow }
    terminationPending = true
    Task {
      await controller.shutdownForTermination()
      sender.reply(toApplicationShouldTerminate: true)
    }
    return .terminateLater
  }

  private var window: NSWindow?
  private var controller: ReplViewController?
  private var terminationPending = false
}

@MainActor
private final class ReplViewController: NSViewController, NSWindowDelegate {
  init(host: DemoRuntimeHost, dialog: PermissionDialogCoordinator) {
    self.host = host
    self.dialog = dialog
    super.init(nibName: nil, bundle: nil)
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }

  override func loadView() {
    input.font = .monospacedSystemFont(ofSize: 13, weight: .regular)
    output.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
    output.isEditable = false
    input.string = """
      // Replace /tmp/zintl-demo with a directory you created.
      Zintl.requestDirectory('/tmp/zintl-demo', { read: true })
        .then(async dir => {
          const file = await dir.openRelative('sample.txt', { read: true });
          try { return Array.from(await file.read({ maxBytes: 65536 })); }
          finally { await file.close(); }
        })
      """
    let run = button("Run", action: #selector(runScript))
    let clear = button("Clear History", action: #selector(clearHistory))
    let cancel = button("Cancel", action: #selector(cancelExecution))
    let shutdown = button("Shutdown", action: #selector(shutdownRuntime))
    let recreate = button("Recreate", action: #selector(recreateRuntime))
    let custom = button("Custom Op Sample", action: #selector(loadCustomSample))
    let controls = NSStackView(views: [run, clear, cancel, shutdown, recreate, custom, status])
    controls.orientation = .horizontal
    controls.spacing = 8
    let inputScroll = scrollView(for: input)
    let outputScroll = scrollView(for: output)
    let stack = NSStackView(views: [
      label("JavaScript"), inputScroll, controls, label("History"), outputScroll,
    ])
    stack.orientation = .vertical
    stack.spacing = 8
    stack.translatesAutoresizingMaskIntoConstraints = false
    let root = NSView()
    root.addSubview(stack)
    NSLayoutConstraint.activate([
      stack.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 16),
      stack.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -16),
      stack.topAnchor.constraint(equalTo: root.topAnchor, constant: 16),
      stack.bottomAnchor.constraint(equalTo: root.bottomAnchor, constant: -16),
      inputScroll.heightAnchor.constraint(equalToConstant: 250),
    ])
    view = root
  }

  func startRuntime() async {
    status.stringValue = "Starting…"
    do { status.stringValue = statusText(try await host.start()) } catch { append(error: error) }
  }

  @objc private func runScript() {
    guard execution == nil else { return }
    status.stringValue = "Pending…"
    let source = input.string
    execution = Task { [weak self] in
      guard let self else { return }
      do {
        let result = try await host.evaluate(source)
        append(line: result.json)
        status.stringValue = "Ready"
      } catch {
        append(error: error)
        status.stringValue = "Failed"
      }
      execution = nil
    }
  }

  @objc private func cancelExecution() {
    execution?.cancel()
    execution = nil
    status.stringValue = "Cancelling and recreating…"
    Task {
      do { status.stringValue = statusText(try await host.cancelAndRecreate()) } catch {
        append(error: error)
      }
    }
  }

  @objc private func shutdownRuntime() {
    execution?.cancel()
    execution = nil
    Task {
      await host.shutdown()
      status.stringValue = "Shutdown"
    }
  }

  @objc private func recreateRuntime() {
    execution?.cancel()
    execution = nil
    status.stringValue = "Recreating…"
    Task {
      do { status.stringValue = statusText(try await host.recreate()) } catch {
        append(error: error)
      }
    }
  }

  @objc private func clearHistory() { output.string = "" }

  @objc private func loadCustomSample() {
    input.string = """
      Zintl.invoke('dev.zintl.demo.reverse', new Uint8Array([90, 105, 110, 116, 108]))
        .then(bytes => Array.from(bytes))
      """
  }

  func windowWillClose(_ notification: Notification) {
    execution?.cancel()
    dialog.cancelAll()
  }

  func shutdownForTermination() async {
    execution?.cancel()
    execution = nil
    dialog.cancelAll()
    await host.shutdown()
  }

  private func append(error: Error) {
    let value: [String: String]
    if let exception = error as? JavaScriptException {
      value = [
        "type": "error", "name": exception.name, "code": exception.code,
        "message": exception.message,
      ]
    } else {
      value = [
        "type": "error", "name": "HostError", "code": "Internal",
        "message": String(describing: error),
      ]
    }
    let data = try? JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
    append(line: data.map { String(decoding: $0, as: UTF8.self) } ?? #"{"type":"error"}"#)
  }

  private func append(line: String) {
    output.string += (output.string.isEmpty ? "" : "\n") + line
    output.scrollToEndOfDocument(nil)
  }

  private func statusText(_ value: DemoRuntimeStatus) -> String {
    switch value {
    case .alreadyRunning: "Runtime already running"
    case .readyWithoutSavedPermission: "Runtime ready; no saved permission"
    case .readyWithImportedPermission:
      "Runtime ready; saved read-only permission imported as savedDirectory"
    case .readyWithInvalidSavedPermission:
      "Runtime ready; saved permission invalid or expired—request it again"
    }
  }

  private func button(_ title: String, action: Selector) -> NSButton {
    let button = NSButton(title: title, target: self, action: action)
    button.bezelStyle = .rounded
    return button
  }

  private func label(_ value: String) -> NSTextField {
    NSTextField(labelWithString: value)
  }

  private func scrollView(for textView: NSTextView) -> NSScrollView {
    let scroll = NSScrollView()
    scroll.hasVerticalScroller = true
    scroll.borderType = .bezelBorder
    scroll.documentView = textView
    return scroll
  }

  private let host: DemoRuntimeHost
  private let dialog: PermissionDialogCoordinator
  private let input = NSTextView()
  private let output = NSTextView()
  private let status = NSTextField(labelWithString: "Not started")
  private var execution: Task<Void, Never>?
}
