# LiquidGlassTabs

`LiquidGlassTabs` is a reusable AppKit tab strip for macOS applications. It
provides Safari-style horizontal tabs with a Liquid Glass selection appearance
on macOS 26 and a standard rounded appearance on earlier supported versions.

The library targets macOS 13 or later and is distributed as a Swift Package
Manager library product.

## Features

- Tab selection, insertion, removal, and drag reordering
- SF Symbol icons and configurable tab titles
- Close buttons for the selected and hovered tabs
- Configurable sizing, spacing, padding, and keyboard shortcut
- Intrinsic-content sizing with optional maximum width and height
- Selection, close, and item-list change callbacks
- Liquid Glass styling on macOS 26 with an automatic fallback

The component intentionally leaves application-specific controls, such as an
add-tab button and its keyboard shortcut, to the host application.

## Installation

Add the package dependency to `Package.swift`:

```swift
dependencies: [
    .package(
        url: "https://github.com/rakuhsg/swift-tab.git",
        branch: "main"
    )
]
```

Then add `LiquidGlassTabs` to the dependencies of the application target:

```swift
.target(
    name: "MyApp",
    dependencies: [
        .product(name: "LiquidGlassTabs", package: "swift-tab")
    ]
)
```

## Usage

The `LiquidGlassTabs` product exposes `TabView`, `TabView.Item`, and configurable
`TabView.Metrics`:

```swift
import LiquidGlassTabs

let tabs = TabView(items: [
    .init(title: "Home", symbolName: "house"),
    .init(title: "Settings", symbolName: "gearshape")
], metrics: .init(
    height: 40,
    horizontalBackgroundPadding: 8,
    verticalBackgroundPadding: 6,
    horizontalContentPadding: 16,
    verticalContentPadding: 6
)) // The default close shortcut remains Command-W.

tabs.onSelectionChanged = { item in
    print(item.title)
}

tabs.onItemClosed = { item in
    print("Closed: \(item.title)")
}
```

### Adding tabs

Tabs are added through data APIs; the library intentionally does not provide an
add button or an add-tab keyboard shortcut. The sample application demonstrates
how an app can provide its own add button and call this API.

Append and select a new tab:

```swift
let item = TabView.Item(title: "New Tab", symbolName: "doc")
tabs.appendItem(item)
```

Append without changing the current selection:

```swift
tabs.appendItem(item, selecting: false)
```

Insert at a specific zero-based position:

```swift
tabs.insertItem(item, at: 1, selecting: true)
```

Both methods return `false` when the item ID is already present or the insertion
index is invalid. Successful insertion triggers `onItemsChanged`; it also
triggers `onSelectionChanged` when the inserted tab becomes selected. Adding the
first tab always selects it. When no tabs remain, the tab strip keeps one
`minimumWidth` slot instead of collapsing to zero width.

### Tab height

The tab height defaults to `34` points and can be changed with
`TabView.Metrics.height`:

```swift
let tabs = TabView(
    items: items,
    metrics: .init(height: 40)
)
```

The tab corner radius, strip height, active Liquid Glass background, and
inactive hover background are calculated from this height automatically.

### Background padding

The padding between the outer gray background and the tab buttons can be
configured independently in the horizontal and vertical directions:

```swift
let tabs = TabView(
    items: items,
    metrics: .init(
        horizontalBackgroundPadding: 8,
        verticalBackgroundPadding: 6
    )
)
```

Both background-padding values default to `4` points. The tab strip's intrinsic
width and height are recalculated automatically from these values.

### View size

`TabView` exposes its current `preferredSize` and provides an API for changing
its intrinsic width or height at runtime:

```swift
tabs.setPreferredSize(width: 800)
tabs.setPreferredSize(width: 800, height: 60)
```

Omitted dimensions keep their current value. The height is clamped so it cannot
become smaller than the configured tab height plus background padding. Restore
the content-derived size with:

```swift
tabs.resetPreferredSize()
```

The maximum size can be configured when creating the tab view:

```swift
let tabs = TabView(
    items: items,
    metrics: .init(maximumWidth: 500, maximumHeight: 60)
)
```

It can also be changed at runtime. Omitted dimensions keep their current
maximum:

```swift
tabs.setMaximumSize(width: 500)
tabs.setMaximumSize(height: 60)
```

The maximum height cannot be smaller than the configured tab height plus its
background padding. Remove both limits with:

```swift
tabs.resetMaximumSize()
```

When the content-derived preferred size exceeds a maximum, the tabs compress
equally. The limit is applied to both the intrinsic size and the view's Auto
Layout constraints, including when the view is hosted by an `NSToolbarItem`.

To keep a `TabView` the same width as its window during resizing, constrain its
leading and trailing edges to the window's content view:

```swift
tabs.translatesAutoresizingMaskIntoConstraints = false
NSLayoutConstraint.activate([
    tabs.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
    tabs.trailingAnchor.constraint(equalTo: contentView.trailingAnchor)
])
```

### Content padding

The padding around each tab's icon and title can be configured independently in
the horizontal and vertical directions:

```swift
let tabs = TabView(
    items: items,
    metrics: .init(
        horizontalContentPadding: 16,
        verticalContentPadding: 6
    )
)
```

The defaults are `12` horizontal points and `4` vertical points. Padding only
changes the content drawing area, so the entire tab—including its padding—stays
clickable.

### Close shortcut

The active-tab close shortcut defaults to `Command-W`. It can be changed through
`TabView.Metrics` by specifying both the key equivalent and modifier mask:

```swift
let tabs = TabView(
    items: items,
    metrics: .init(
        closeKeyEquivalent: "k",
        closeKeyEquivalentModifierMask: [.command, .shift]
    )
) // Closes the active tab with Command-Shift-K.
```

Set `closeKeyEquivalent` to an empty string to disable the shortcut:

```swift
let tabs = TabView(
    items: items,
    metrics: .init(closeKeyEquivalent: "")
)
```

## Demo applications

The repository includes two applications demonstrating library integration.
They are separate from the library source under `DemoApps/`:

```sh
swift run AppKitSandbox
swift run ComplexApp
```

`AppKitSandbox` shows a minimal window integration. `ComplexApp` demonstrates a
tab strip embedded in an `NSToolbar` alongside a sidebar and content area. The
library implementation is contained in
`Library/Sources/LiquidGlassTabs/TabView.swift`.
