use crate::model::{BoolStyle, Category, Group, Setting};

pub fn category() -> Category {
    Category::settings("appearance", "Appearance")
        .icon("ph-palette-symbolic")
        .keywords(&["look", "theme", "colors"])
        .group(
            Group::custom(super::theme::build)
                .search_item(
                    "appearance.style",
                    "Style",
                    &["dark mode", "light mode", "theme"],
                )
                .search_item(
                    "appearance.accent",
                    "Accent Color",
                    &["highlight", "color", "theme"],
                ),
        )
        .group(Group::custom(super::wallpaper::build).search_item(
            "appearance.wallpaper",
            "Wallpaper",
            &["background", "picture", "image", "desktop"],
        ))
        .group(
            Group::untitled()
                .setting(
                    Setting::switch("App Title Bars", "prefer-no-csd", |c| c.prefer_no_csd)
                        .inverted()
                        .keywords(&["csd", "client-side decorations", "header bar"]),
                )
                .setting(
                    Setting::color("Background Color", "layout.background-color", |c| {
                        c.layout.background_color
                    })
                    .keywords(&[
                        "wallpaper",
                        "workspace",
                        "backdrop",
                        "color",
                    ]),
                ),
        )
        .group(
            Group::new("Motion").setting(
                Setting::switch("Animations", "animations", |c| !c.animations.off)
                    .style(BoolStyle::OnOff)
                    .keywords(&["reduce motion", "transitions"])
                    .expands(vec![Setting::number(
                        "Slowdown",
                        "animations.slowdown",
                        |c| c.animations.slowdown,
                    )
                    .range(0.1, 4., 0.05)
                    .digits(2)
                    .slider()
                    .format(|x| format!("{x:.2}×"))
                    .keywords(&["speed", "duration", "slow", "fast"])]),
            ),
        )
}
