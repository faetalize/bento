//! Every page, in sidebar order. Add a page by writing a function that returns a `Category` and
//! listing it in a section here.

mod about;
mod appearance;
mod engine;
mod input;
mod motion;
mod planned;
mod system;

use crate::model::Section;

pub fn sections() -> Vec<Section> {
    vec![
        Section::new(
            "Look and Feel",
            vec![
                appearance::category(),
                motion::animations(),
                motion::overview(),
            ],
        ),
        Section::new(
            "Tiling",
            vec![
                engine::category(),
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
                input::cursor(),
                planned::gestures(),
                planned::shortcuts(),
            ],
        ),
        Section::new(
            "General",
            vec![
                planned::displays(),
                planned::startup(),
                system::category(),
                about::category(),
            ],
        ),
    ]
}
