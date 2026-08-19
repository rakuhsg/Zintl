use zintl_ui::composer::Composer;
pub use zintl_ui::element::{Element, IntoElement};
use zintl_ui::renderer::{RenderBackend, RenderNode as RenderNodeTrait};
pub use zintl_ui::view::{Context, View};

pub use zpd_appkit::geometry::Rect;

#[derive(Clone, Debug, PartialEq)]
pub enum RenderNode {
    Text(String),
    Window { bounds: Rect, title: String },
}

impl RenderNodeTrait for RenderNode {
    fn same_kind(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::Text(_), Self::Text(_)) | (Self::Window { .. }, Self::Window { .. })
        )
    }
}

pub struct Window {
    bounds: Rect,
    title: String,
}

impl Window {
    pub fn new(bounds: Rect, title: impl Into<String>) -> Self {
        Self {
            bounds,
            title: title.into(),
        }
    }
}

impl View for Window {
    type Output = RenderNode;

    fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
        Element::node(RenderNode::Window {
            bounds: self.bounds,
            title: self.title.clone(),
        })
    }
}

pub struct Text {
    content: String,
}

impl Text {
    pub fn new(content: String) -> Self {
        Text { content }
    }
}

impl IntoElement for Text {
    type Output = RenderNode;

    fn into_element(self) -> Element<Self::Output> {
        Element::node(RenderNode::Text(self.content))
    }
}

struct Node {
    value: Option<RenderNode>,
    parent: Option<usize>,
    children: Vec<usize>,
}

struct TreeBackend {
    nodes: Vec<Option<Node>>,
}

impl TreeBackend {
    fn new() -> Self {
        Self {
            nodes: vec![Some(Node {
                value: None,
                parent: None,
                children: Vec::new(),
            })],
        }
    }

    fn node(&self, id: usize) -> &Node {
        self.nodes[id].as_ref().unwrap()
    }

    fn node_mut(&mut self, id: usize) -> &mut Node {
        self.nodes[id].as_mut().unwrap()
    }

    fn detach(&mut self, child: usize) {
        let Some(parent) = self.node(child).parent else {
            return;
        };
        self.node_mut(parent)
            .children
            .retain(|candidate| *candidate != child);
        self.node_mut(child).parent = None;
    }
}

impl RenderBackend<RenderNode> for TreeBackend {
    type NodeId = usize;

    fn root(&self) -> Self::NodeId {
        0
    }

    fn create(&mut self, value: &RenderNode) -> Self::NodeId {
        let id = self.nodes.len();
        self.nodes.push(Some(Node {
            value: Some(value.clone()),
            parent: None,
            children: Vec::new(),
        }));
        id
    }

    fn update(&mut self, node: Self::NodeId, value: &RenderNode) {
        self.node_mut(node).value = Some(value.clone());
    }

    fn insert_child(&mut self, parent: Self::NodeId, index: usize, child: Self::NodeId) {
        self.detach(child);
        let index = index.min(self.node(parent).children.len());
        self.node_mut(parent).children.insert(index, child);
        self.node_mut(child).parent = Some(parent);
    }

    fn remove(&mut self, node: Self::NodeId) {
        self.detach(node);
        self.nodes[node] = None;
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

pub struct App {
    composer: Composer<RenderNode, TreeBackend>,
}

impl App {
    pub fn new<E>(root: E) -> Self
    where
        E: IntoElement<Output = RenderNode>,
    {
        let mut composer = Composer::new(TreeBackend::new());
        composer.mount(root);
        Self { composer }
    }

    pub fn render(&self) -> RenderNode {
        let backend = self.composer.backend();
        let root = backend.node(0).children[0];
        backend.node(root).value.clone().unwrap()
    }
}

#[cfg(target_os = "macos")]
mod appkit {
    use std::error::Error;
    use std::fmt;

    use messageloop_appkit::{
        Context as MessageContext, MessageLoopAppkit, MessageLoopHandler, SendError, Sender,
    };
    use zpd_appkit::runloop::{Application, ApplicationError, RunLoopSourceError};
    use zpd_appkit::ui::{
        CommandError, CommandItem, CommandModifier, CommandRole, CommandSet,
        Window as NativeWindow, WindowAppMenu, WindowError,
    };

    use super::{App, RenderNode};

    #[derive(Debug)]
    pub enum AppError {
        Application(ApplicationError),
        Command(CommandError),
        Window(WindowError),
        RunLoopSource(RunLoopSourceError),
        MessageLoop(SendError),
        NoWindow,
    }

    impl fmt::Display for AppError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Application(error) => error.fmt(formatter),
                Self::Command(error) => error.fmt(formatter),
                Self::Window(error) => error.fmt(formatter),
                Self::RunLoopSource(error) => error.fmt(formatter),
                Self::MessageLoop(error) => error.fmt(formatter),
                Self::NoWindow => {
                    formatter.write_str("the rendered tree does not contain a window")
                }
            }
        }
    }

    impl Error for AppError {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            match self {
                Self::Application(error) => Some(error),
                Self::Command(error) => Some(error),
                Self::Window(error) => Some(error),
                Self::RunLoopSource(error) => Some(error),
                Self::MessageLoop(error) => Some(error),
                Self::NoWindow => None,
            }
        }
    }

    struct AppHandler<'application> {
        windows: Vec<NativeWindow<'application, ()>>,
    }

    enum Message {
        ShowWindow { index: usize },
    }

    impl MessageLoopHandler<Message> for AppHandler<'_> {
        fn on(&mut self, _cx: &MessageContext<'_, Message>, message: Message) {
            match message {
                Message::ShowWindow { index } => self
                    .windows
                    .get(index)
                    .expect("show-window message must reference an existing window")
                    .show()
                    .expect("a newly created AppKit window must still be open"),
            }
        }
    }

    impl App {
        pub fn run(self) -> Result<(), AppError> {
            let specifications: Vec<_> = self
                .composer
                .backend()
                .nodes
                .iter()
                .filter_map(|node| match node.as_ref()?.value.as_ref()? {
                    RenderNode::Window { bounds, title } => Some((*bounds, title.clone())),
                    RenderNode::Text(_) => None,
                })
                .collect();
            if specifications.is_empty() {
                return Err(AppError::NoWindow);
            }

            let application = Application::new(()).map_err(AppError::Application)?;
            application
                .set_commands(&quit_commands(), |_| {})
                .map_err(AppError::Command)?;

            let mut windows = Vec::with_capacity(specifications.len());
            for (bounds, title) in specifications {
                let window = application.create_window(()).map_err(AppError::Window)?;
                window.set_title(&title).map_err(AppError::Window)?;
                window.set_bounds(bounds).map_err(AppError::Window)?;
                windows.push(window);
            }
            let window_count = windows.len();

            let message_loop = MessageLoopAppkit::new(&application, AppHandler { windows })
                .map_err(AppError::RunLoopSource)?;
            let sender = message_loop.sender();
            for index in 0..window_count {
                sender
                    .send(Message::ShowWindow { index })
                    .map_err(AppError::MessageLoop)?;
            }
            message_loop.run();
            Ok(())
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
}

#[cfg(target_os = "macos")]
pub use appkit::AppError;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_element_preserves_bounds_and_title() {
        // Verifies the Window view preserves its declarative bounds and title.
        let bounds = Rect::new(10.0, 20.0, 640.0, 480.0);
        let app = App::new(Window::new(bounds, "Zintl"));
        assert_eq!(
            app.render(),
            RenderNode::Window {
                bounds,
                title: "Zintl".into(),
            }
        );
    }
}
