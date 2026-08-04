import AppKit
import LiquidGlassTabs

@main
@MainActor
struct ComplexApp {
    static func main() {
        let application = NSApplication.shared
        let delegate = AppDelegate()

        application.setActivationPolicy(.regular)
        application.delegate = delegate
        application.run()
    }
}

@MainActor
private final class AppDelegate: NSObject, NSApplicationDelegate, NSToolbarDelegate {
    private var window: NSWindow?
    private var splitView: NSSplitView?
    private var toolbarTabView: TabView?
    private var nextTabNumber = 1

    private static let toolbarIdentifier = NSToolbar.Identifier("MainToolbar")
    private static let sidebarSeparatorIdentifier = NSToolbarItem.Identifier("SidebarSeparator")
    private static let tabsItemIdentifier = NSToolbarItem.Identifier("ToolbarTabs")
    private static let addTabItemIdentifier = NSToolbarItem.Identifier("AddToolbarTab")

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.mainMenu = makeMainMenu()

        let splitViewController = NSSplitViewController()

        let sidebarController = SidebarViewController()
        let sidebarItem = NSSplitViewItem(sidebarWithViewController: sidebarController)
        sidebarItem.minimumThickness = 180
        sidebarItem.maximumThickness = 320
        sidebarItem.canCollapse = true
        sidebarItem.allowsFullHeightLayout = true

        let contentController = ContentViewController()
        let contentItem = NSSplitViewItem(viewController: contentController)
        contentItem.minimumThickness = 360

        sidebarController.onSelectionChanged = { [weak contentController] item in
            contentController?.show(item: item)
        }

        splitViewController.addSplitViewItem(sidebarItem)
        splitViewController.addSplitViewItem(contentItem)

        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 900, height: 600),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        window.title = "Complex App"
        window.titlebarAppearsTransparent = true
        window.contentViewController = splitViewController
        splitView = splitViewController.splitView

        let toolbar = NSToolbar(identifier: Self.toolbarIdentifier)
        toolbar.delegate = self
        toolbar.displayMode = .iconOnly
        toolbar.centeredItemIdentifiers = [Self.tabsItemIdentifier]
        window.toolbar = toolbar
        window.toolbarStyle = .unified
        window.center()
        window.makeKeyAndOrderFront(nil)

        self.window = window
        NSApp.activate(ignoringOtherApps: true)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        [
            .toggleSidebar,
            Self.sidebarSeparatorIdentifier,
            .flexibleSpace,
            Self.tabsItemIdentifier,
            .flexibleSpace,
            Self.addTabItemIdentifier
        ]
    }

    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        [
            .toggleSidebar,
            Self.sidebarSeparatorIdentifier,
            Self.tabsItemIdentifier,
            Self.addTabItemIdentifier,
            .flexibleSpace,
            .space
        ]
    }

    func toolbar(
        _ toolbar: NSToolbar,
        itemForItemIdentifier itemIdentifier: NSToolbarItem.Identifier,
        willBeInsertedIntoToolbar flag: Bool
    ) -> NSToolbarItem? {
        if itemIdentifier == Self.sidebarSeparatorIdentifier {
            guard let splitView else { return nil }
            return NSTrackingSeparatorToolbarItem(
                identifier: itemIdentifier,
                splitView: splitView,
                dividerIndex: 0
            )
        }

        if itemIdentifier == Self.addTabItemIdentifier {
            let toolbarItem = NSToolbarItem(itemIdentifier: itemIdentifier)
            toolbarItem.label = "Add Tab"
            toolbarItem.paletteLabel = "Add Tab"
            toolbarItem.toolTip = "Add Tab"
            toolbarItem.image = NSImage(
                systemSymbolName: "plus",
                accessibilityDescription: "Add Tab"
            )
            toolbarItem.target = self
            toolbarItem.action = #selector(addToolbarTab(_:))
            return toolbarItem
        }

        guard itemIdentifier == Self.tabsItemIdentifier else { return nil }

        let tabView = TabView(
            items: [
                TabView.Item(title: "Overview", symbolName: "rectangle.grid.2x2"),
                TabView.Item(title: "Activity", symbolName: "waveform.path.ecg"),
                TabView.Item(title: "Details", symbolName: "list.bullet.rectangle")
            ],
            metrics: .init(
                height: 32,
                minimumWidth: 300,
                maximumWidth: 1010,
                horizontalBackgroundPadding: 2,
                verticalBackgroundPadding: 2,
                horizontalContentPadding: 10,
                verticalContentPadding: 2
            )
        )
        tabView.onSelectionChanged = { [weak window] item in
            window?.title = "\(item.title) — Complex App"
        }
        toolbarTabView = tabView

        let toolbarItem = NSToolbarItem(itemIdentifier: itemIdentifier)
        toolbarItem.label = "Sections"
        toolbarItem.paletteLabel = "Sections"
        toolbarItem.toolTip = "Switch Section"
        toolbarItem.view = tabView
        return toolbarItem
    }

    @objc
    private func addToolbarTab(_ sender: Any?) {
        let number = nextTabNumber
        nextTabNumber += 1
        toolbarTabView?.appendItem(
            TabView.Item(title: "New Tab \(number)", symbolName: "doc"),
            selecting: true
        )
        toolbarTabView?.resetPreferredSize()
    }

    private func makeMainMenu() -> NSMenu {
        let mainMenu = NSMenu()

        let applicationMenuItem = NSMenuItem()
        let applicationMenu = NSMenu()
        let quitItem = NSMenuItem(
            title: "Quit Complex App",
            action: #selector(NSApplication.terminate(_:)),
            keyEquivalent: "q"
        )
        quitItem.target = NSApp
        applicationMenu.addItem(quitItem)
        applicationMenuItem.submenu = applicationMenu
        mainMenu.addItem(applicationMenuItem)

        let fileMenuItem = NSMenuItem()
        let fileMenu = NSMenu(title: "File")
        let newTabItem = NSMenuItem(
            title: "New Tab",
            action: #selector(addToolbarTab(_:)),
            keyEquivalent: "t"
        )
        newTabItem.keyEquivalentModifierMask = [.command]
        newTabItem.target = self
        fileMenu.addItem(newTabItem)
        fileMenuItem.submenu = fileMenu
        mainMenu.addItem(fileMenuItem)

        return mainMenu
    }
}

@MainActor
private final class SidebarViewController: NSViewController, NSTableViewDataSource, NSTableViewDelegate {
    struct Item {
        let title: String
        let symbolName: String
    }

    var onSelectionChanged: ((Item) -> Void)?

    private let items = [
        Item(title: "Home", symbolName: "house"),
        Item(title: "Documents", symbolName: "doc"),
        Item(title: "Settings", symbolName: "gearshape")
    ]
    private let tableView = NSTableView()

    override func loadView() {
        let scrollView = NSScrollView()
        scrollView.drawsBackground = false
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true

        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("SidebarColumn"))
        column.resizingMask = .autoresizingMask
        tableView.addTableColumn(column)
        tableView.headerView = nil
        tableView.backgroundColor = .clear
        tableView.rowHeight = 32
        tableView.style = .sourceList
        tableView.dataSource = self
        tableView.delegate = self
        scrollView.documentView = tableView

        view = scrollView
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        guard tableView.selectedRow == -1, !items.isEmpty else { return }
        tableView.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
        onSelectionChanged?(items[0])
    }

    func numberOfRows(in tableView: NSTableView) -> Int {
        items.count
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let item = items[row]
        let cell = NSTableCellView()

        let imageView = NSImageView()
        imageView.image = NSImage(systemSymbolName: item.symbolName, accessibilityDescription: item.title)
        imageView.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 15, weight: .regular)
        imageView.contentTintColor = .labelColor
        imageView.translatesAutoresizingMaskIntoConstraints = false

        let textField = NSTextField(labelWithString: item.title)
        textField.lineBreakMode = .byTruncatingTail
        textField.translatesAutoresizingMaskIntoConstraints = false

        cell.addSubview(imageView)
        cell.addSubview(textField)
        NSLayoutConstraint.activate([
            imageView.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 4),
            imageView.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
            imageView.widthAnchor.constraint(equalToConstant: 18),
            imageView.heightAnchor.constraint(equalToConstant: 18),
            textField.leadingAnchor.constraint(equalTo: imageView.trailingAnchor, constant: 8),
            textField.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -4),
            textField.centerYAnchor.constraint(equalTo: cell.centerYAnchor)
        ])

        return cell
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        guard items.indices.contains(tableView.selectedRow) else { return }
        onSelectionChanged?(items[tableView.selectedRow])
    }
}

@MainActor
private final class ContentViewController: NSViewController {
    private let titleLabel = NSTextField(labelWithString: "Home")

    override func loadView() {
        let contentView = NSView()
        titleLabel.font = .systemFont(ofSize: 28, weight: .semibold)
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(titleLabel)

        NSLayoutConstraint.activate([
            titleLabel.centerXAnchor.constraint(equalTo: contentView.centerXAnchor),
            titleLabel.centerYAnchor.constraint(equalTo: contentView.centerYAnchor)
        ])

        view = contentView
    }

    func show(item: SidebarViewController.Item) {
        titleLabel.stringValue = item.title
    }
}
