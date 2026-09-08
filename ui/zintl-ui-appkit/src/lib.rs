//! Declarative Zintl UI views that mirror AppKit widgets.

mod button;
mod children;
mod event;
mod render_node;
mod sidebar;
mod text_field;
mod view;
mod window;

pub use button::Button;
pub use children::{Children, Empty};
pub use event::{Event, EventKind};
pub use render_node::{Rect, RenderNode};
pub use sidebar::{Sidebar, SidebarItem, SidebarSection, SidebarState};
pub use text_field::TextField;
pub use view::View;
pub use window::Window;
pub use zintl_ui::element::{Element, IntoElement};
pub use zintl_ui::store::Store;
pub use zintl_ui::view::Context;
pub use zintl_ui_layout::{LayoutStyle, Size};

#[cfg(target_os = "macos")]
pub use zintl_ui_appkit_backend::{AppError, AppKitBackend, NodeId, run_composer};

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use zintl_ui::composer::Composer;
    use zintl_ui::renderer::{RenderBackend, RenderNode as _};
    use zintl_ui::view::View as ViewTrait;
    use zintl_ui_appkit_backend::AppKitRenderNode;
    use zintl_ui_layout::{Axis, LayoutStyle};

    use super::*;

    fn composer<E>(root: E) -> Composer<RenderNode, AppKitBackend<RenderNode>>
    where
        E: IntoElement<Output = RenderNode>,
    {
        let mut composer = Composer::new(AppKitBackend::new());
        composer.mount(root);
        composer
    }

    fn root_id(composer: &Composer<RenderNode, AppKitBackend<RenderNode>>) -> NodeId {
        let backend = composer.backend();
        *backend
            .children(backend.root())
            .first()
            .expect("the test tree must contain a root")
    }

    fn assert_view<T: ViewTrait<Output = RenderNode>>() {}

    #[test]
    fn appkit_widgets_are_zintl_views() {
        // Verifies each AppKit-shaped widget participates in declarative composition.
        assert_view::<Button>();
        assert_view::<TextField>();
        assert_view::<View<(Button,)>>();
        assert_view::<Window<(TextField,)>>();
    }

    #[test]
    fn text_field_presentations_share_one_native_kind() {
        // Verifies labels and editable fields are configurations of one NSTextField node.
        let label = composer(TextField::label_with_string("Label"));
        let field = composer(TextField::new());
        let label = label.backend().value(root_id(&label)).unwrap();
        let field = field.backend().value(root_id(&field)).unwrap();

        assert!(matches!(
            label,
            RenderNode::NSTextField {
                editable: false,
                selectable: false,
                bordered: false,
                draws_background: false,
                ..
            }
        ));
        assert!(matches!(
            field,
            RenderNode::NSTextField {
                editable: true,
                selectable: true,
                bordered: true,
                draws_background: true,
                ..
            }
        ));
        assert!(label.same_kind(field));
    }

    #[test]
    fn native_variants_translate_without_changing_widget_boundaries() {
        // Verifies AppKit render nodes map directly to backend widget descriptions.
        let nodes = [
            RenderNode::NSView {
                layout: LayoutStyle::stack(Axis::Vertical, 0.0),
                id: Some("content".into()),
            },
            RenderNode::NSButton {
                title: "Save".into(),
                layout: LayoutStyle::leaf(Size::new(80.0, 32.0)),
                id: Some("save".into()),
            },
            RenderNode::NSTextField {
                value: "Name".into(),
                placeholder: Some("Your name".into()),
                editable: true,
                selectable: true,
                bordered: true,
                draws_background: true,
                layout: LayoutStyle::leaf(Size::new(160.0, 28.0)),
                id: Some("name".into()),
            },
        ];

        assert!(matches!(
            nodes[0].appkit_node(),
            zintl_ui_appkit_backend::NodeKind::View {
                kind: zintl_ui_appkit_backend::ViewKind::Container,
                id: Some(id),
                ..
            } if id == "content"
        ));
        assert!(matches!(
            nodes[1].appkit_node(),
            zintl_ui_appkit_backend::NodeKind::View {
                kind: zintl_ui_appkit_backend::ViewKind::Button(title),
                id: Some(id),
                ..
            } if title == "Save" && id == "save"
        ));
        assert!(matches!(
            nodes[2].appkit_node(),
            zintl_ui_appkit_backend::NodeKind::View {
                kind: zintl_ui_appkit_backend::ViewKind::TextField { editable: true, .. },
                id: Some(id),
                ..
            } if id == "name"
        ));
    }

    #[test]
    fn text_binding_and_button_action_route_through_appkit_views() {
        // Verifies AppKit controls connect native events to Store updates and actions.
        let mut composer = Composer::new(AppKitBackend::new());
        let value = composer.context(|cx| cx.store(String::new()));
        let changes = Rc::new(Cell::new(0));
        let received = changes.clone();
        composer.mount(View::new(
            LayoutStyle::stack(Axis::Vertical, 8.0),
            (
                TextField::new().bind(value),
                Button::new("Save").on_click(move |_| received.set(received.get() + 1)),
            ),
        ));
        let root = root_id(&composer);
        let field = composer.backend().children(root)[0];
        let button = composer.backend().children(root)[1];
        let field_route = composer.backend().event_route(field).unwrap();
        let button_route = composer.backend().event_route(button).unwrap();

        assert!(composer.dispatch_event(
            field_route,
            Event::TextChanged {
                value: "typed".into(),
            },
        ));
        assert!(composer.dispatch_event(button_route, Event::Activated));
        assert_eq!(composer.context(|cx| cx.get(value).clone()), "typed");
        assert_eq!(changes.get(), 1);
    }

    #[test]
    fn sidebar_binding_filters_unknown_native_ids() {
        // Verifies AppKit sidebar selection updates only for declared items.
        let mut composer = Composer::new(AppKitBackend::new());
        let selection = composer.context(|cx| cx.store(None::<String>));
        composer.mount(
            Window::new(Rect::new(0.0, 0.0, 640.0, 480.0), "Navigation").sidebar(
                Sidebar::new([SidebarSection::new([SidebarItem::new("home", "Home")])])
                    .bind(selection),
            ),
        );
        let root = root_id(&composer);
        let route = composer.backend().event_route(root).unwrap();

        assert!(
            composer.dispatch_event(route, Event::SidebarSelectionChanged { id: "home".into() },)
        );
        assert_eq!(
            composer.context(|cx| cx.get(selection).clone()),
            Some("home".into())
        );
        assert!(composer.dispatch_event(
            route,
            Event::SidebarSelectionChanged {
                id: "missing".into(),
            },
        ));
        assert_eq!(
            composer.context(|cx| cx.get(selection).clone()),
            Some("home".into())
        );
    }
}
