//! AppKit rendering backend for Zintl's desktop UI descriptions.

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
        on_change: Option<u64>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    TextChanged { id: u64, value: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ViewSpec {
    pub kind: ViewKind,
    pub layout: LayoutStyle,
    pub children: Vec<ViewSpec>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowSpec {
    pub bounds: Rect,
    pub title: String,
    pub children: Vec<ViewSpec>,
}

#[cfg(target_os = "macos")]
mod backend {
    use std::cell::RefCell;
    use std::error::Error;
    use std::fmt;
    use std::rc::Rc;

    use messageloop_appkit::{
        Context as MessageContext, MessageLoopAppkit, MessageLoopHandler, SendError, Sender,
    };
    use zintl_ui_layout::{LayoutError, LayoutTree, NodeId, Size};
    use zpd_appkit::geometry::Rect as NativeRect;
    use zpd_appkit::runloop::{Application, ApplicationError, RunLoopSourceError};
    use zpd_appkit::ui::{
        AsView, Button as NativeButton, CommandError, CommandItem, CommandModifier, CommandRole,
        CommandSet, TextField as NativeTextField, View as NativeView, ViewError,
        Window as NativeWindow, WindowAppMenu, WindowError,
    };

    use super::{Event, ViewKind, ViewSpec, WindowSpec};

    #[derive(Debug)]
    pub enum AppError {
        Application(ApplicationError),
        Command(CommandError),
        Window(WindowError),
        View(ViewError),
        Layout(LayoutError),
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
                Self::View(error) => error.fmt(formatter),
                Self::Layout(error) => error.fmt(formatter),
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
                Self::View(error) => Some(error),
                Self::Layout(error) => Some(error),
                Self::RunLoopSource(error) => Some(error),
                Self::MessageLoop(error) => Some(error),
                Self::NoWindow => None,
            }
        }
    }

    struct AppHandler<'application, F> {
        windows: Rc<RefCell<Vec<NativeWindow<'application, ()>>>>,
        on_event: F,
    }

    enum Message {
        ShowWindow { index: usize },
        Event(Event),
    }

    impl<F> MessageLoopHandler<Message> for AppHandler<'_, F>
    where
        F: FnMut(Event),
    {
        fn on(&mut self, _cx: &MessageContext<'_, Message>, message: Message) {
            match message {
                Message::ShowWindow { index } => self
                    .windows
                    .borrow()
                    .get(index)
                    .expect("show-window message must reference an existing window")
                    .show()
                    .expect("a newly created AppKit window must still be open"),
                Message::Event(event) => (self.on_event)(event),
            }
        }
    }

    pub fn run(specifications: Vec<WindowSpec>) -> Result<(), AppError> {
        run_with_event_handler(specifications, |_| {})
    }

    pub fn run_with_event_handler<F>(
        specifications: Vec<WindowSpec>,
        on_event: F,
    ) -> Result<(), AppError>
    where
        F: FnMut(Event) + 'static,
    {
        if specifications.is_empty() {
            return Err(AppError::NoWindow);
        }

        let application = Application::new(()).map_err(AppError::Application)?;
        application
            .set_commands(&quit_commands(), |_| {})
            .map_err(AppError::Command)?;

        let windows = Rc::new(RefCell::new(Vec::with_capacity(specifications.len())));
        let message_loop = MessageLoopAppkit::new(
            &application,
            AppHandler {
                windows: windows.clone(),
                on_event,
            },
        )
        .map_err(AppError::RunLoopSource)?;
        let sender = message_loop.sender();

        for specification in specifications {
            let window = application.create_window(()).map_err(AppError::Window)?;
            window
                .set_title(&specification.title)
                .map_err(AppError::Window)?;
            window
                .set_bounds(NativeRect::new(
                    specification.bounds.x,
                    specification.bounds.y,
                    specification.bounds.width,
                    specification.bounds.height,
                ))
                .map_err(AppError::Window)?;
            install_views(&application, &window, &specification, &sender)?;
            windows.borrow_mut().push(window);
        }
        let window_count = windows.borrow().len();

        for index in 0..window_count {
            sender
                .send(Message::ShowWindow { index })
                .map_err(AppError::MessageLoop)?;
        }
        message_loop.run();
        Ok(())
    }

    fn install_views(
        application: &Application<()>,
        window: &NativeWindow<'_, ()>,
        specification: &WindowSpec,
        sender: &messageloop_appkit::AppkitSender<Message>,
    ) -> Result<(), AppError> {
        let mut layout = LayoutTree::new();
        let mut nodes = Vec::new();
        for child in &specification.children {
            nodes.push(build_layout_tree(&mut layout, child)?);
        }
        let root_children: Vec<_> = nodes.iter().map(|node| node.node).collect();
        let root = layout
            .create_node(
                zintl_ui_layout::LayoutStyle::stack(zintl_ui_layout::Axis::Vertical, 0.0),
                &root_children,
            )
            .map_err(AppError::Layout)?;
        let available = Size::new(
            specification.bounds.width as f32,
            specification.bounds.height as f32,
        );
        layout.compute(root, available).map_err(AppError::Layout)?;

        let content_view = window.content_view().map_err(AppError::Window)?;
        for (child, node) in specification.children.iter().zip(&nodes) {
            install_view(
                application,
                content_view,
                child,
                node,
                &layout,
                available.height,
                sender,
            )?;
        }
        Ok(())
    }

    struct BuiltLayout {
        node: NodeId,
        children: Vec<BuiltLayout>,
    }

    fn build_layout_tree(
        layout: &mut LayoutTree,
        view: &ViewSpec,
    ) -> Result<BuiltLayout, AppError> {
        let children = view
            .children
            .iter()
            .map(|child| build_layout_tree(layout, child))
            .collect::<Result<Vec<_>, _>>()?;
        let child_nodes: Vec<_> = children.iter().map(|child| child.node).collect();
        let node = layout
            .create_node(view.layout, &child_nodes)
            .map_err(AppError::Layout)?;
        Ok(BuiltLayout { node, children })
    }

    fn install_view(
        application: &Application<()>,
        parent: impl AsView,
        view: &ViewSpec,
        built: &BuiltLayout,
        layout: &LayoutTree,
        parent_height: f32,
        sender: &messageloop_appkit::AppkitSender<Message>,
    ) -> Result<(), AppError> {
        let frame = layout.layout(built.node).map_err(AppError::Layout)?;
        let native_frame = native_frame(frame, parent_height);

        match &view.kind {
            ViewKind::Container => {
                let native = NativeView::new(application, native_frame).map_err(AppError::View)?;
                parent.add_subview(&native);
                for (child, child_layout) in view.children.iter().zip(&built.children) {
                    install_view(
                        application,
                        native.as_view(),
                        child,
                        child_layout,
                        layout,
                        frame.height,
                        sender,
                    )?;
                }
            }
            ViewKind::Label(content) => {
                let native = NativeTextField::label_with_string(application, content)
                    .map_err(AppError::View)?;
                native.set_frame(native_frame);
                parent.add_subview(&native);
            }
            ViewKind::Button(title) => {
                let native =
                    NativeButton::with_title(application, title).map_err(AppError::View)?;
                native.set_frame(native_frame);
                parent.add_subview(&native);
            }
            ViewKind::TextField {
                value,
                placeholder,
                on_change,
            } => {
                let native =
                    NativeTextField::with_string(application, value).map_err(AppError::View)?;
                native.set_placeholder_string(placeholder.as_deref());
                if let Some(id) = on_change {
                    let id = *id;
                    let sender = sender.clone();
                    native.set_change_handler(move |value| {
                        let _ = sender.send(Message::Event(Event::TextChanged { id, value }));
                    });
                }
                native.set_frame(native_frame);
                parent.add_subview(&native);
            }
        }
        Ok(())
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
pub use backend::{AppError, run, run_with_event_handler};
