use crate::model::{BoolStyle, Category, Group, Setting};

pub fn category() -> Category {
    Category::settings("appearance", "Appearance")
        .icon("ph-palette-symbolic")
        .keywords(&["look", "theme", "colors", "blur", "glass"])
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
            Group::custom(super::blur::build)
                .search_item(
                    "appearance.blur",
                    "Blur",
                    &[
                        "glass",
                        "frosted",
                        "transparency",
                        "translucency",
                        "transparent",
                        "translucent",
                        "background effect",
                    ],
                )
                .search_item(
                    "blur.material",
                    "Blur Material",
                    &["glass", "frosted", "transparency", "translucency"],
                )
                .search_item(
                    "blur.glass.preset",
                    "Glass Preset",
                    &["clear", "regular", "hazy", "style"],
                )
                .search_item(
                    "blur.roughness",
                    "Blur Roughness",
                    &["amount", "radius", "frost", "transparency", "translucency"],
                )
                .search_item("blur.offset", "Blur Strength", &["radius", "offset"])
                .search_item("blur.passes", "Blur Passes", &["quality"])
                .search_item("blur.noise", "Blur Noise", &["grain", "dither"])
                .search_item("blur.saturation", "Blur Saturation", &["vibrancy", "color"])
                .search_item(
                    "blur.glass.sheen",
                    "Glass Sheen",
                    &["gloss", "polish", "reflection"],
                )
                .search_item("blur.glass.bezel", "Glass Bezel", &["edge", "curve"])
                .search_item(
                    "blur.glass.thickness",
                    "Glass Thickness",
                    &["height", "depth"],
                )
                .search_item(
                    "blur.glass.refractive-index",
                    "Glass Refractive Index",
                    &["refraction", "bend", "lens"],
                )
                .search_item(
                    "blur.glass.dispersion",
                    "Glass Dispersion",
                    &["chromatic aberration", "rainbow", "prism"],
                )
                .search_item(
                    "blur.glass.highlight",
                    "Glass Highlight",
                    &["glare", "edge light", "shine"],
                )
                .search_item(
                    "blur.glass.light-size",
                    "Glass Light Size",
                    &["light", "softness", "highlight"],
                )
                .search_item(
                    "blur.glass.light-angle",
                    "Glass Light Angle",
                    &["light direction"],
                ),
        )
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
