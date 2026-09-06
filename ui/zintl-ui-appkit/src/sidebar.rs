/// AppKit adapter data, independent of desktop views and reactive state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sidebar {
    pub sections: Vec<SidebarSection>,
    pub selected_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarSection {
    pub title: Option<String>,
    pub items: Vec<SidebarItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarItem {
    pub id: String,
    pub title: String,
    pub system_image: Option<String>,
}

#[cfg(target_os = "macos")]
impl Sidebar {
    pub(crate) fn native(&self) -> zpd_appkit::ui::Sidebar {
        zpd_appkit::ui::Sidebar {
            selected_id: self.selected_id.clone(),
            sections: self
                .sections
                .iter()
                .map(|section| zpd_appkit::ui::SidebarSection {
                    title: section.title.clone(),
                    items: section
                        .items
                        .iter()
                        .map(|item| zpd_appkit::ui::SidebarItem {
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
