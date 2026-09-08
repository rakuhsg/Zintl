use std::rc::Rc;

use crate::{Context, Element, Event, EventKind, RenderNode, Store};

type SelectionAction = dyn for<'a> Fn(&mut Context<'a>, &str);

#[derive(Clone, Default)]
pub struct Sidebar {
    sections: Vec<SidebarSection>,
    selection: Option<Store<Option<String>>>,
    action: Option<Rc<SelectionAction>>,
}

impl std::fmt::Debug for Sidebar {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Sidebar")
            .field("sections", &self.sections)
            .field("selection", &self.selection)
            .field("has_action", &self.action.is_some())
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarSection {
    pub title: Option<String>,
    pub items: Vec<SidebarItem>,
}

impl SidebarSection {
    pub fn new(items: impl IntoIterator<Item = SidebarItem>) -> Self {
        Self {
            title: None,
            items: items.into_iter().collect(),
        }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarItem {
    pub id: String,
    pub title: String,
    pub system_image: Option<String>,
}

impl SidebarItem {
    /// Item IDs should be unique across the sidebar.
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            system_image: None,
        }
    }

    pub fn system_image(mut self, name: impl Into<String>) -> Self {
        self.system_image = Some(name.into());
        self
    }
}

/// Rendered sidebar data without Store handles or native AppKit objects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarState {
    pub sections: Vec<SidebarSection>,
    pub selected_id: Option<String>,
}

impl Sidebar {
    pub fn new(sections: impl IntoIterator<Item = SidebarSection>) -> Self {
        Self {
            sections: sections.into_iter().collect(),
            selection: None,
            action: None,
        }
    }

    pub fn bind(mut self, selection: Store<Option<String>>) -> Self {
        self.selection = Some(selection);
        self
    }

    /// Runs after AppKit changes the selection and updates the bound Store.
    pub fn on_select(mut self, action: impl for<'a> Fn(&mut Context<'a>, &str) + 'static) -> Self {
        self.action = Some(Rc::new(action));
        self
    }

    pub(crate) fn state(&self, cx: &mut Context<'_>) -> SidebarState {
        SidebarState {
            sections: self.sections.clone(),
            selected_id: self.selection.and_then(|store| cx.get(store).clone()),
        }
    }

    pub(crate) fn route(&self, element: Element<RenderNode>) -> Element<RenderNode> {
        let store = self.selection;
        let action = self.action.clone();
        if store.is_none() && action.is_none() {
            return element;
        }
        let sections = self.sections.clone();
        element.on_event(EventKind::SidebarSelectionChanged, move |cx, event| {
            if let Event::SidebarSelectionChanged { id } = event
                && sections
                    .iter()
                    .flat_map(|section| &section.items)
                    .any(|item| item.id == id)
            {
                if let Some(store) = store {
                    cx.update(store, |selection| *selection = Some(id.clone()));
                }
                if let Some(action) = &action {
                    action(cx, &id);
                }
            }
        })
    }
}

#[cfg(target_os = "macos")]
impl SidebarState {
    pub(crate) fn backend(&self) -> zintl_ui_appkit_backend::Sidebar {
        zintl_ui_appkit_backend::Sidebar {
            selected_id: self.selected_id.clone(),
            sections: self
                .sections
                .iter()
                .map(|section| zintl_ui_appkit_backend::SidebarSection {
                    title: section.title.clone(),
                    items: section
                        .items
                        .iter()
                        .map(|item| zintl_ui_appkit_backend::SidebarItem {
                            id: item.id.clone(),
                            title: item.title.clone(),
                            system_image: item.system_image.clone(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}
