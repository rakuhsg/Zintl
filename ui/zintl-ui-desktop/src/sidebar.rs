use std::rc::Rc;

use crate::{Context, Element, Event, EventKind, RenderNode, Store};

/// Declarative navigation attached to a Window. Native support is AppKit-only.
#[derive(Clone, Default)]
pub struct Sidebar {
    sections: Vec<SidebarSection>,
    selection: Option<Store<Option<String>>>,
    action: Option<Rc<dyn for<'a> Fn(&mut Context<'a>, &str)>>,
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

/// Rendered data without Store handles or native AppKit objects.
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

    /// Runs after a native selection change, after updating any bound Store.
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
    pub(crate) fn appkit(&self) -> zintl_ui_appkit::Sidebar {
        zintl_ui_appkit::Sidebar {
            selected_id: self.selected_id.clone(),
            sections: self
                .sections
                .iter()
                .map(|section| zintl_ui_appkit::SidebarSection {
                    title: section.title.clone(),
                    items: section
                        .items
                        .iter()
                        .map(|item| zintl_ui_appkit::SidebarItem {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{App, IntoElement, Rect, View, Window};
    use std::cell::Cell;
    use std::rc::Rc;

    struct Navigation {
        selection: Store<Option<String>>,
        captured: Rc<Cell<Store<Option<String>>>>,
    }

    impl View for Navigation {
        type Output = RenderNode;

        fn init(&mut self, cx: &mut Context<'_>) {
            self.selection = cx.store(Some("home".into()));
            self.captured.set(self.selection);
        }

        fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = RenderNode> {
            Window::new(Rect::new(0.0, 0.0, 640.0, 480.0), "Navigation").sidebar(
                Sidebar::new([SidebarSection::new([
                    SidebarItem::new("home", "Home"),
                    SidebarItem::new("settings", "Settings").system_image("gearshape"),
                ])
                .title("Pages")])
                .bind(self.selection)
                .on_select({
                    let store = self.selection;
                    move |cx, id| assert_eq!(cx.get(store).as_deref(), Some(id))
                }),
            )
        }
    }

    #[test]
    fn selection_round_trips_and_rejects_unknown_items() {
        // Verifies native input, stale IDs, and programmatic deselection share one Store.
        let captured = Rc::new(Cell::new(Store::default()));
        let mut app = App::new(Navigation {
            selection: Store::default(),
            captured: captured.clone(),
        });
        assert!(
            matches!(app.render(), RenderNode::Window { sidebar: Some(SidebarState { selected_id: Some(id), .. }), .. } if id == "home")
        );
        app.dispatch_root_event(Event::SidebarSelectionChanged {
            id: "settings".into(),
        });
        assert_eq!(
            app.composer.context(|cx| cx.get(captured.get()).clone()),
            Some("settings".into())
        );
        app.dispatch_root_event(Event::SidebarSelectionChanged {
            id: "missing".into(),
        });
        assert!(
            matches!(app.render(), RenderNode::Window { sidebar: Some(SidebarState { selected_id: Some(id), .. }), .. } if id == "settings")
        );
        app.composer
            .context(|cx| cx.update(captured.get(), |selection| *selection = None));
        app.composer.flush();
        assert!(matches!(
            app.render(),
            RenderNode::Window {
                sidebar: Some(SidebarState {
                    selected_id: None,
                    ..
                }),
                ..
            }
        ));
    }

    #[test]
    fn callback_without_binding_receives_only_known_items() {
        // Verifies on_select works independently and ignores stale native IDs.
        let received = Rc::new(std::cell::RefCell::new(Vec::new()));
        let callback = received.clone();
        let mut app = App::new(
            Window::new(Rect::new(0.0, 0.0, 640.0, 480.0), "Callback").sidebar(
                Sidebar::new([SidebarSection::new([SidebarItem::new("home", "Home")])])
                    .on_select(move |_, id| callback.borrow_mut().push(id.to_owned())),
            ),
        );
        assert!(received.borrow().is_empty());
        app.dispatch_root_event(Event::SidebarSelectionChanged { id: "home".into() });
        app.dispatch_root_event(Event::SidebarSelectionChanged {
            id: "missing".into(),
        });
        assert_eq!(&*received.borrow(), &["home"]);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn appkit_translation_preserves_sections_and_symbols() {
        // Verifies adapter data preserves navigation metadata without exposing native types.
        let state = SidebarState {
            selected_id: Some("home".into()),
            sections: vec![
                SidebarSection::new([SidebarItem::new("home", "Home").system_image("house")])
                    .title("Pages"),
            ],
        };
        let native = state.appkit();
        assert_eq!(native.selected_id, state.selected_id);
        assert_eq!(native.sections[0].title.as_deref(), Some("Pages"));
        assert_eq!(native.sections[0].items[0].id, "home");
        assert_eq!(native.sections[0].items[0].title, "Home");
        assert_eq!(
            native.sections[0].items[0].system_image.as_deref(),
            Some("house")
        );
    }
}
