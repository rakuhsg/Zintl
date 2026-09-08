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

Use `list!` to erase different `IntoElement` types into a reusable factory
vector. Each expression is evaluated once; rendering clones the captured value
and creates a fresh `Element`.

```rust,ignore
let children = list![
    Text::new("Name"),
    TextField::new(),
    Button::new("Save"),
];

Element::node(AppRenderNode::Form)
    .with_children(children.into_iter().map(IntoElement::into_element))
```

`list!` has no arity limit and returns a `Vec<ElementFactory<_>>`. Use
`ElementFactory::new` with `push` or another `list!` with `extend` when children
are assembled dynamically. Use keys when list identity must survive inserts,
removals, or reordering.

```rust,ignore
let mut rows = list![Header::new("Tasks")];
for task in &tasks {
    rows.push(ElementFactory::new(TaskRow::new(task).key(task.id)));
}

Element::node(AppRenderNode::List)
    .with_children(rows.into_iter().map(IntoElement::into_element))
```
