use crate::model::{BoolStyle, Category, Group, Setting};

pub fn category() -> Category {
    Category::settings("appearance", "Appearance")
        .icon("applications-graphics-symbolic")
        .keywords(&["look", "style", "theme", "decorations"])
        .group(
            Group::untitled()
                .setting(
                    Setting::number("Gaps", "layout.gaps", |c| c.layout.gaps)
                        .range(0., 128., 1.)
                        .keywords(&["spacing", "margin", "padding"]),
                )
                .setting(
                    Setting::switch("App Title Bars", "prefer-no-csd", |c| c.prefer_no_csd)
                        .inverted()
                        .keywords(&["csd", "client-side decorations", "header bar"]),
                )
                .setting(
                    Setting::color("Background", "layout.background-color", |c| {
                        c.layout.background_color
                    })
                    .keywords(&["workspace", "backdrop", "color"]),
                ),
        )
        .group(
            Group::untitled()
                .setting(
                    Setting::switch("Focus Ring", "layout.focus-ring", |c| {
                        !c.layout.focus_ring.off
                    })
                    .style(BoolStyle::OnOff)
                    .keywords(&["outline", "highlight", "active window"])
                    .expands(vec![
                        Setting::number("Width", "layout.focus-ring.width", |c| {
                            c.layout.focus_ring.width
                        })
                        .range(0., 32., 1.),
                        Setting::color("Color", "layout.focus-ring.active-color", |c| {
                            c.layout.focus_ring.active_color
                        }),
                        Setting::color("Inactive Color", "layout.focus-ring.inactive-color", |c| {
                            c.layout.focus_ring.inactive_color
                        }),
                    ]),
                )
                .setting(
                    Setting::switch("Border", "layout.border", |c| !c.layout.border.off)
                        .style(BoolStyle::OnOff)
                        .keywords(&["outline", "frame"])
                        .expands(vec![
                            Setting::number("Width", "layout.border.width", |c| {
                                c.layout.border.width
                            })
                            .range(0., 32., 1.),
                            Setting::color("Color", "layout.border.active-color", |c| {
                                c.layout.border.active_color
                            }),
                            Setting::color("Inactive Color", "layout.border.inactive-color", |c| {
                                c.layout.border.inactive_color
                            }),
                        ]),
                )
                .setting(
                    Setting::switch("Shadow", "layout.shadow", |c| c.layout.shadow.on)
                        .style(BoolStyle::OnOff)
                        .keywords(&["drop shadow", "depth"])
                        .expands(vec![
                            Setting::number("Softness", "layout.shadow.softness", |c| {
                                c.layout.shadow.softness
                            })
                            .range(0., 256., 1.)
                            .keywords(&["blur"]),
                            Setting::number("Spread", "layout.shadow.spread", |c| {
                                c.layout.shadow.spread
                            })
                            .range(-256., 256., 1.),
                            Setting::color("Color", "layout.shadow.color", |c| {
                                c.layout.shadow.color
                            }),
                        ]),
                ),
        )
}
