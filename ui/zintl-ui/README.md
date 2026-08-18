# zintl-ui

UI primitives for Zintl.

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
```

Use keys for dynamic lists.

```rust,ignore
let rows = tasks
    .iter()
    .map(|task| TaskRow::new(task).key(task.id).into_element());
Element::node(AppRenderNode::List).with_children(rows)
```
