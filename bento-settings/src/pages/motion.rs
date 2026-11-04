use crate::model::{BoolStyle, Category, Group, Setting};

pub fn animations() -> Category {
    Category::settings("animations", "Animations")
        .icon("media-playback-start-symbolic")
        .description("Motion when windows open, close, move and when the view scrolls.")
        .keywords(&["motion", "transitions", "speed", "reduce motion"])
        .group(
            Group::new("Animations")
                .setting(
                    Setting::switch("Animations", "animations", |c| !c.animations.off)
                        .style(BoolStyle::OnOff)
                        .keywords(&["reduce motion", "disable"]),
                )
                .setting(
                    Setting::number("Slowdown", "animations.slowdown", |c| c.animations.slowdown)
                        .subtitle("Multiplies every animation's length: 2 is half as fast")
                        .range(0.1, 10., 0.1)
                        .digits(1)
                        .enabled_if(|c| !c.animations.off)
                        .keywords(&["speed", "duration", "slow", "fast"]),
                ),
        )
}

pub fn overview() -> Category {
    Category::settings("overview", "Overview")
        .icon("view-grid-symbolic")
        .description("The zoomed-out view of every workspace.")
        .keywords(&["expose", "mission control", "workspaces"])
        .group(
            Group::new("Overview")
                .setting(
                    Setting::number("Zoom", "overview.zoom", |c| c.overview.zoom)
                        .subtitle("How big workspaces are in the overview")
                        .range(0.05, 0.75, 0.05)
                        .digits(2)
                        .keywords(&["scale", "size"]),
                )
                .setting(
                    Setting::color("Backdrop", "overview.backdrop-color", |c| {
                        c.overview.backdrop_color
                    })
                    .subtitle("Shown behind the workspaces")
                    .keywords(&["background", "color"]),
                ),
        )
}
