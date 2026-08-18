# zintl-ui

`zintl-ui` is a renderer-independent reactive UI core. A renderer supplies a
`RenderNode` description and a `RenderBackend` that mounts those descriptions
to retained native objects.

Views form reactive `Bound` boundaries. Stores read while rendering a view are
recorded as that boundary's dependencies. Updating a store marks only its
dependent boundaries dirty; `Composer::flush` rebuilds those boundaries,
reconciles their cached element subtrees, and applies backend operations.

```rust,ignore
struct Counter {
    count: Option<Store<i32>>,
}

impl View for Counter {
    type Output = AppRenderNode;

    fn init(&mut self, cx: &mut Context<'_>) {
        self.count = Some(cx.store(0));
    }

    fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
        Text::new(cx.get(self.count.unwrap()).to_string())
    }
}

let mut composer = Composer::new(app_backend);
composer.mount(Counter { count: None });

// Event handlers receive a Context and update the registered Store through it.
```

Children are matched by position and element kind by default. Dynamic lists
should provide keys so their bound and native identities survive insertion and
reordering.

```rust,ignore
let rows = tasks
    .iter()
    .map(|task| TaskRow::new(task).key(task.id).into_element());
Element::node(AppRenderNode::List).with_children(rows)
```

See `zintl-ui-test` for a complete in-memory render node and backend used by the
unit tests.
