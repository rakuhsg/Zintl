import AppKit
import RuntimeEmbed

@MainActor
public final class PermissionDialogCoordinator: @unchecked Sendable {
  public init(window: @escaping @MainActor @Sendable () -> NSWindow?) {
    self.window = window
  }

  public func resolve(_ request: PermissionRequest) async -> PermissionDecision {
    if let approved = oneShotReopen,
      approved.requestedDirectory == request.requestedDirectory,
      approved.requestedRights == request.requestedRights,
      approved.requestedScope == request.requestedScope
    {
      oneShotReopen = nil
      return allow(request)
    }
    guard let parent = window(), let directory = request.requestedDirectory else {
      return .deny
    }
    return await withTaskCancellationHandler {
      await withCheckedContinuation { continuation in
        let alert = makeAlert(request: request, directory: directory)
        pending[request.requestID] = Pending(alert: alert, continuation: continuation)
        alert.beginSheetModal(for: parent) { [weak self] response in
          guard let self else { return }
          let decision: PermissionDecision
          switch response {
          case .alertSecondButtonReturn:
            decision = self.allow(request)
          case .alertThirdButtonReturn:
            self.saveRequests.append(request)
            decision = self.allow(request)
          default:
            decision = .deny
          }
          self.finish(requestID: request.requestID, decision: decision)
        }
      }
    } onCancel: {
      Task { @MainActor [weak self] in
        self?.cancel(requestID: request.requestID)
      }
    }
  }

  public func consumeSaveRequest() -> PermissionRequest? {
    guard !saveRequests.isEmpty else { return nil }
    let request = saveRequests.removeFirst()
    oneShotReopen = request
    return request
  }

  public func cancelAll() {
    for requestID in Array(pending.keys) {
      cancel(requestID: requestID)
    }
    saveRequests.removeAll()
    oneShotReopen = nil
  }

  private func cancel(requestID: UInt64) {
    guard let item = pending[requestID] else { return }
    item.alert.window.sheetParent?.endSheet(item.alert.window, returnCode: .abort)
    finish(requestID: requestID, decision: .deny)
  }

  private func finish(requestID: UInt64, decision: PermissionDecision) {
    guard let item = pending.removeValue(forKey: requestID) else { return }
    item.continuation.resume(returning: decision)
  }

  private func allow(_ request: PermissionRequest) -> PermissionDecision {
    .allow(
      scope: request.requestedScope,
      rights: request.requestedRights,
      quota: 4 * 1_024 * 1_024
    )
  }

  private func makeAlert(request: PermissionRequest, directory: String) -> NSAlert {
    let alert = NSAlert()
    alert.alertStyle = .warning
    alert.messageText = "JavaScript requests directory access"
    alert.informativeText = """
      Origin: \(request.operation)
      Rights: \(rightsDescription(request.requestedRights))
      The exact directory below is the complete requested scope.
      """
    alert.addButton(withTitle: "Deny")
    alert.addButton(withTitle: "Allow Once")
    alert.addButton(withTitle: "Save and Allow")
    let field = NSTextField(wrappingLabelWithString: directory)
    field.isSelectable = true
    field.frame = NSRect(x: 0, y: 0, width: 560, height: 72)
    alert.accessoryView = field
    return alert
  }

  private func rightsDescription(_ rights: UInt64) -> String {
    let names: [(UInt64, String)] = [
      (1, "read"), (2, "write"), (4, "create"), (8, "metadata"),
      (16, "enumerate"), (32, "truncate"),
    ]
    return names.compactMap { rights & $0.0 == 0 ? nil : $0.1 }.joined(separator: ", ")
  }

  private struct Pending {
    let alert: NSAlert
    let continuation: CheckedContinuation<PermissionDecision, Never>
  }

  private let window: @MainActor @Sendable () -> NSWindow?
  private var pending: [UInt64: Pending] = [:]
  private var saveRequests: [PermissionRequest] = []
  private var oneShotReopen: PermissionRequest?
}
