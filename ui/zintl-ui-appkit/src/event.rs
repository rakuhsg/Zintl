use zintl_ui::event::Event as EventTrait;

/// Selects the semantic AppKit event handled by an element route.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum EventKind {
    Activated,
    TextChanged,
    SidebarSelectionChanged,
    WindowCreated,
    WindowDidResize,
    WindowWillClose,
    WindowDidClose,
}

/// A semantic event produced by the AppKit backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Activated,
    TextChanged { value: String },
    SidebarSelectionChanged { id: String },
    WindowCreated,
    WindowDidResize,
    WindowWillClose,
    WindowDidClose,
}

impl EventTrait for Event {
    type Kind = EventKind;

    fn kind(&self) -> Self::Kind {
        match self {
            Self::Activated => EventKind::Activated,
            Self::TextChanged { .. } => EventKind::TextChanged,
            Self::SidebarSelectionChanged { .. } => EventKind::SidebarSelectionChanged,
            Self::WindowCreated => EventKind::WindowCreated,
            Self::WindowDidResize => EventKind::WindowDidResize,
            Self::WindowWillClose => EventKind::WindowWillClose,
            Self::WindowDidClose => EventKind::WindowDidClose,
        }
    }
}
