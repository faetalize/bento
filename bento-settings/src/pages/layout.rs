use niri_config::{Config, WindowRule};

use crate::kdl::Value;
use crate::model::{BoolStyle, Category, Group, Setting};

/// The value from the last window rule that matches every window. That's what windows get unless
/// a rule for specific apps says otherwise.
pub(super) fn every_window<T>(
    config: &Config,
    get: impl Fn(&WindowRule) -> Option<T>,
) -> Option<T> {
    config
        .window_rules
        .iter()
        .rev()
        .filter(|rule| rule.matches.is_empty() && rule.excludes.is_empty())
        .find_map(get)
}

/// Windows' rounded corners: their content cut to the radius, which the focus ring, border and
/// shadow follow too.
fn rounded_corners() -> Setting {
    Setting::switch("Rounded Corners", "window-rule.clip-to-geometry", |c| {
        every_window(c, |rule| rule.clip_to_geometry).unwrap_or(false)
    })
    .style(BoolStyle::Arg)
    .along("window-rule.geometry-corner-radius", Value::Int(12))
    .keywords(&["radius", "corner radius", "clip to geometry", "round"])
    .expands(vec![Setting::number(
        "Radius",
        "window-rule.geometry-corner-radius",
        |c| {
            every_window(c, |rule| rule.geometry_corner_radius)
                .map_or(0., |radius| f64::from(radius.top_left))
        },
    )
    .range(0., 64., 1.)
    .keywords(&["corner radius", "geometry corner radius"])])
}

pub fn category() -> Category {
    Category::settings("layout", "Layout")
        .icon("ph-squares-four-symbolic")
        .keywords(&["windows", "tiling", "spacing", "decorations"])
        .group(Group::custom(super::preview::build).search_item(
            "layout.preview",
            "Preview",
            &["example"],
        ))
        .group(
            Group::untitled().setting(
                Setting::number("Gaps", "layout.gaps", |c| c.layout.gaps)
                    .range(0., 128., 1.)
                    .keywords(&["spacing", "margin", "padding"]),
            ),
        )
        .group(
            Group::untitled()
                .setting(rounded_corners())
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
                            .range(0., 128., 1.)
                            .slider()
                            .format(|x| format!("{x:.0} px"))
                            .keywords(&["blur"]),
                            Setting::number("Spread", "layout.shadow.spread", |c| {
                                c.layout.shadow.spread
                            })
                            .range(-64., 64., 1.)
                            .slider()
                            .format(|x| format!("{x:.0} px")),
                            Setting::color("Color", "layout.shadow.color", |c| {
                                c.layout.shadow.color
                            }),
                        ]),
                ),
        )
        .group(
            Group::new("Reserved Space")
                .setting(
                    Setting::number("Top", "layout.struts.top", |c| c.layout.struts.top.0)
                        .range(0., 512., 1.)
                        .keywords(&["struts", "edge", "margin"]),
                )
                .setting(
                    Setting::number("Bottom", "layout.struts.bottom", |c| {
                        c.layout.struts.bottom.0
                    })
                    .range(0., 512., 1.)
                    .keywords(&["struts", "edge", "margin"]),
                )
                .setting(
                    Setting::number("Left", "layout.struts.left", |c| c.layout.struts.left.0)
                        .range(0., 512., 1.)
                        .keywords(&["struts", "edge", "margin"]),
                )
                .setting(
                    Setting::number("Right", "layout.struts.right", |c| c.layout.struts.right.0)
                        .range(0., 512., 1.)
                        .keywords(&["struts", "edge", "margin"]),
                ),
        )
        .group(
            Group::new("Overview")
                .setting(
                    Setting::number("Workspace Size", "overview.zoom", |c| c.overview.zoom)
                        .range(0.1, 0.75, 0.05)
                        .digits(2)
                        .slider()
                        .format(|x| format!("{:.0}%", x * 100.))
                        .keywords(&["zoom", "expose", "mission control", "scale"]),
                )
                .setting(
                    Setting::color("Backdrop", "overview.backdrop-color", |c| {
                        c.overview.backdrop_color
                    })
                    .keywords(&["expose", "background", "color"]),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kdl::Document;
    use crate::model::Input;

    #[test]
    fn rounded_corners_off_leaves_no_corner_rounded_and_on_rounds_them() {
        let rounded =
            "window-rule {\n    geometry-corner-radius 12\n    clip-to-geometry true\n}\n";
        let mut doc = Document::parse(String::from(rounded)).unwrap();
        rounded_corners().write(&mut doc, Input::Bool(false));
        assert!(
            !doc.text().contains("geometry-corner-radius"),
            "{}",
            doc.text()
        );

        rounded_corners().write(&mut doc, Input::Bool(true));
        let text = doc.text();
        assert!(text.contains("clip-to-geometry true"), "{text}");
        assert!(text.contains("geometry-corner-radius 12"), "{text}");
    }
}
