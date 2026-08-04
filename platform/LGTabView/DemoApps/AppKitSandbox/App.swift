import AppKit
import LiquidGlassTabs

@main
@MainActor
struct AppKitSandboxApp {
    static func main() {
        let application = NSApplication.shared
        let delegate = AppDelegate()

        application.setActivationPolicy(.regular)
        application.delegate = delegate
        application.run()
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var window: NSWindow?
    private var tabView: TabView?
    private var nextTabNumber = 1

    func applicationDidFinishLaunching(_ notification: Notification) {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 720, height: 480),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "AppKit Sandbox"
        window.center()

        let items = [
            TabView.Item(title: "Start Page", symbolName: "sparkles"),
            TabView.Item(title: "Projects", symbolName: "folder"),
            TabView.Item(title: "Downloads", symbolName: "arrow.down.circle"),
            TabView.Item(title: "Settings", symbolName: "gearshape")
        ]
        let tabView = TabView(
            items: items,
            metrics: .init(
                height: 32,
                horizontalBackgroundPadding: 2,
                verticalBackgroundPadding: 2,
                horizontalContentPadding: 16,
                verticalContentPadding: 2,
                closeKeyEquivalentModifierMask: [.command]
            )
        )
        tabView.setPreferredSize(width: window.contentLayoutRect.width)
        tabView.translatesAutoresizingMaskIntoConstraints = false
        tabView.onSelectionChanged = { [weak window] item in
            window?.title = "\(item.title) — AppKit Sandbox"
        }
        tabView.onItemClosed = { item in
            print("\(item.title) closed")
        }

        let addTabButton = NSButton(
            image: NSImage(systemSymbolName: "plus", accessibilityDescription: "Add tab")!,
            target: self,
            action: #selector(addTab(_:))
        )
        addTabButton.imagePosition = .imageOnly
        addTabButton.toolTip = "Add Tab"
        addTabButton.setAccessibilityLabel("Add Tab")
        addTabButton.translatesAutoresizingMaskIntoConstraints = false
        if #available(macOS 26.0, *) {
            addTabButton.bezelStyle = .glass
        } else {
            addTabButton.bezelStyle = .circular
        }

        let contentView = NSView()
        contentView.addSubview(tabView)
        contentView.addSubview(addTabButton)
        NSLayoutConstraint.activate([
            tabView.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            tabView.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
            tabView.centerYAnchor.constraint(equalTo: contentView.centerYAnchor, constant: -24),
            addTabButton.topAnchor.constraint(equalTo: tabView.bottomAnchor, constant: 12),
            addTabButton.centerXAnchor.constraint(equalTo: contentView.centerXAnchor),
            addTabButton.widthAnchor.constraint(equalToConstant: 32),
            addTabButton.heightAnchor.constraint(equalToConstant: 32)
        ])

        window.contentView = contentView
        window.makeKeyAndOrderFront(nil)
        if let selectedItem = tabView.selectedItem {
            window.title = "\(selectedItem.title) — AppKit Sandbox"
        }
        self.window = window
        self.tabView = tabView

        NSApp.activate(ignoringOtherApps: true)
    }

    @objc
    private func addTab(_ sender: NSButton) {
        let number = nextTabNumber
        nextTabNumber += 1
        tabView?.appendItem(
            TabView.Item(title: "New Tab \(number)", symbolName: "doc"),
            selecting: true
        )
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }
}
