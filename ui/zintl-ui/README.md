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
    count: Store<i32>,
}

impl View for Counter {
    type Output = AppRenderNode;

    fn render(&mut self, cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
        Text::new(cx.get(self.count).to_string())
    }
}

let mut composer = Composer::new(app_backend);
let count = composer.context(|cx| cx.store(0));
composer.mount(Counter { count });

composer.context(|cx| cx.update(count, |value| *value += 1));
composer.flush();
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
