import AppKit

@MainActor
public final class TabView: NSView {
    public struct Item: Identifiable, Equatable, Sendable {
        public let id: UUID
        public var title: String
        public var symbolName: String

        public init(id: UUID = UUID(), title: String, symbolName: String) {
            self.id = id
            self.title = title
            self.symbolName = symbolName
        }
    }

    public struct Metrics: Sendable {
        public var height: CGFloat
        public var minimumWidth: CGFloat
        public var maximumWidth: CGFloat?
        public var maximumHeight: CGFloat?
        public var spacing: CGFloat
        public var horizontalBackgroundPadding: CGFloat
        public var verticalBackgroundPadding: CGFloat
        public var glassOpticalInset: CGFloat
        public var closeButtonSize: CGFloat
        public var closeButtonTrailingPadding: CGFloat
        public var dragActivationDistance: CGFloat
        public var horizontalContentPadding: CGFloat
        public var verticalContentPadding: CGFloat
        public var closeKeyEquivalent: String
        public var closeKeyEquivalentModifierMask: NSEvent.ModifierFlags

        public init(
            height: CGFloat = 34,
            minimumWidth: CGFloat = 170,
            maximumWidth: CGFloat? = nil,
            maximumHeight: CGFloat? = nil,
            spacing: CGFloat = 2,
            horizontalBackgroundPadding: CGFloat = 4,
            verticalBackgroundPadding: CGFloat = 4,
            glassOpticalInset: CGFloat = 1,
            closeButtonSize: CGFloat = 20,
            closeButtonTrailingPadding: CGFloat = 10,
            dragActivationDistance: CGFloat = 8,
            horizontalContentPadding: CGFloat = 12,
            verticalContentPadding: CGFloat = 4,
            closeKeyEquivalent: String = "w",
            closeKeyEquivalentModifierMask: NSEvent.ModifierFlags = [.command]
        ) {
            self.height = height
            self.minimumWidth = minimumWidth
            self.maximumWidth = maximumWidth
            self.maximumHeight = maximumHeight
            self.spacing = spacing
            self.horizontalBackgroundPadding = horizontalBackgroundPadding
            self.verticalBackgroundPadding = verticalBackgroundPadding
            self.glassOpticalInset = glassOpticalInset
            self.closeButtonSize = closeButtonSize
            self.closeButtonTrailingPadding = closeButtonTrailingPadding
            self.dragActivationDistance = dragActivationDistance
            self.horizontalContentPadding = horizontalContentPadding
            self.verticalContentPadding = verticalContentPadding
            self.closeKeyEquivalent = closeKeyEquivalent
            self.closeKeyEquivalentModifierMask = closeKeyEquivalentModifierMask
        }

        var cornerRadius: CGFloat { height / 2 }
        var hoverHeight: CGFloat { height - (glassOpticalInset * 2) }
        var stripHeight: CGFloat { height + (verticalBackgroundPadding * 2) }

        func stripWidth(tabCount: Int) -> CGFloat {
            let tabsWidth = minimumWidth * CGFloat(max(tabCount, 1))
            let gapsWidth = spacing * CGFloat(max(tabCount - 1, 0))
            return tabsWidth + gapsWidth + (horizontalBackgroundPadding * 2)
        }
    }

    public private(set) var items: [Item]
    public private(set) var selectedItemID: UUID?
    public var selectedItem: Item? { items.first { $0.id == selectedItemID } }
    public private(set) var preferredSize: NSSize
    public private(set) var maximumSize = NSSize(
        width: CGFloat.greatestFiniteMagnitude,
        height: CGFloat.greatestFiniteMagnitude
    )
    public var onSelectionChanged: ((Item) -> Void)?
    public var onItemClosed: ((Item) -> Void)?
    public var onItemsChanged: (([Item]) -> Void)?

    private let metrics: Metrics
    private let stackView = NSStackView()
    private var tabButtons: [TabButton] = []
    private var maximumWidthConstraint: NSLayoutConstraint?
    private var maximumHeightConstraint: NSLayoutConstraint?

    public init(
        items: [Item],
        selectedItemID: UUID? = nil,
        metrics: Metrics = Metrics()
    ) {
        self.items = items
        self.metrics = metrics
        preferredSize = NSSize(
            width: metrics.stripWidth(tabCount: items.count),
            height: metrics.stripHeight
        )
        maximumSize = NSSize(
            width: max(
                metrics.maximumWidth ?? CGFloat.greatestFiniteMagnitude,
                metrics.horizontalBackgroundPadding * 2
            ),
            height: max(
                metrics.maximumHeight ?? CGFloat.greatestFiniteMagnitude,
                metrics.stripHeight
            )
        )
        self.selectedItemID = selectedItemID ?? items.first?.id
        super.init(frame: .zero)

        wantsLayer = true
        layer?.cornerCurve = .continuous

        stackView.orientation = .horizontal
        stackView.alignment = .centerY
        stackView.distribution = .fillEqually
        stackView.spacing = metrics.spacing
        stackView.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stackView)

        NSLayoutConstraint.activate([
            stackView.leadingAnchor.constraint(
                equalTo: leadingAnchor,
                constant: metrics.horizontalBackgroundPadding
            ),
            stackView.trailingAnchor.constraint(
                equalTo: trailingAnchor,
                constant: -metrics.horizontalBackgroundPadding
            ),
            stackView.topAnchor.constraint(
                equalTo: topAnchor,
                constant: metrics.verticalBackgroundPadding
            ),
            stackView.bottomAnchor.constraint(
                equalTo: bottomAnchor,
                constant: -metrics.verticalBackgroundPadding
            )
        ])
        updateMaximumSizeConstraints()

        tabButtons = items.enumerated().map(makeTabButton)
        for tab in tabButtons {
            stackView.addArrangedSubview(tab)
        }
        updateSelectionAppearance()
    }

    required init?(coder: NSCoder) {
        nil
    }

    public override var intrinsicContentSize: NSSize {
        NSSize(
            width: min(preferredSize.width, maximumSize.width),
            height: min(preferredSize.height, maximumSize.height)
        )
    }

    public override var wantsUpdateLayer: Bool { true }

    public override func layout() {
        super.layout()
        layer?.cornerRadius = bounds.height / 2
    }

    public override func updateLayer() {
        layer?.backgroundColor = NSColor.systemGray.withAlphaComponent(0.18).cgColor
    }

    public func selectItem(id: UUID) {
        guard items.contains(where: { $0.id == id }) else { return }
        selectedItemID = id
        updateSelectionAppearance()
        if let selectedItem {
            onSelectionChanged?(selectedItem)
        }
    }

    public func removeItem(id: UUID) {
        guard let tab = tabButtons.first(where: { $0.item.id == id }) else { return }
        removeTab(tab)
    }

    @discardableResult
    public func appendItem(_ item: Item, selecting: Bool = true) -> Bool {
        insertItem(item, at: items.count, selecting: selecting)
    }

    @discardableResult
    public func insertItem(_ item: Item, at index: Int, selecting: Bool = true) -> Bool {
        guard
            !items.contains(where: { $0.id == item.id }),
            (0...items.count).contains(index)
        else { return false }

        let tab = makeTabButton(index: index, item: item)
        items.insert(item, at: index)
        tabButtons.insert(tab, at: index)
        stackView.insertArrangedSubview(tab, at: index)
        updateTabShortcuts()

        let shouldSelect = selecting || selectedItemID == nil
        if shouldSelect {
            selectedItemID = item.id
            updateSelectionAppearance()
        }

        onItemsChanged?(items)
        if shouldSelect {
            onSelectionChanged?(item)
        }
        return true
    }

    public func setPreferredSize(width: CGFloat? = nil, height: CGFloat? = nil) {
        let minimumOuterWidth = metrics.horizontalBackgroundPadding * 2
        preferredSize = NSSize(
            width: max(width ?? preferredSize.width, minimumOuterWidth),
            height: max(height ?? preferredSize.height, metrics.stripHeight)
        )
        invalidateIntrinsicContentSize()
    }

    public func resetPreferredSize() {
        preferredSize = NSSize(
            width: metrics.stripWidth(tabCount: items.count),
            height: metrics.stripHeight
        )
        invalidateIntrinsicContentSize()
    }

    public func setMaximumSize(width: CGFloat? = nil, height: CGFloat? = nil) {
        let minimumOuterWidth = metrics.horizontalBackgroundPadding * 2
        maximumSize = NSSize(
            width: max(width ?? maximumSize.width, minimumOuterWidth),
            height: max(height ?? maximumSize.height, metrics.stripHeight)
        )
        updateMaximumSizeConstraints()
        invalidateIntrinsicContentSize()
    }

    public func resetMaximumSize() {
        maximumSize = NSSize(
            width: CGFloat.greatestFiniteMagnitude,
            height: CGFloat.greatestFiniteMagnitude
        )
        updateMaximumSizeConstraints()
        invalidateIntrinsicContentSize()
    }

    private func updateMaximumSizeConstraints() {
        maximumWidthConstraint?.isActive = false
        maximumHeightConstraint?.isActive = false

        if maximumSize.width < CGFloat.greatestFiniteMagnitude {
            let constraint = widthAnchor.constraint(lessThanOrEqualToConstant: maximumSize.width)
            constraint.isActive = true
            maximumWidthConstraint = constraint
        } else {
            maximumWidthConstraint = nil
        }

        if maximumSize.height < CGFloat.greatestFiniteMagnitude {
            let constraint = heightAnchor.constraint(lessThanOrEqualToConstant: maximumSize.height)
            constraint.isActive = true
            maximumHeightConstraint = constraint
        } else {
            maximumHeightConstraint = nil
        }
    }

    private func makeTabButton(index: Int, item: Item) -> TabButton {
        let tab = TabButton(
            item: item,
            shortcut: index + 1,
            metrics: metrics,
            target: self,
            selectAction: #selector(selectTab(_:)),
            closeAction: #selector(closeTab(_:))
        )
        tab.translatesAutoresizingMaskIntoConstraints = false
        let minimumWidthConstraint = tab.widthAnchor.constraint(
            greaterThanOrEqualToConstant: metrics.minimumWidth
        )
        minimumWidthConstraint.priority = .defaultHigh
        minimumWidthConstraint.isActive = true
        tab.heightAnchor.constraint(equalToConstant: metrics.height).isActive = true
        tab.onDragChanged = { [weak self] draggedTab, locationX in
            self?.reorderTab(draggedTab, locationX: locationX)
        }
        return tab
    }

    @objc
    private func selectTab(_ sender: NSButton) {
        guard let tab = tabButtons.first(where: { $0.button === sender }) else { return }
        selectItem(id: tab.item.id)
    }

    @objc
    private func closeTab(_ sender: NSButton) {
        guard let tab = sender.superview as? TabButton else { return }
        removeTab(tab)
    }

    private func removeTab(_ tab: TabButton) {
        guard let closedIndex = tabButtons.firstIndex(where: { $0 === tab }) else { return }
        let closedItem = tab.item
        let wasActive = tab.item.id == selectedItemID

        tabButtons.remove(at: closedIndex)
        items.removeAll { $0.id == tab.item.id }
        stackView.removeArrangedSubview(tab)
        tab.removeFromSuperview()
        updateTabShortcuts()
        resetPreferredSize()
        onItemClosed?(closedItem)

        if items.isEmpty {
            selectedItemID = nil
        } else if wasActive {
            selectedItemID = items[min(closedIndex, items.count - 1)].id
            updateSelectionAppearance()
            if let selectedItem {
                onSelectionChanged?(selectedItem)
            }
        }
        onItemsChanged?(items)
    }

    private func updateSelectionAppearance() {
        for tab in tabButtons {
            tab.isTabSelected = tab.item.id == selectedItemID
        }
    }

    private func reorderTab(_ draggedTab: TabButton, locationX: CGFloat) {
        guard let currentIndex = tabButtons.firstIndex(where: { $0 === draggedTab }) else { return }

        let destinationIndex: Int
        if currentIndex > 0, locationX < tabButtons[currentIndex - 1].frame.midX {
            destinationIndex = currentIndex - 1
        } else if
            currentIndex < tabButtons.count - 1,
            locationX > tabButtons[currentIndex + 1].frame.midX
        {
            destinationIndex = currentIndex + 1
        } else {
            return
        }

        tabButtons.remove(at: currentIndex)
        tabButtons.insert(draggedTab, at: destinationIndex)
        let item = items.remove(at: currentIndex)
        items.insert(item, at: destinationIndex)
        stackView.removeArrangedSubview(draggedTab)
        stackView.insertArrangedSubview(draggedTab, at: destinationIndex)
        stackView.layoutSubtreeIfNeeded()
        updateTabShortcuts()
        onItemsChanged?(items)
    }

    private func updateTabShortcuts() {
        for (index, tab) in tabButtons.enumerated() {
            tab.button.keyEquivalent = String(index + 1)
        }
    }
}

@MainActor
private final class TabButton: NSView {
    let item: TabView.Item
    let button: NSButton
    var onDragChanged: ((TabButton, CGFloat) -> Void)?

    var isTabSelected = false {
        didSet {
            activeBackground.isHidden = !isTabSelected
            button.contentTintColor = isTabSelected ? .labelColor : .secondaryLabelColor
            updateCloseButtonVisibility()
            updateHoverBackground()
        }
    }

    private let metrics: TabView.Metrics
    private let activeBackground: NSView
    private let closeButton = NSButton()
    private let hoverBackgroundLayer = CALayer()
    private var isHovered = false {
        didSet {
            updateCloseButtonVisibility()
            updateHoverBackground()
        }
    }
    private var hoverTrackingArea: NSTrackingArea?

    init(
        item: TabView.Item,
        shortcut: Int,
        metrics: TabView.Metrics,
        target: AnyObject?,
        selectAction: Selector,
        closeAction: Selector
    ) {
        self.item = item
        self.metrics = metrics
        button = NSButton(title: item.title, target: target, action: selectAction)

        if #available(macOS 26.0, *) {
            let glassView = NSGlassEffectView()
            glassView.cornerRadius = metrics.cornerRadius
            glassView.style = .regular
            activeBackground = glassView
        } else {
            let effectView = NSVisualEffectView()
            effectView.material = .selection
            effectView.blendingMode = .withinWindow
            effectView.state = .active
            effectView.wantsLayer = true
            effectView.layer?.cornerRadius = metrics.cornerRadius
            effectView.layer?.cornerCurve = .continuous
            activeBackground = effectView
        }

        super.init(frame: .zero)
        wantsLayer = true
        layer?.cornerRadius = metrics.cornerRadius
        layer?.cornerCurve = .continuous
        hoverBackgroundLayer.cornerCurve = .continuous
        layer?.addSublayer(hoverBackgroundLayer)

        activeBackground.isHidden = true
        activeBackground.translatesAutoresizingMaskIntoConstraints = false

        button.cell = PaddedButtonCell(
            title: item.title,
            horizontalPadding: metrics.horizontalContentPadding,
            verticalPadding: metrics.verticalContentPadding
        )
        button.target = target
        button.action = selectAction
        button.setButtonType(.momentaryPushIn)
        button.controlSize = .large
        button.font = .systemFont(ofSize: 13, weight: .medium)
        button.image = NSImage(systemSymbolName: item.symbolName, accessibilityDescription: nil)
        button.imagePosition = .imageLeading
        button.imageHugsTitle = true
        button.keyEquivalent = String(shortcut)
        button.keyEquivalentModifierMask = [.command]
        button.isBordered = false
        button.contentTintColor = .secondaryLabelColor
        button.translatesAutoresizingMaskIntoConstraints = false

        closeButton.target = target
        closeButton.action = closeAction
        closeButton.image = NSImage(
            systemSymbolName: "xmark.circle.fill",
            accessibilityDescription: "Close tab"
        )
        closeButton.imagePosition = .imageOnly
        closeButton.isBordered = false
        closeButton.contentTintColor = .secondaryLabelColor
        closeButton.keyEquivalent = ""
        closeButton.keyEquivalentModifierMask = metrics.closeKeyEquivalentModifierMask
        closeButton.toolTip = "Close \(item.title)"
        closeButton.setAccessibilityLabel("Close \(item.title)")
        closeButton.isHidden = true
        closeButton.translatesAutoresizingMaskIntoConstraints = false

        addSubview(activeBackground)
        addSubview(button)
        addSubview(closeButton)

        let dragGesture = ThresholdDragGestureRecognizer(
            threshold: metrics.dragActivationDistance,
            target: self,
            action: #selector(handleDrag(_:))
        )
        dragGesture.delaysPrimaryMouseButtonEvents = true
        addGestureRecognizer(dragGesture)

        NSLayoutConstraint.activate([
            activeBackground.leadingAnchor.constraint(equalTo: leadingAnchor),
            activeBackground.trailingAnchor.constraint(equalTo: trailingAnchor),
            activeBackground.topAnchor.constraint(equalTo: topAnchor),
            activeBackground.bottomAnchor.constraint(equalTo: bottomAnchor),
            button.leadingAnchor.constraint(equalTo: leadingAnchor),
            button.trailingAnchor.constraint(equalTo: trailingAnchor),
            button.topAnchor.constraint(equalTo: topAnchor),
            button.bottomAnchor.constraint(equalTo: bottomAnchor),
            closeButton.trailingAnchor.constraint(
                equalTo: trailingAnchor,
                constant: -metrics.closeButtonTrailingPadding
            ),
            closeButton.centerYAnchor.constraint(equalTo: centerYAnchor),
            closeButton.widthAnchor.constraint(equalToConstant: metrics.closeButtonSize),
            closeButton.heightAnchor.constraint(equalToConstant: metrics.closeButtonSize)
        ])
    }

    required init?(coder: NSCoder) {
        nil
    }

    @objc
    private func handleDrag(_ recognizer: ThresholdDragGestureRecognizer) {
        guard let stackView = superview else { return }

        switch recognizer.state {
        case .began:
            layer?.zPosition = 10
            alphaValue = 0.88
        case .changed:
            let locationX = recognizer.currentLocation(in: stackView).x
            onDragChanged?(self, locationX)
            stackView.layoutSubtreeIfNeeded()
            let updatedLocationX = recognizer.currentLocation(in: stackView).x
            layer?.setAffineTransform(
                CGAffineTransform(translationX: updatedLocationX - frame.midX, y: 0)
            )
        case .ended, .cancelled, .failed:
            layer?.setAffineTransform(.identity)
            layer?.zPosition = 0
            alphaValue = 1
        default:
            break
        }
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let hoverTrackingArea {
            removeTrackingArea(hoverTrackingArea)
        }
        let trackingArea = NSTrackingArea(
            rect: .zero,
            options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect],
            owner: self,
            userInfo: nil
        )
        addTrackingArea(trackingArea)
        hoverTrackingArea = trackingArea
    }

    override func layout() {
        super.layout()
        hoverBackgroundLayer.frame = bounds.insetBy(dx: 0, dy: metrics.glassOpticalInset)
        hoverBackgroundLayer.cornerRadius = metrics.hoverHeight / 2
        updateCloseButtonVisibility()
    }

    override func mouseEntered(with event: NSEvent) {
        super.mouseEntered(with: event)
        isHovered = true
    }

    override func mouseExited(with event: NSEvent) {
        super.mouseExited(with: event)
        isHovered = false
    }

    private func updateHoverBackground() {
        let color: NSColor = !isTabSelected && isHovered
            ? NSColor.systemGray.withAlphaComponent(0.28)
            : .clear
        hoverBackgroundLayer.backgroundColor = color.cgColor
    }

    private func updateCloseButtonVisibility() {
        let canShowInactiveCloseButton = bounds.width >= metrics.closeButtonSize * 3.5
        closeButton.isHidden = !isTabSelected && (!isHovered || !canShowInactiveCloseButton)
        closeButton.keyEquivalent = isTabSelected ? metrics.closeKeyEquivalent : ""
    }
}

@MainActor
private final class PaddedButtonCell: NSButtonCell {
    private let horizontalPadding: CGFloat
    private let verticalPadding: CGFloat

    init(title: String, horizontalPadding: CGFloat, verticalPadding: CGFloat) {
        self.horizontalPadding = horizontalPadding
        self.verticalPadding = verticalPadding
        super.init(textCell: title)
    }

    required init(coder: NSCoder) {
        horizontalPadding = 0
        verticalPadding = 0
        super.init(coder: coder)
    }

    override func drawInterior(withFrame cellFrame: NSRect, in controlView: NSView) {
        let contentFrame = cellFrame.insetBy(
            dx: min(horizontalPadding, cellFrame.width / 2),
            dy: min(verticalPadding, cellFrame.height / 2)
        )
        super.drawInterior(withFrame: contentFrame, in: controlView)
    }
}

@MainActor
private final class ThresholdDragGestureRecognizer: NSGestureRecognizer {
    private let threshold: CGFloat
    private var initialLocationInWindow: NSPoint?
    private var currentLocationInWindow = NSPoint.zero

    init(threshold: CGFloat, target: AnyObject?, action: Selector?) {
        self.threshold = threshold
        super.init(target: target, action: action)
    }

    required init?(coder: NSCoder) {
        nil
    }

    override func mouseDown(with event: NSEvent) {
        super.mouseDown(with: event)
        initialLocationInWindow = event.locationInWindow
        currentLocationInWindow = event.locationInWindow
    }

    override func mouseDragged(with event: NSEvent) {
        super.mouseDragged(with: event)
        currentLocationInWindow = event.locationInWindow
        guard let initialLocationInWindow else {
            state = .failed
            return
        }
        let horizontalDistance = abs(currentLocationInWindow.x - initialLocationInWindow.x)
        if state == .possible, horizontalDistance >= threshold {
            state = .began
        } else if state == .began || state == .changed {
            state = .changed
        }
    }

    override func mouseUp(with event: NSEvent) {
        super.mouseUp(with: event)
        currentLocationInWindow = event.locationInWindow
        if state == .possible {
            state = .failed
        } else if state == .began || state == .changed {
            state = .ended
        }
    }

    override func reset() {
        super.reset()
        initialLocationInWindow = nil
        currentLocationInWindow = .zero
    }

    func currentLocation(in view: NSView) -> NSPoint {
        view.convert(currentLocationInWindow, from: nil)
    }
}
