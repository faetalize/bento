use crate::model::{BoolStyle, Category, Group, Setting};

pub fn animations() -> Category {
    Category::settings("animations", "Animations")
        .icon("media-playback-start-symbolic")
        .keywords(&["motion", "transitions", "reduce motion"])
        .group(
            Group::untitled().setting(
                Setting::switch("Animations", "animations", |c| !c.animations.off)
                    .style(BoolStyle::OnOff)
                    .keywords(&["reduce motion"])
                    .expands(vec![Setting::number(
                        "Slowdown",
                        "animations.slowdown",
                        |c| c.animations.slowdown,
                    )
                    .range(0.1, 10., 0.1)
                    .digits(1)
                    .keywords(&["speed", "duration", "slow", "fast"])]),
            ),
        )
}

pub fn overview() -> Category {
    Category::settings("overview", "Overview")
        .icon("view-grid-symbolic")
        .keywords(&["expose", "mission control", "workspaces"])
        .group(
            Group::untitled()
                .setting(
                    Setting::number("Zoom", "overview.zoom", |c| c.overview.zoom)
                        .range(0.05, 0.75, 0.05)
                        .digits(2)
                        .keywords(&["scale", "size"]),
                )
                .setting(
                    Setting::color("Backdrop", "overview.backdrop-color", |c| {
                        c.overview.backdrop_color
                    })
                    .keywords(&["background", "color"]),
                ),
        )
}
