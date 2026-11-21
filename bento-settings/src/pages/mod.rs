//! Every page, in sidebar order. Add a page by writing a function that returns a `Category` and
//! listing it in a section here.

mod about;
mod account;
mod appearance;
mod bind_list;
pub mod binds;
mod blur;
mod blur_live;
mod blur_preview;
mod chord_line;
mod control_center;
mod declared;
mod engine;
mod input;
mod layout;
mod layouts;
mod location;
mod migrate;
mod planned;
mod preview;
mod settings_file;
mod shell;
mod system;
mod theme;
mod title_buttons;
mod wallpaper;

pub use declared::placed;

use crate::model::{Content, Section};

pub fn sections() -> Vec<Section> {
    let sections = vec![
        Section::untitled(vec![account::category()]),
        Section::new(
            "Desktop",
            vec![
                appearance::category(),
                layout::category(),
                engine::category(),
                control_center::category(),
                planned::workspaces(),
                planned::window_rules(),
            ],
        ),
        Section::new(
            "Input",
            vec![
                input::keyboard(),
                input::mouse(),
                input::touchpad(),
                planned::gestures(),
                binds::category(),
            ],
        ),
        Section::new(
            "General",
            vec![
                planned::displays(),
                planned::startup(),
                location::category(),
                system::category(),
                about::category(),
            ],
        ),
    ];

    // Pages that aren't ready stay out of the sidebar and search until they work.
    sections
        .into_iter()
        .map(|mut section| {
            section
                .categories
                .retain(|category| !matches!(category.content, Content::Planned));
            section
        })
        .filter(|section| !section.categories.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_a_setting_can_name_is_a_page_of_the_app() {
        let ids: Vec<&str> = sections()
            .iter()
            .flat_map(|section| section.categories.iter().map(|category| category.id))
            .collect();
        for page in niri_config::SETTINGS_PAGES {
            assert!(ids.contains(page), "the app has no page {page:?}");
        }
    }
}
