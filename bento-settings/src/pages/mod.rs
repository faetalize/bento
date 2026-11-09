//! Every page, in sidebar order. Add a page by writing a function that returns a `Category` and
//! listing it in a section here.

mod about;
mod account;
mod appearance;
mod blur;
mod blur_live;
mod blur_preview;
mod control_center;
mod engine;
mod input;
mod layout;
mod layouts;
mod location;
mod migrate;
mod planned;
mod preview;
pub mod shortcuts;
mod system;
mod theme;
mod wallpaper;

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
                shortcuts::category(),
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
