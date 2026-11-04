use crate::model::{BoolStyle, Category, Group, Setting};

pub fn category() -> Category {
    Category::settings("appearance", "Appearance")
        .icon("applications-graphics-symbolic")
        .description("How windows look: the space around them, and the rings, borders and shadows bento draws.")
        .keywords(&["look", "style", "theme", "decorations"])
        .group(
            Group::new("Windows")
                .setting(
                    Setting::number("Gaps", "layout.gaps", |c| c.layout.gaps)
                        .subtitle("Space between windows and around the edges, in logical pixels")
                        .range(0., 128., 1.)
                        .keywords(&["spacing", "margin", "padding"]),
                )
                .setting(
                    Setting::switch("Hide app title bars", "prefer-no-csd", |c| c.prefer_no_csd)
                        .subtitle("Ask apps to leave out their own title bars and shadows")
                        .keywords(&["csd", "decorations", "header bar"]),
                ),
        )
        .group(
            Group::new("Focus ring")
                .description("Drawn around the focused window.")
                .setting(
                    Setting::switch("Focus ring", "layout.focus-ring", |c| !c.layout.focus_ring.off)
                        .style(BoolStyle::OnOff)
                        .keywords(&["outline", "highlight", "active"]),
                )
                .setting(
                    Setting::number("Width", "layout.focus-ring.width", |c| c.layout.focus_ring.width)
                        .range(0., 32., 1.)
                        .enabled_if(|c| !c.layout.focus_ring.off)
                        .keywords(&["thickness", "size"]),
                )
                .setting(
                    Setting::color("Focused window", "layout.focus-ring.active-color", |c| c.layout.focus_ring.active_color)
                        .enabled_if(|c| !c.layout.focus_ring.off)
                        .keywords(&["color", "active"]),
                )
                .setting(
                    Setting::color("Other windows", "layout.focus-ring.inactive-color", |c| c.layout.focus_ring.inactive_color)
                        .subtitle("On monitors other than the focused one")
                        .enabled_if(|c| !c.layout.focus_ring.off)
                        .keywords(&["color", "inactive"]),
                ),
        )
        .group(
            Group::new("Border")
                .description("Drawn around every window, taking space from it.")
                .setting(
                    Setting::switch("Border", "layout.border", |c| !c.layout.border.off)
                        .style(BoolStyle::OnOff)
                        .keywords(&["outline", "frame"]),
                )
                .setting(
                    Setting::number("Width", "layout.border.width", |c| c.layout.border.width)
                        .range(0., 32., 1.)
                        .enabled_if(|c| !c.layout.border.off)
                        .keywords(&["thickness", "size"]),
                )
                .setting(
                    Setting::color("Focused window", "layout.border.active-color", |c| c.layout.border.active_color)
                        .enabled_if(|c| !c.layout.border.off)
                        .keywords(&["color", "active"]),
                )
                .setting(
                    Setting::color("Other windows", "layout.border.inactive-color", |c| c.layout.border.inactive_color)
                        .enabled_if(|c| !c.layout.border.off)
                        .keywords(&["color", "inactive"]),
                ),
        )
        .group(
            Group::new("Shadows")
                .setting(
                    Setting::switch("Shadows", "layout.shadow", |c| c.layout.shadow.on)
                        .style(BoolStyle::OnOff)
                        .keywords(&["drop shadow", "depth"]),
                )
                .setting(
                    Setting::number("Softness", "layout.shadow.softness", |c| c.layout.shadow.softness)
                        .subtitle("Blur radius, in logical pixels")
                        .range(0., 256., 1.)
                        .enabled_if(|c| c.layout.shadow.on)
                        .keywords(&["blur", "radius"]),
                )
                .setting(
                    Setting::number("Spread", "layout.shadow.spread", |c| c.layout.shadow.spread)
                        .subtitle("How far the shadow grows past the window")
                        .range(-256., 256., 1.)
                        .enabled_if(|c| c.layout.shadow.on)
                        .keywords(&["size"]),
                )
                .setting(
                    Setting::switch("Draw behind windows", "layout.shadow.draw-behind-window", |c| c.layout.shadow.draw_behind_window)
                        .style(BoolStyle::Arg)
                        .subtitle("For windows with rounded corners that bento doesn't know about")
                        .enabled_if(|c| c.layout.shadow.on),
                )
                .setting(
                    Setting::color("Color", "layout.shadow.color", |c| c.layout.shadow.color)
                        .enabled_if(|c| c.layout.shadow.on),
                ),
        )
        .group(
            Group::new("Background").setting(
                Setting::color("Workspace background", "layout.background-color", |c| c.layout.background_color)
                    .subtitle("Shown where no window or wallpaper covers the workspace")
                    .keywords(&["wallpaper", "backdrop"]),
            ),
        )
}
