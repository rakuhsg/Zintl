//! AppKit renderer for Zintl UI trees.

use zintl_ui::renderer::RenderNode;
use zintl_ui_layout::LayoutStyle;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ViewKind {
    Container,
    Label(String),
    Button(String),
    TextField {
        value: String,
        placeholder: Option<String>,
        on_change: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    Window {
        bounds: Rect,
        title: String,
        id: Option<String>,
    },
    View {
        kind: ViewKind,
        layout: LayoutStyle,
        id: Option<String>,
    },
}

pub trait AppKitRenderNode: RenderNode {
    fn appkit_node(&self) -> NodeKind;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(usize);

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    TextChanged { node: NodeId, value: String },
}

#[cfg(target_os = "macos")]
mod backend {
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};
    use std::error::Error;
    use std::fmt;
    use std::rc::Rc;

    use messageloop_appkit::{
        AppkitSender, Context as MessageContext, MessageLoopAppkit, MessageLoopHandler, Sender,
    };
    use zintl_ui::composer::Composer;
    use zintl_ui::renderer::RenderBackend;
    use zintl_ui_layout::{LayoutError, LayoutStyle, LayoutTree, Size};
    use zpd_appkit::geometry::Rect as NativeRect;
    use zpd_appkit::runloop::{Application, ApplicationError, RunLoopSourceError};
    use zpd_appkit::ui::{
        AsView, Button as NativeButton, CommandError, CommandItem, CommandModifier, CommandRole,
        CommandSet, TextField as NativeTextField, View as NativeView, ViewError, ViewRef,
        Window as NativeWindow, WindowAppMenu, WindowError,
    };

    use super::{AppKitRenderNode, Event, NodeId, NodeKind, Rect, ViewKind};

    #[derive(Debug)]
    pub enum AppError {
        Application(ApplicationError),
        Command(CommandError),
        Window(WindowError),
        View(ViewError),
        Layout(LayoutError),
        RunLoopSource(RunLoopSourceError),
        NoWindow,
        InvalidTree,
    }

    impl fmt::Display for AppError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Application(error) => error.fmt(formatter),
                Self::Command(error) => error.fmt(formatter),
                Self::Window(error) => error.fmt(formatter),
                Self::View(error) => error.fmt(formatter),
                Self::Layout(error) => error.fmt(formatter),
                Self::RunLoopSource(error) => error.fmt(formatter),
                Self::NoWindow => {
                    formatter.write_str("the rendered tree does not contain a window")
                }
                Self::InvalidTree => formatter.write_str("a window cannot be nested in a view"),
            }
        }
    }

    impl Error for AppError {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            match self {
                Self::Application(error) => Some(error),
                Self::Command(error) => Some(error),
                Self::Window(error) => Some(error),
                Self::View(error) => Some(error),
                Self::Layout(error) => Some(error),
                Self::RunLoopSource(error) => Some(error),
                Self::NoWindow | Self::InvalidTree => None,
            }
        }
    }

    enum NativeNode {
        Container(NativeView),
        Label(NativeTextField),
        Button(NativeButton),
        TextField(NativeTextField),
    }

    impl NativeNode {
        fn as_view(&self) -> ViewRef<'_> {
            match self {
                Self::Container(view) => view.as_view(),
                Self::Label(view) | Self::TextField(view) => view.as_view(),
                Self::Button(view) => view.as_view(),
            }
        }
    }

    struct BackendNode<R> {
        value: Option<R>,
        parent: Option<NodeId>,
        children: Vec<NodeId>,
        native: Option<NativeNode>,
    }

    pub struct AppKitBackend<R: AppKitRenderNode> {
        nodes: Vec<Option<BackendNode<R>>>,
        sender: Option<AppkitSender<Message>>,
        structure_dirty: bool,
        layout_dirty: bool,
    }

    impl<R: AppKitRenderNode> Default for AppKitBackend<R> {
        fn default() -> Self {
            Self::new()
        }
    }

    impl<R: AppKitRenderNode> AppKitBackend<R> {
        pub fn new() -> Self {
            Self {
                nodes: vec![Some(BackendNode {
                    value: None,
                    parent: None,
                    children: Vec::new(),
                    native: None,
                })],
                sender: None,
                structure_dirty: true,
                layout_dirty: true,
            }
        }

        pub fn value(&self, id: NodeId) -> Option<&R> {
            self.nodes
                .get(id.0)
                .and_then(Option::as_ref)
                .and_then(|node| node.value.as_ref())
        }

        pub fn children(&self, id: NodeId) -> &[NodeId] {
            &self.node(id).children
        }

        fn node(&self, id: NodeId) -> &BackendNode<R> {
            self.nodes
                .get(id.0)
                .and_then(Option::as_ref)
                .expect("node handle must belong to this AppKit backend")
        }

        fn node_mut(&mut self, id: NodeId) -> &mut BackendNode<R> {
            self.nodes
                .get_mut(id.0)
                .and_then(Option::as_mut)
                .expect("node handle must belong to this AppKit backend")
        }

        fn detach(&mut self, child: NodeId) {
            let Some(parent) = self.node(child).parent else {
                return;
            };
            self.node_mut(parent)
                .children
                .retain(|candidate| *candidate != child);
            self.node_mut(child).parent = None;
        }

        fn window_ids(&self) -> Vec<NodeId> {
            self.children(NodeId(0))
                .iter()
                .copied()
                .filter(|id| {
                    matches!(
                        self.value(*id).map(AppKitRenderNode::appkit_node),
                        Some(NodeKind::Window { .. })
                    )
                })
                .collect()
        }

        fn synchronize<'application>(
            &mut self,
            application: &'application Application<()>,
            windows: &mut HashMap<NodeId, NativeWindow<'application, ()>>,
            sender: AppkitSender<Message>,
        ) -> Result<(), AppError> {
            self.sender = Some(sender);
            let window_ids = self.window_ids();
            if window_ids.is_empty() {
                return Err(AppError::NoWindow);
            }

            let live: HashSet<_> = window_ids.iter().copied().collect();
            windows.retain(|id, _| live.contains(id));
            let rebuild_structure = self.structure_dirty;
            let update_layout = self.layout_dirty || rebuild_structure;

            for window_id in window_ids {
                let description = self
                    .value(window_id)
                    .expect("window nodes have values")
                    .appkit_node();
                let NodeKind::Window { bounds, title, id } = description else {
                    continue;
                };

                let new_window = !windows.contains_key(&window_id);
                if new_window {
                    let window = application.create_window(()).map_err(AppError::Window)?;
                    window
                        .set_bounds(native_rect(bounds))
                        .map_err(AppError::Window)?;
                    windows.insert(window_id, window);
                }
                let window = windows
                    .get(&window_id)
                    .expect("a synchronized window must exist");
                window.set_title(&title).map_err(AppError::Window)?;
                window
                    .set_identifier(id.as_deref())
                    .map_err(AppError::Window)?;

                self.materialize_children(application, window_id)?;
                if rebuild_structure {
                    let content = window.content_view().map_err(AppError::Window)?;
                    self.attach_children(content, window_id);
                }
                if update_layout {
                    self.layout_window(window_id, bounds)?;
                }

                if new_window {
                    window.show().map_err(AppError::Window)?;
                }
            }
            self.structure_dirty = false;
            self.layout_dirty = false;
            Ok(())
        }

        fn materialize_children(
            &mut self,
            application: &Application<()>,
            parent: NodeId,
        ) -> Result<(), AppError> {
            let children = self.children(parent).to_vec();
            for child in children {
                if self.node(child).native.is_none() {
                    let description = self
                        .value(child)
                        .expect("render nodes have values")
                        .appkit_node();
                    let native = self.create_native(application, child, &description)?;
                    self.node_mut(child).native = native;
                }
                self.materialize_children(application, child)?;
            }
            Ok(())
        }

        fn create_native(
            &self,
            application: &Application<()>,
            id: NodeId,
            description: &NodeKind,
        ) -> Result<Option<NativeNode>, AppError> {
            let NodeKind::View {
                kind,
                id: accessibility_id,
                ..
            } = description
            else {
                return Ok(None);
            };
            let native = match kind {
                ViewKind::Container => NativeNode::Container(
                    NativeView::new(application, NativeRect::new(0.0, 0.0, 0.0, 0.0))
                        .map_err(AppError::View)?,
                ),
                ViewKind::Label(content) => NativeNode::Label(
                    NativeTextField::label_with_string(application, content)
                        .map_err(AppError::View)?,
                ),
                ViewKind::Button(title) => NativeNode::Button(
                    NativeButton::with_title(application, title).map_err(AppError::View)?,
                ),
                ViewKind::TextField {
                    value,
                    placeholder,
                    on_change,
                } => {
                    let field =
                        NativeTextField::with_string(application, value).map_err(AppError::View)?;
                    field.set_placeholder_string(placeholder.as_deref());
                    if *on_change {
                        self.install_change_handler(&field, id);
                    }
                    NativeNode::TextField(field)
                }
            };
            native.as_view().set_identifier(accessibility_id.as_deref());
            Ok(Some(native))
        }

        fn install_change_handler(&self, field: &NativeTextField, id: NodeId) {
            let sender = self
                .sender
                .as_ref()
                .expect("the AppKit event sender is installed before native views")
                .clone();
            field.set_change_handler(move |value| {
                let _ = sender.send(Message::Event(Event::TextChanged { node: id, value }));
            });
        }

        fn attach_children(&self, parent: ViewRef<'_>, parent_id: NodeId) {
            for child in self.children(parent_id) {
                let Some(native) = self.node(*child).native.as_ref() else {
                    continue;
                };
                native.as_view().remove_from_superview();
                parent.add_subview(&native.as_view());
                self.attach_children(native.as_view(), *child);
            }
        }

        fn layout_window(&self, window_id: NodeId, bounds: Rect) -> Result<(), AppError> {
            let mut layout = LayoutTree::new();
            let built = self
                .children(window_id)
                .iter()
                .map(|child| self.build_layout(&mut layout, *child))
                .collect::<Result<Vec<_>, _>>()?;
            let root_children = built.iter().map(|node| node.layout).collect::<Vec<_>>();
            let root = layout
                .create_node(
                    LayoutStyle::stack(zintl_ui_layout::Axis::Vertical, 0.0),
                    &root_children,
                )
                .map_err(AppError::Layout)?;
            let available = Size::new(bounds.width as f32, bounds.height as f32);
            layout.compute(root, available).map_err(AppError::Layout)?;
            for node in &built {
                self.apply_layout(node, &layout, available.height)?;
            }
            Ok(())
        }

        fn build_layout(
            &self,
            layout: &mut LayoutTree,
            id: NodeId,
        ) -> Result<BuiltLayout, AppError> {
            let children = self
                .children(id)
                .iter()
                .map(|child| self.build_layout(layout, *child))
                .collect::<Result<Vec<_>, _>>()?;
            let child_nodes = children
                .iter()
                .map(|child| child.layout)
                .collect::<Vec<_>>();
            let style = match self
                .value(id)
                .expect("render nodes have values")
                .appkit_node()
            {
                NodeKind::View { layout, .. } => layout,
                NodeKind::Window { .. } => return Err(AppError::InvalidTree),
            };
            let layout_id = layout
                .create_node(style, &child_nodes)
                .map_err(AppError::Layout)?;
            Ok(BuiltLayout {
                node: id,
                layout: layout_id,
                children,
            })
        }

        fn apply_layout(
            &self,
            node: &BuiltLayout,
            layout: &LayoutTree,
            parent_height: f32,
        ) -> Result<(), AppError> {
            let frame = layout.layout(node.layout).map_err(AppError::Layout)?;
            if let Some(native) = self.node(node.node).native.as_ref() {
                native
                    .as_view()
                    .set_frame(native_frame(frame, parent_height));
            }
            for child in &node.children {
                self.apply_layout(child, layout, frame.height)?;
            }
            Ok(())
        }

        fn update_native(&self, id: NodeId, old: &NodeKind, new: &NodeKind) {
            let Some(native) = self.node(id).native.as_ref() else {
                return;
            };
            if let (NodeKind::View { id: old_id, .. }, NodeKind::View { id: new_id, .. }) =
                (old, new)
                && old_id != new_id
            {
                native.as_view().set_identifier(new_id.as_deref());
            }
            match (native, old, new) {
                (
                    NativeNode::Label(field),
                    NodeKind::View {
                        kind: ViewKind::Label(old),
                        ..
                    },
                    NodeKind::View {
                        kind: ViewKind::Label(new),
                        ..
                    },
                ) if old != new => field.set_string_value(new),
                (
                    NativeNode::Button(button),
                    NodeKind::View {
                        kind: ViewKind::Button(old),
                        ..
                    },
                    NodeKind::View {
                        kind: ViewKind::Button(new),
                        ..
                    },
                ) if old != new => button.set_title(new),
                (
                    NativeNode::TextField(field),
                    NodeKind::View {
                        kind:
                            ViewKind::TextField {
                                on_change: old_handler,
                                ..
                            },
                        ..
                    },
                    NodeKind::View {
                        kind:
                            ViewKind::TextField {
                                value,
                                placeholder,
                                on_change,
                            },
                        ..
                    },
                ) => {
                    if field.string_value() != *value {
                        field.set_string_value(value);
                    }
                    field.set_placeholder_string(placeholder.as_deref());
                    if old_handler != on_change {
                        if *on_change {
                            self.install_change_handler(field, id);
                        } else {
                            field.clear_change_handler();
                        }
                    }
                }
                _ => {}
            }
        }
    }

    impl<R: AppKitRenderNode> RenderBackend<R> for AppKitBackend<R> {
        type NodeId = NodeId;

        fn root(&self) -> Self::NodeId {
            NodeId(0)
        }

        fn create(&mut self, value: &R) -> Self::NodeId {
            let id = NodeId(self.nodes.len());
            self.nodes.push(Some(BackendNode {
                value: Some(value.clone()),
                parent: None,
                children: Vec::new(),
                native: None,
            }));
            self.structure_dirty = true;
            self.layout_dirty = true;
            id
        }

        fn update(&mut self, id: Self::NodeId, value: &R) {
            let old = self
                .value(id)
                .expect("render nodes have values")
                .appkit_node();
            let new = value.appkit_node();
            self.update_native(id, &old, &new);
            self.node_mut(id).value = Some(value.clone());
            self.layout_dirty = true;
        }

        fn insert_child(&mut self, parent: Self::NodeId, index: usize, child: Self::NodeId) {
            self.detach(child);
            let index = index.min(self.node(parent).children.len());
            self.node_mut(parent).children.insert(index, child);
            self.node_mut(child).parent = Some(parent);
            self.structure_dirty = true;
            self.layout_dirty = true;
        }

        fn remove(&mut self, id: Self::NodeId) {
            self.detach(id);
            if let Some(mut node) = self.nodes.get_mut(id.0).and_then(Option::take)
                && let Some(native) = node.native.take()
            {
                native.as_view().remove_from_superview();
            }
            self.structure_dirty = true;
            self.layout_dirty = true;
        }

        fn move_child(&mut self, parent: Self::NodeId, index: usize, child: Self::NodeId) {
            if self.node(child).parent == Some(parent)
                && self.node(parent).children.get(index) == Some(&child)
            {
                return;
            }
            self.insert_child(parent, index, child);
        }
    }

    struct BuiltLayout {
        node: NodeId,
        layout: zintl_ui_layout::NodeId,
        children: Vec<BuiltLayout>,
    }

    enum Message {
        Event(Event),
    }

    struct AppHandler<'application, R, F>
    where
        R: AppKitRenderNode,
    {
        application: &'application Application<()>,
        composer: Composer<R, AppKitBackend<R>>,
        windows: HashMap<NodeId, NativeWindow<'application, ()>>,
        on_event: F,
        error: Rc<RefCell<Option<AppError>>>,
    }

    impl<R, F> AppHandler<'_, R, F>
    where
        R: AppKitRenderNode,
        F: FnMut(&mut Composer<R, AppKitBackend<R>>, Event),
    {
        fn synchronize(&mut self, cx: &MessageContext<'_, Message>) -> bool {
            match self.composer.backend_mut().synchronize(
                self.application,
                &mut self.windows,
                cx.sender(),
            ) {
                Ok(()) => true,
                Err(error) => {
                    *self.error.borrow_mut() = Some(error);
                    cx.request_termination();
                    false
                }
            }
        }
    }

    impl<R, F> MessageLoopHandler<Message> for AppHandler<'_, R, F>
    where
        R: AppKitRenderNode,
        F: FnMut(&mut Composer<R, AppKitBackend<R>>, Event),
    {
        fn init(&mut self, cx: &MessageContext<'_, Message>) {
            self.synchronize(cx);
        }

        fn on(&mut self, cx: &MessageContext<'_, Message>, message: Message) {
            match message {
                Message::Event(event) => {
                    (self.on_event)(&mut self.composer, event);
                    self.synchronize(cx);
                }
            }
        }
    }

    pub fn run_composer<R, F>(
        composer: Composer<R, AppKitBackend<R>>,
        on_event: F,
    ) -> Result<(), AppError>
    where
        R: AppKitRenderNode,
        F: FnMut(&mut Composer<R, AppKitBackend<R>>, Event),
    {
        let application = Application::new(()).map_err(AppError::Application)?;
        application
            .set_commands(&quit_commands(), |_| {})
            .map_err(AppError::Command)?;

        let error = Rc::new(RefCell::new(None));
        let message_loop = MessageLoopAppkit::new(
            &application,
            AppHandler {
                application: &application,
                composer,
                windows: HashMap::new(),
                on_event,
                error: error.clone(),
            },
        )
        .map_err(AppError::RunLoopSource)?;
        message_loop.run();
        error.borrow_mut().take().map_or(Ok(()), Err)
    }

    fn native_rect(rect: Rect) -> NativeRect {
        NativeRect::new(rect.x, rect.y, rect.width, rect.height)
    }

    fn native_frame(frame: zintl_ui_layout::Rect, parent_height: f32) -> NativeRect {
        NativeRect::new(
            frame.x as f64,
            (parent_height - frame.y - frame.height) as f64,
            frame.width as f64,
            frame.height as f64,
        )
    }

    fn quit_commands() -> CommandSet {
        CommandSet {
            app_menu: Some(WindowAppMenu {
                items: vec![CommandItem {
                    id: None,
                    title: "Quit".into(),
                    role: Some(CommandRole::Quit),
                    key: Some("q".into()),
                    modifiers: vec![CommandModifier::Cmd],
                    enabled: true,
                }],
            }),
            menus: Vec::new(),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn converts_taffy_top_origin_to_cgrect_bottom_origin() {
            // Verifies AppKit frames flip Taffy's vertical coordinate within the parent view.
            let frame = zintl_ui_layout::Rect {
                x: 12.0,
                y: 20.0,
                width: 80.0,
                height: 32.0,
            };

            assert_eq!(
                native_frame(frame, 100.0),
                NativeRect::new(12.0, 48.0, 80.0, 32.0)
            );
        }
    }
}

#[cfg(target_os = "macos")]
pub use backend::{AppError, AppKitBackend, run_composer};
