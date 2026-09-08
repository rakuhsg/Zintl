# zintl-ui-desktop

Declarative desktop views. Native sidebar support is currently AppKit-only.

Attach navigation to a window with `Window::sidebar`:

```rust,ignore
// Create this Store in View::init and retain it on the view.
self.selection = cx.store(Some("home".to_owned()));

// Build this window in View::render.
Window::new(Rect::new(100.0, 100.0, 640.0, 400.0), "Navigation")
    .sidebar(
        Sidebar::new([SidebarSection::new([
            SidebarItem::new("home", "Home").system_image("house"),
            SidebarItem::new("settings", "Settings").system_image("gearshape"),
        ]).title("Pages")])
        .bind(self.selection)
        .on_select(|_cx, id| println!("Selected: {id}")),
    )
    .content(cx.watch(self.selection, |selected| {
        Text::new(selected.as_deref().unwrap_or("No selection"))
    }))
```

## Child lists

Use `list!` for heterogeneous stack children. The macro returns a
`ListFactory<RenderNode>`, an alias for `Vec<ElementFactory<RenderNode>>`, so the
number of children is not limited by a tuple arity.

```rust,ignore
VStack::new(list![
    Text::new("Profile"),
    TextField::new().placeholder("Name"),
    TextField::new().placeholder("Email"),
    Text::new("Preferences"),
    Button::new("Import"),
    Button::new("Export"),
    Button::new("Reset"),
    Button::new("Save"),
])
```

The returned vector can also be extended at runtime.

```rust,ignore
let mut actions = list![Button::new("Save")];
if can_delete {
    actions.extend(list![Button::new("Delete")]);
}
actions.extend(list![Text::new("Ready"), Button::new("Close")]);

HStack::new(actions)
```

AppKit uses a scrollable `NSTableView` with source-list styling and 32-point rows.
Section titles are nonselectable group rows; item cells show a symbol and a
truncating label. The sidebar width is limited to 180–320 points.

`on_select` receives the selected item ID after the bound Store is updated. It
also works without `bind`. Initial/programmatic selection and deselection do not
invoke it; native item selection changes do.

Item IDs must be unique across the sidebar. `bind` reads and updates a
`Store<Option<String>>`; programmatic changes also update the native selection.
AppKit automatically adds a standard sidebar toggle to the window toolbar.
Removing the sidebar also removes the toolbar item it added; preexisting toolbar
items are preserved. Collapse state survives reactive updates.

Unknown selection events are ignored. Rebuilding a window without `.sidebar(...)`
removes its sidebar. SF Symbol names are interpreted by AppKit.

## Crate responsibilities

- `zintl-ui-desktop` owns the cross-platform builders and wraps platform views.
- `zintl-ui-appkit` owns AppKit-shaped views, render nodes, Store bindings, and events.
- `zintl-ui-appkit-backend` translates render nodes and synchronizes native state.
- `zpd-appkit` owns AppKit controllers, native lifetime, and native events.
- `zintl-ui` and `zintl-ui-layout` provide composition and layout primitives.

The non-macOS tree backend retains sidebar declarations for inspection; it does
not provide a native sidebar implementation.

See `examples/app.rs` for a complete application.
