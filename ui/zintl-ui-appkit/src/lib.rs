//! AppKit renderer for Zintl UI trees.

use zintl_ui::renderer::RenderNode;
use zintl_ui_layout::LayoutStyle;
pub use zpd_appkit::actor::WindowEventKind as AppKitEvent;

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
    fn appkit_event(event: AppKitEvent) -> Self::Event;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(usize);

#[cfg(target_os = "macos")]
mod backend {
    use std::cell::RefCell;
    use std::error::Error;
    use std::fmt;
    use std::rc::Rc;

    use messageloop_appkit::{
        Context as MessageContext, MessageLoopAppkit, MessageLoopError, MessageLoopHandler,
    };
    use zintl_ui::composer::Composer;
    use zintl_ui::event::EventRouteId;
    use zintl_ui::renderer::RenderBackend;
    use zintl_ui_layout::{
        CrossAxisAlignment, LayoutDimension, LayoutError, LayoutStyle, LayoutTree, Size,
    };
    use zpd_appkit::actor::{ActorId, EventRouteToken, WindowEvent, WindowEventKind};
    use zpd_appkit::geometry::Rect as NativeRect;
    use zpd_appkit::runloop::{Application, ApplicationError};
    use zpd_appkit::ui::{
        AsView, Button as NativeButton, CommandError, CommandItem, CommandModifier, CommandRole,
        CommandSet, TextField as NativeTextField, View as NativeView, ViewError, ViewRef,
        WindowAppMenu, WindowError,
    };

    use super::{AppKitRenderNode, NodeId, NodeKind, Rect, ViewKind};

    #[derive(Debug)]
    pub enum AppError {
        Application(ApplicationError),
        Command(CommandError),
        Window(WindowError),
        View(ViewError),
        Layout(LayoutError),
        MessageLoop(MessageLoopError),
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
                Self::MessageLoop(error) => error.fmt(formatter),
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
                Self::MessageLoop(error) => Some(error),
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
        native: Option<Rc<NativeNode>>,
        native_window: Option<ActorId>,
        window_layout: Option<Rc<RefCell<WindowLayout>>>,
        applied_window_bounds: Option<Rect>,
        event_route: Option<EventRouteId>,
    }

    pub struct AppKitBackend<R: AppKitRenderNode> {
        nodes: Vec<Option<BackendNode<R>>>,
        retired_windows: Vec<ActorId>,
        structure_dirty: bool,
        layout_dirty: bool,
        pending_error: Rc<RefCell<Option<AppError>>>,
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
                    native_window: None,
                    window_layout: None,
                    applied_window_bounds: None,
                    event_route: None,
                })],
                retired_windows: Vec::new(),
                structure_dirty: true,
                layout_dirty: true,
                pending_error: Rc::new(RefCell::new(None)),
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

        /// Returns the Composer route associated with a rendered backend node.
        pub fn event_route(&self, id: NodeId) -> Option<EventRouteId> {
            self.node(id).event_route
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

        fn record_error(&mut self, error: AppError) {
            record_error(&self.pending_error, error);
        }

        fn remove_subtree(&mut self, id: NodeId) {
            let Some(children) = self
                .nodes
                .get(id.0)
                .and_then(Option::as_ref)
                .map(|node| node.children.clone())
            else {
                return;
            };
            for child in children {
                self.remove_subtree(child);
            }
            self.detach(id);
            if let Some(window) = self.node(id).native_window {
                self.retired_windows.push(window);
            }
            if let Some(mut node) = self.nodes.get_mut(id.0).and_then(Option::take)
                && let Some(native) = node.native.take()
            {
                if let Err(error) = native.as_view().remove_from_superview() {
                    self.record_error(AppError::View(error));
                }
            }
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

        fn synchronize<'application, 'windows>(
            &mut self,
            application: &'application Application<()>,
            cx: &MessageContext<'_, 'windows, Message>,
        ) -> Result<(), AppError> {
            if let Some(error) = self.pending_error.borrow_mut().take() {
                return Err(error);
            }
            for window in self.retired_windows.drain(..) {
                cx.remove_window(window);
            }
            let window_ids = self.window_ids();
            if window_ids.is_empty() {
                return Err(AppError::NoWindow);
            }

            let rebuild_structure = self.structure_dirty;
            for window_id in window_ids {
                let description = self
                    .value(window_id)
                    .expect("window nodes have values")
                    .appkit_node();
                let NodeKind::Window { bounds, title, id } = description else {
                    continue;
                };

                let new_window = self
                    .node(window_id)
                    .native_window
                    .is_none_or(|id| !cx.contains_window(id));
                if new_window {
                    let route = self.node(window_id).event_route.map(route_token);
                    let native_id = cx
                        .create_window_with_event_route(route)
                        .map_err(AppError::Window)?;
                    let layout = Rc::new(RefCell::new(WindowLayout::default()));
                    let callback_layout = layout.clone();
                    let callback_error = self.pending_error.clone();
                    cx.with_window(native_id, |window| {
                        window.set_content_layout_handler(move |bounds| {
                            if let Err(error) = callback_layout.borrow().layout(bounds) {
                                record_error(&callback_error, error);
                            }
                        })
                    })
                    .ok_or(AppError::Window(WindowError::Closed))?
                    .map_err(AppError::Window)?;
                    let node = self.node_mut(window_id);
                    node.native_window = Some(native_id);
                    node.window_layout = Some(layout);
                    node.applied_window_bounds = None;
                }
                let native_id = self
                    .node(window_id)
                    .native_window
                    .expect("a synchronized window has a native Actor");
                let event_route = self.node(window_id).event_route.map(route_token);
                let apply_bounds =
                    window_bounds_need_update(self.node(window_id).applied_window_bounds, bounds);
                cx.with_window(native_id, |window| -> Result<(), AppError> {
                    window
                        .actor_ref()
                        .set_event_route(event_route)
                        .map_err(WindowError::from)
                        .map_err(AppError::Window)?;
                    if apply_bounds {
                        window
                            .set_bounds(native_rect(bounds))
                            .map_err(AppError::Window)?;
                    }
                    window.set_title(&title).map_err(AppError::Window)?;
                    window
                        .set_identifier(id.as_deref())
                        .map_err(AppError::Window)
                })
                .ok_or(AppError::Window(WindowError::Closed))??;
                if apply_bounds {
                    self.node_mut(window_id).applied_window_bounds = Some(bounds);
                }

                self.materialize_children(application, window_id)?;
                if rebuild_structure {
                    cx.with_window(native_id, |window| -> Result<(), AppError> {
                        let content = window.content_view().map_err(AppError::Window)?;
                        self.attach_children(content, window_id)
                    })
                    .ok_or(AppError::Window(WindowError::Closed))??;
                }
                let update_layout = self.layout_dirty || rebuild_structure || new_window;
                if update_layout {
                    let snapshot = WindowLayout {
                        children: self
                            .children(window_id)
                            .iter()
                            .map(|child| self.build_layout_snapshot(*child))
                            .collect::<Result<Vec<_>, _>>()?,
                    };
                    let state = self
                        .node(window_id)
                        .window_layout
                        .as_ref()
                        .expect("a synchronized window has layout state");
                    *state.borrow_mut() = snapshot;
                    cx.with_window(native_id, |window| -> Result<(), AppError> {
                        let content = window.content_view().map_err(AppError::Window)?;
                        content.set_needs_layout(true).map_err(AppError::View)?;
                        content.layout_subtree_if_needed().map_err(AppError::View)
                    })
                    .ok_or(AppError::Window(WindowError::Closed))??;
                    if let Some(error) = self.pending_error.borrow_mut().take() {
                        return Err(error);
                    }
                }

                if new_window {
                    cx.with_window(native_id, |window| window.show())
                        .ok_or(AppError::Window(WindowError::Closed))?
                        .map_err(AppError::Window)?;
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
            &mut self,
            application: &Application<()>,
            id: NodeId,
            description: &NodeKind,
        ) -> Result<Option<Rc<NativeNode>>, AppError> {
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
                ViewKind::TextField { value, placeholder } => {
                    let field =
                        NativeTextField::with_string(application, value).map_err(AppError::View)?;
                    field
                        .set_placeholder_string(placeholder.as_deref())
                        .map_err(AppError::View)?;
                    NativeNode::TextField(field)
                }
            };
            native
                .as_view()
                .set_identifier(accessibility_id.as_deref())
                .map_err(AppError::View)?;
            native
                .as_view()
                .actor_ref()
                .set_event_route(self.node(id).event_route.map(route_token))
                .map_err(ViewError::from)
                .map_err(AppError::View)?;
            Ok(Some(Rc::new(native)))
        }

        fn attach_children(&self, parent: ViewRef<'_>, parent_id: NodeId) -> Result<(), AppError> {
            for child in self.children(parent_id) {
                let Some(native) = self.node(*child).native.as_ref() else {
                    continue;
                };
                native
                    .as_view()
                    .remove_from_superview()
                    .map_err(AppError::View)?;
                parent
                    .add_subview(&native.as_view())
                    .map_err(AppError::View)?;
                self.attach_children(native.as_view(), *child)?;
            }
            Ok(())
        }

        fn build_layout_snapshot(&self, id: NodeId) -> Result<LayoutSnapshot, AppError> {
            let children = self
                .children(id)
                .iter()
                .map(|child| self.build_layout_snapshot(*child))
                .collect::<Result<Vec<_>, _>>()?;
            let style = match self
                .value(id)
                .expect("render nodes have values")
                .appkit_node()
            {
                NodeKind::View { layout, .. } => layout,
                NodeKind::Window { .. } => return Err(AppError::InvalidTree),
            };
            let native = self
                .node(id)
                .native
                .as_ref()
                .expect("materialized view nodes have native views")
                .clone();
            Ok(LayoutSnapshot {
                style,
                native,
                children,
            })
        }

        fn update_native(
            &self,
            id: NodeId,
            old: &NodeKind,
            new: &NodeKind,
        ) -> Result<(), AppError> {
            let Some(native) = self.node(id).native.as_ref() else {
                return Ok(());
            };
            if let (NodeKind::View { id: old_id, .. }, NodeKind::View { id: new_id, .. }) =
                (old, new)
                && old_id != new_id
            {
                native
                    .as_view()
                    .set_identifier(new_id.as_deref())
                    .map_err(AppError::View)?;
            }
            match (native.as_ref(), old, new) {
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
                ) if old != new => field.set_string_value(new).map_err(AppError::View)?,
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
                ) if old != new => button.set_title(new).map_err(AppError::View)?,
                (
                    NativeNode::TextField(field),
                    NodeKind::View {
                        kind: ViewKind::TextField { .. },
                        ..
                    },
                    NodeKind::View {
                        kind: ViewKind::TextField { value, placeholder },
                        ..
                    },
                ) => {
                    if field.string_value().map_err(AppError::View)? != *value {
                        field.set_string_value(value).map_err(AppError::View)?;
                    }
                    field
                        .set_placeholder_string(placeholder.as_deref())
                        .map_err(AppError::View)?;
                }
                _ => {}
            }
            Ok(())
        }
    }

    impl<R: AppKitRenderNode> RenderBackend<R> for AppKitBackend<R> {
        type NodeId = NodeId;

        fn root(&self) -> Self::NodeId {
            NodeId(0)
        }

        fn create(&mut self, value: &R, event_route: Option<EventRouteId>) -> Self::NodeId {
            let id = NodeId(self.nodes.len());
            self.nodes.push(Some(BackendNode {
                value: Some(value.clone()),
                parent: None,
                children: Vec::new(),
                native: None,
                native_window: None,
                window_layout: None,
                applied_window_bounds: None,
                event_route,
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
            if let Err(error) = self.update_native(id, &old, &new) {
                self.record_error(error);
            }
            self.node_mut(id).value = Some(value.clone());
            self.layout_dirty = true;
        }

        fn set_event_route(&mut self, id: Self::NodeId, event_route: Option<EventRouteId>) {
            self.node_mut(id).event_route = event_route;
            let token = event_route.map(route_token);
            let result = if let Some(native) = self.node(id).native.as_ref() {
                native.as_view().actor_ref().set_event_route(token)
            } else {
                Ok(())
            };
            if let Err(error) = result {
                self.record_error(AppError::View(ViewError::from(error)));
            }
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
            self.remove_subtree(id);
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

    #[derive(Default)]
    struct WindowLayout {
        children: Vec<LayoutSnapshot>,
    }

    impl WindowLayout {
        fn layout(&self, bounds: NativeRect) -> Result<(), AppError> {
            let available = Size::new(bounds.width as f32, bounds.height as f32);
            let mut layout = LayoutTree::new();
            let built = self
                .children
                .iter()
                .map(|child| child.build(&mut layout))
                .collect::<Result<Vec<_>, _>>()?;
            let root_children = built.iter().map(|node| node.layout).collect::<Vec<_>>();
            let mut root_style = LayoutStyle::stack(zintl_ui_layout::Axis::Vertical, 0.0);
            root_style.width = LayoutDimension::Points(available.width);
            root_style.cross_axis_alignment = CrossAxisAlignment::Start;
            let root = layout
                .create_node(root_style, &root_children)
                .map_err(AppError::Layout)?;
            layout.compute(root, available).map_err(AppError::Layout)?;
            for node in &built {
                node.apply(&layout, available.height)?;
            }
            Ok(())
        }
    }

    struct LayoutSnapshot {
        style: LayoutStyle,
        native: Rc<NativeNode>,
        children: Vec<LayoutSnapshot>,
    }

    impl LayoutSnapshot {
        fn build(&self, layout: &mut LayoutTree) -> Result<BuiltLayout, AppError> {
            let children = self
                .children
                .iter()
                .map(|child| child.build(layout))
                .collect::<Result<Vec<_>, _>>()?;
            let child_nodes = children
                .iter()
                .map(|child| child.layout)
                .collect::<Vec<_>>();
            let layout_id = layout
                .create_node(self.style, &child_nodes)
                .map_err(AppError::Layout)?;
            Ok(BuiltLayout {
                native: self.native.clone(),
                layout: layout_id,
                children,
            })
        }
    }

    struct BuiltLayout {
        native: Rc<NativeNode>,
        layout: zintl_ui_layout::NodeId,
        children: Vec<BuiltLayout>,
    }

    impl BuiltLayout {
        fn apply(&self, layout: &LayoutTree, parent_height: f32) -> Result<(), AppError> {
            let frame = layout.layout(self.layout).map_err(AppError::Layout)?;
            self.native
                .as_view()
                .set_frame(native_frame(frame, parent_height))
                .map_err(AppError::View)?;
            for child in &self.children {
                child.apply(layout, frame.height)?;
            }
            Ok(())
        }
    }

    enum Message {
        Window(WindowEvent),
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum WindowEventAction {
        Synchronize,
        WaitForClose,
        Terminate,
    }

    fn window_event_action(kind: &WindowEventKind) -> WindowEventAction {
        match kind {
            WindowEventKind::WillClose => WindowEventAction::WaitForClose,
            WindowEventKind::DidClose => WindowEventAction::Terminate,
            _ => WindowEventAction::Synchronize,
        }
    }

    impl From<WindowEvent> for Message {
        fn from(event: WindowEvent) -> Self {
            Self::Window(event)
        }
    }

    struct AppHandler<'application, R>
    where
        R: AppKitRenderNode,
    {
        application: &'application Application<()>,
        composer: Composer<R, AppKitBackend<R>>,
        error: Rc<RefCell<Option<AppError>>>,
    }

    impl<R> AppHandler<'_, R>
    where
        R: AppKitRenderNode,
    {
        fn synchronize(&mut self, cx: &MessageContext<'_, '_, Message>) -> bool {
            match self
                .composer
                .backend_mut()
                .synchronize(self.application, cx)
            {
                Ok(()) => true,
                Err(error) => {
                    *self.error.borrow_mut() = Some(error);
                    cx.request_termination();
                    false
                }
            }
        }
    }

    impl<R> MessageLoopHandler<Message> for AppHandler<'_, R>
    where
        R: AppKitRenderNode,
    {
        fn init(&mut self, cx: &MessageContext<'_, '_, Message>) {
            self.synchronize(cx);
        }

        fn on(&mut self, cx: &MessageContext<'_, '_, Message>, message: Message) {
            match message {
                Message::Window(event) => {
                    let action = window_event_action(&event.kind);
                    if let Some(route) = event.route {
                        let route = EventRouteId::from_raw(route.get());
                        self.composer
                            .dispatch_event(route, R::appkit_event(event.kind));
                    }
                    match action {
                        WindowEventAction::Synchronize => {
                            self.synchronize(cx);
                        }
                        WindowEventAction::WaitForClose => {}
                        WindowEventAction::Terminate => cx.request_termination(),
                    }
                }
            }
        }
    }

    pub fn run_composer<R>(composer: Composer<R, AppKitBackend<R>>) -> Result<(), AppError>
    where
        R: AppKitRenderNode,
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
                error: error.clone(),
            },
        )
        .map_err(AppError::MessageLoop)?;
        message_loop.run().map_err(AppError::MessageLoop)?;
        error.borrow_mut().take().map_or(Ok(()), Err)
    }

    fn route_token(route: EventRouteId) -> EventRouteToken {
        EventRouteToken::new(route.into_raw())
    }

    fn native_rect(rect: Rect) -> NativeRect {
        NativeRect::new(rect.x, rect.y, rect.width, rect.height)
    }

    fn window_bounds_need_update(applied: Option<Rect>, declared: Rect) -> bool {
        applied != Some(declared)
    }

    fn native_frame(frame: zintl_ui_layout::Rect, parent_height: f32) -> NativeRect {
        NativeRect::new(
            frame.x as f64,
            (parent_height - frame.y - frame.height) as f64,
            frame.width as f64,
            frame.height as f64,
        )
    }

    fn record_error(slot: &RefCell<Option<AppError>>, error: AppError) {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Some(error);
        }
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
        use zintl_ui::renderer::RenderNode;

        #[derive(Clone, PartialEq)]
        struct TestNode;

        #[derive(Clone)]
        struct TestEvent;

        impl zintl_ui::event::Event for TestEvent {
            type Kind = ();

            fn kind(&self) -> Self::Kind {}
        }

        impl RenderNode for TestNode {
            type Event = TestEvent;

            fn same_kind(&self, _other: &Self) -> bool {
                true
            }
        }

        impl AppKitRenderNode for TestNode {
            fn appkit_node(&self) -> NodeKind {
                NodeKind::View {
                    kind: ViewKind::Container,
                    layout: LayoutStyle::leaf(Size::new(0.0, 0.0)),
                    id: None,
                }
            }

            fn appkit_event(_event: crate::AppKitEvent) -> Self::Event {
                TestEvent
            }
        }

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

        #[test]
        fn removing_backend_parent_removes_its_descendants() {
            // Verifies renderer removal cannot leave detached descendants in backend storage.
            let mut backend = AppKitBackend::<TestNode>::new();
            let parent = backend.create(&TestNode, None);
            let child = backend.create(&TestNode, None);
            backend.insert_child(backend.root(), 0, parent);
            backend.insert_child(parent, 0, child);

            backend.remove(parent);

            assert!(backend.nodes[parent.0].is_none());
            assert!(backend.nodes[child.0].is_none());
            assert!(backend.children(backend.root()).is_empty());
        }

        #[test]
        fn backend_tracks_composer_event_routes_on_nodes() {
            // Verifies the adapter stores direct routes instead of building an Actor-to-UI lookup.
            let mut backend = AppKitBackend::<TestNode>::new();
            let route = EventRouteId::from_raw(17);
            let node = backend.create(&TestNode, Some(route));
            assert_eq!(backend.event_route(node), Some(route));

            backend.set_event_route(node, None);
            assert_eq!(backend.event_route(node), None);
        }

        #[test]
        fn resize_event_uses_normal_synchronization() {
            // Verifies resize events remain dispatchable without a separate Taffy layout path.
            assert_eq!(
                window_event_action(&WindowEventKind::DidResize),
                WindowEventAction::Synchronize
            );
        }

        #[test]
        fn unchanged_declarative_bounds_are_not_reapplied() {
            // Verifies later synchronization preserves a user-resized native window.
            let declared = Rect::new(10.0, 20.0, 640.0, 480.0);

            assert!(window_bounds_need_update(None, declared));
            assert!(!window_bounds_need_update(Some(declared), declared));
            assert!(window_bounds_need_update(
                Some(Rect::new(10.0, 20.0, 800.0, 600.0)),
                declared
            ));
        }

        #[test]
        fn closing_window_skips_synchronization_and_terminates_after_close() {
            // Verifies closing never synchronizes a closed native window and exits after DidClose.
            assert_eq!(
                window_event_action(&WindowEventKind::WillClose),
                WindowEventAction::WaitForClose
            );
            assert_eq!(
                window_event_action(&WindowEventKind::DidClose),
                WindowEventAction::Terminate
            );
            assert_eq!(
                window_event_action(&WindowEventKind::Created),
                WindowEventAction::Synchronize
            );
        }
    }
}

#[cfg(target_os = "macos")]
pub use backend::{AppError, AppKitBackend, run_composer};
