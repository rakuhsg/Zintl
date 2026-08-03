import AppKit
import ZintlAppkitSupportTypes

struct ZintlSidebarConfiguration: Decodable {
  var sections: [ZintlSidebarSectionConfiguration]
  var selectedId: String?
}

struct ZintlSidebarSectionConfiguration: Decodable {
  var title: String?
  var items: [ZintlSidebarItemConfiguration]
}

struct ZintlSidebarItemConfiguration: Decodable {
  var id: String
  var title: String
  var systemImage: String?
}

@MainActor
private final class ZintlSidebarSectionNode: NSObject {
  let title: String
  let items: [ZintlSidebarItemNode]

  init(title: String, items: [ZintlSidebarItemNode]) {
    self.title = title
    self.items = items
  }
}

@MainActor
private final class ZintlSidebarItemNode: NSObject {
  let id: String
  let title: String
  let systemImage: String?

  init(configuration: ZintlSidebarItemConfiguration) {
    self.id = configuration.id
    self.title = configuration.title
    self.systemImage = configuration.systemImage
  }
}

@MainActor
private final class ZintlSidebarRowView: NSTableRowView {
  override var isEmphasized: Bool {
    get { false }
    set {}
  }
}

@MainActor
final class ZintlSidebarViewController: NSViewController,
  NSOutlineViewDataSource, NSOutlineViewDelegate
{
  // Callback state is fixed after initialization. Swift 6 deinits are
  // nonisolated, so releasing the Rust allocation needs unchecked access.
  nonisolated(unsafe) private var userData: UnsafeRawPointer?
  nonisolated(unsafe) private var selectionCallback: ZintlSidebarSelectionCallback?
  nonisolated(unsafe) private var releaseCallback: ZintlSidebarRelease?

  private let rootNodes: [NSObject]
  private let selectedId: String?
  private let outlineView = NSOutlineView()

  init(
    configuration: ZintlSidebarConfiguration,
    userData: UnsafeRawPointer?,
    selectionCallback: ZintlSidebarSelectionCallback?,
    releaseCallback: ZintlSidebarRelease?
  ) {
    var nodes: [NSObject] = []
    for section in configuration.sections {
      let items = section.items.map(ZintlSidebarItemNode.init)
      if let title = section.title {
        nodes.append(ZintlSidebarSectionNode(title: title, items: items))
      } else {
        nodes.append(contentsOf: items)
      }
    }
    self.rootNodes = nodes
    self.selectedId = configuration.selectedId
    self.userData = userData
    self.selectionCallback = selectionCallback
    self.releaseCallback = releaseCallback
    super.init(nibName: nil, bundle: nil)
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) is unavailable")
  }

  deinit {
    self.releaseUserData()
  }

  override func loadView() {
    let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("sidebar"))
    column.resizingMask = .autoresizingMask
    self.outlineView.addTableColumn(column)
    self.outlineView.outlineTableColumn = column
    self.outlineView.headerView = nil
    if #available(macOS 11.0, *) {
      self.outlineView.style = .sourceList
    } else {
      self.outlineView.selectionHighlightStyle = .sourceList
    }
    self.outlineView.rowSizeStyle = .medium
    self.outlineView.backgroundColor = .clear
    self.outlineView.dataSource = self
    self.outlineView.delegate = self

    let scrollView = NSScrollView()
    scrollView.documentView = self.outlineView
    scrollView.hasVerticalScroller = true
    scrollView.drawsBackground = false
    self.view = scrollView
  }

  override func viewDidLoad() {
    super.viewDidLoad()
    for node in self.rootNodes where node is ZintlSidebarSectionNode {
      self.outlineView.expandItem(node)
    }
    guard let selectedId else {
      return
    }
    for row in 0..<self.outlineView.numberOfRows {
      guard let item = self.outlineView.item(atRow: row) as? ZintlSidebarItemNode,
        item.id == selectedId
      else {
        continue
      }
      self.outlineView.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
      break
    }
  }

  nonisolated func releaseUserData() {
    let release = self.releaseCallback
    self.releaseCallback = nil
    let userData = self.userData
    self.userData = nil
    self.selectionCallback = nil
    release?(userData)
  }

  func outlineView(_ outlineView: NSOutlineView, numberOfChildrenOfItem item: Any?) -> Int {
    if item == nil {
      return self.rootNodes.count
    }
    return (item as? ZintlSidebarSectionNode)?.items.count ?? 0
  }

  func outlineView(_ outlineView: NSOutlineView, child index: Int, ofItem item: Any?) -> Any {
    if let section = item as? ZintlSidebarSectionNode {
      return section.items[index]
    }
    return self.rootNodes[index]
  }

  func outlineView(_ outlineView: NSOutlineView, isItemExpandable item: Any) -> Bool {
    item is ZintlSidebarSectionNode
  }

  func outlineView(_ outlineView: NSOutlineView, isGroupItem item: Any) -> Bool {
    item is ZintlSidebarSectionNode
  }

  func outlineView(
    _ outlineView: NSOutlineView,
    shouldSelectItem item: Any
  ) -> Bool {
    item is ZintlSidebarItemNode
  }

  func outlineView(
    _ outlineView: NSOutlineView,
    rowViewForItem item: Any
  ) -> NSTableRowView? {
    guard item is ZintlSidebarItemNode else {
      return nil
    }
    return ZintlSidebarRowView()
  }

  func outlineView(
    _ outlineView: NSOutlineView,
    viewFor tableColumn: NSTableColumn?,
    item: Any
  ) -> NSView? {
    if let section = item as? ZintlSidebarSectionNode {
      return self.makeCell(identifier: "sidebar-group", title: section.title, systemImage: nil)
    }
    guard let item = item as? ZintlSidebarItemNode else {
      return nil
    }
    return self.makeCell(
      identifier: item.systemImage == nil ? "sidebar-item" : "sidebar-item-image",
      title: item.title,
      systemImage: item.systemImage
    )
  }

  func outlineViewSelectionDidChange(_ notification: Notification) {
    let row = self.outlineView.selectedRow
    guard row >= 0,
      let item = self.outlineView.item(atRow: row) as? ZintlSidebarItemNode,
      let callback = self.selectionCallback
    else {
      return
    }
    withZintlString(item.id) { itemId in
      callback(self.userData, itemId)
    }
  }

  private func makeCell(
    identifier: String,
    title: String,
    systemImage: String?
  ) -> NSTableCellView {
    let identifier = NSUserInterfaceItemIdentifier(identifier)
    if let cell = self.outlineView.makeView(withIdentifier: identifier, owner: self)
      as? NSTableCellView
    {
      cell.textField?.stringValue = title
      if #available(macOS 11.0, *), let systemImage {
        cell.imageView?.image = NSImage(
          systemSymbolName: systemImage,
          accessibilityDescription: title
        )
      }
      return cell
    }

    let cell = NSTableCellView()
    cell.identifier = identifier
    let label = NSTextField(labelWithString: title)
    label.lineBreakMode = .byTruncatingTail
    label.translatesAutoresizingMaskIntoConstraints = false
    cell.textField = label
    cell.addSubview(label)

    if #available(macOS 11.0, *), let systemImage {
      let image = NSImageView()
      image.image = NSImage(systemSymbolName: systemImage, accessibilityDescription: title)
      image.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 15, weight: .regular)
      image.translatesAutoresizingMaskIntoConstraints = false
      cell.imageView = image
      cell.addSubview(image)
      NSLayoutConstraint.activate([
        image.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 2),
        image.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
        image.widthAnchor.constraint(equalToConstant: 18),
        image.heightAnchor.constraint(equalToConstant: 18),
        label.leadingAnchor.constraint(equalTo: image.trailingAnchor, constant: 6),
      ])
    } else {
      label.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 2).isActive = true
    }
    NSLayoutConstraint.activate([
      label.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -4),
      label.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
    ])
    return cell
  }
}

@MainActor
func zintlSidebarConfiguration(_ json: ZintlString) throws -> ZintlSidebarConfiguration {
  let string = zintlString(json)
  guard let data = string.data(using: .utf8) else {
    throw CocoaError(.fileReadInapplicableStringEncoding)
  }
  return try JSONDecoder().decode(ZintlSidebarConfiguration.self, from: data)
}
