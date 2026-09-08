use zintl_ui::renderer::RenderNode as RenderNodeTrait;
use zintl_ui_layout::LayoutStyle;

use crate::{Event, SidebarState};

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
pub enum RenderNode {
    NSWindow {
        sidebar: Option<SidebarState>,
        bounds: Rect,
        title: String,
        full_size_content_view: bool,
        id: Option<String>,
    },
    NSView {
        layout: LayoutStyle,
        id: Option<String>,
    },
    NSButton {
        title: String,
        layout: LayoutStyle,
        id: Option<String>,
    },
    NSTextField {
        value: String,
        placeholder: Option<String>,
        editable: bool,
        selectable: bool,
        bordered: bool,
        draws_background: bool,
        layout: LayoutStyle,
        id: Option<String>,
    },
}

impl RenderNodeTrait for RenderNode {
    type Event = Event;

    fn same_kind(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::NSWindow { .. }, Self::NSWindow { .. })
                | (Self::NSView { .. }, Self::NSView { .. })
                | (Self::NSButton { .. }, Self::NSButton { .. })
                | (Self::NSTextField { .. }, Self::NSTextField { .. })
        )
    }
}

#[cfg(target_os = "macos")]
impl zintl_ui_appkit_backend::AppKitRenderNode for RenderNode {
    fn appkit_node(&self) -> zintl_ui_appkit_backend::NodeKind {
        use zintl_ui_appkit_backend::{NodeKind, ViewKind};

        match self {
            Self::NSWindow {
                sidebar,
                bounds,
                title,
                full_size_content_view,
                id,
            } => NodeKind::Window {
                sidebar: sidebar.as_ref().map(SidebarState::backend),
                bounds: zintl_ui_appkit_backend::Rect::new(
                    bounds.x,
                    bounds.y,
                    bounds.width,
                    bounds.height,
                ),
                title: title.clone(),
                full_size_content_view: *full_size_content_view,
                id: id.clone(),
            },
            Self::NSView { layout, id } => NodeKind::View {
                kind: ViewKind::Container,
                layout: *layout,
                id: id.clone(),
            },
            Self::NSButton { title, layout, id } => NodeKind::View {
                kind: ViewKind::Button(title.clone()),
                layout: *layout,
                id: id.clone(),
            },
            Self::NSTextField {
                value,
                placeholder,
                editable,
                selectable,
                bordered,
                draws_background,
                layout,
                id,
            } => NodeKind::View {
                kind: ViewKind::TextField {
                    value: value.clone(),
                    placeholder: placeholder.clone(),
                    editable: *editable,
                    selectable: *selectable,
                    bordered: *bordered,
                    draws_background: *draws_background,
                },
                layout: *layout,
                id: id.clone(),
            },
        }
    }

    fn appkit_event(event: zintl_ui_appkit_backend::AppKitEvent) -> Self::Event {
        match event {
            zintl_ui_appkit_backend::AppKitEvent::SidebarSelectionChanged { id } => {
                Event::SidebarSelectionChanged { id }
            }
            zintl_ui_appkit_backend::AppKitEvent::Created => Event::WindowCreated,
            zintl_ui_appkit_backend::AppKitEvent::DidResize => Event::WindowDidResize,
            zintl_ui_appkit_backend::AppKitEvent::WillClose => Event::WindowWillClose,
            zintl_ui_appkit_backend::AppKitEvent::DidClose => Event::WindowDidClose,
            zintl_ui_appkit_backend::AppKitEvent::ButtonClicked => Event::Activated,
            zintl_ui_appkit_backend::AppKitEvent::TextChanged { value } => {
                Event::TextChanged { value }
            }
        }
    }
}
