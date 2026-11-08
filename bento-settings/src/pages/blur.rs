//! Blur: the material bento draws behind windows and panels that ask for it, glass or plain,
//! with a preview of it over the wallpaper.
//!
//! Two sliders set several settings at once: Roughness sets how far the blur reaches (passes and
//! strength) and how much glass spreads the light it reflects (sheen), and Preset sets a whole
//! look. Each reads Custom, without its knob, while the settings it sets don't match any of its
//! positions, as after changing one of them under Advanced.
//!
//! Glass has a list of up to four lights. A config can't leave it empty, as glass without lights
//! has its default one, so removing the last light leaves one switched off, which reads as No
//! Lights; a light on its own has no switch, so switching it off doesn't turn it into No Lights
//! under the pointer.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::LazyLock;

use adw::prelude::*;
use niri_config::{Blur, Config, Glass, GlassLight, GlassPattern, Material, MAX_GLASS_LIGHTS};

use super::blur_live::{Live, Unsaved};
use crate::kdl::{Document, Value};
use crate::model::{BoolStyle, Control, Input, Setting};
use crate::page::{setting_row_watched, PageContext};

const MATERIAL_KEY: &str = "blur.material";
/// The store's key for Roughness's edits, which set passes, offset and sheen together.
const ROUGHNESS_KEY: &str = "blur.roughness";
/// The store's key for the Preset slider's edits, which set a whole look for glass.
const PRESET_KEY: &str = "blur.glass.preset";
/// The store's key for adding or removing lights, under which each such edit gets a key of its
/// own, since each moves the lights after it.
const LIGHTS_KEY: &str = "blur.glass.lights";
const MATERIAL_PATH: &[&str] = &["blur", "material"];
const LIGHTS_PATH: &[&str] = &["blur", "glass", "light"];

/// What resetting the advanced settings removes from the config.
const ADVANCED_PATHS: &[&[&str]] = &[
    &["blur", "passes"],
    &["blur", "offset"],
    &["blur", "noise"],
    &["blur", "saturation"],
    &["blur", "glass"],
];

static SWITCH: LazyLock<Setting> = LazyLock::new(|| {
    Setting::switch("Blur", "blur", |c| !c.blur.off)
        .style(BoolStyle::OnOff)
        .keywords(&["transparency", "translucency", "background effect"])
});

static PASSES: LazyLock<Setting> = LazyLock::new(|| {
    Setting::number("Passes", "blur.passes", |c| f64::from(c.blur.passes))
        .range(1., 8., 1.)
        .slider()
        .format(|x| format!("{x:.0}"))
        .keywords(&["quality", "iterations"])
});

static STRENGTH: LazyLock<Setting> = LazyLock::new(|| {
    Setting::number("Strength", "blur.offset", |c| c.blur.offset)
        .range(0., 10., 0.01)
        .digits(2)
        .slider()
        .format(|x| format!("{x:.2}"))
        .keywords(&["radius", "offset"])
});

static SHEEN: LazyLock<Setting> = LazyLock::new(|| {
    Setting::number("Sheen", "blur.glass.sheen", |c| c.blur.glass.sheen)
        .range(0., 1., 0.01)
        .digits(2)
        .slider()
        .format(|x| format!("{:.0} %", x * 100.))
        .keywords(&["gloss", "polish", "reflection"])
});

static NOISE: LazyLock<Setting> = LazyLock::new(|| {
    Setting::number("Noise", "blur.noise", |c| c.blur.noise)
        .range(0., 0.2, 0.005)
        .digits(3)
        .slider()
        .format(|x| format!("{x:.3}"))
        .keywords(&["grain", "dither"])
});

static SATURATION: LazyLock<Setting> = LazyLock::new(|| {
    Setting::number("Saturation", "blur.saturation", |c| c.blur.saturation)
        .range(0., 3., 0.01)
        .digits(2)
        .slider()
        .format(|x| format!("{:.0} %", x * 100.))
        .keywords(&["vibrancy", "color"])
});

static PATTERN: LazyLock<Setting> = LazyLock::new(|| {
    Setting::choice("Pattern", "blur.glass.pattern", |c| {
        Some(pattern_name(c.blur.glass.pattern))
    })
    .option(Some("smooth"), "Smooth")
    .option(Some("reeded"), "Reeded")
    .option(Some("hammered"), "Hammered")
    .option(Some("frosted"), "Frosted")
    .keywords(&["texture", "surface", "fluted", "ribbed", "dimpled"])
});

static COLOR: LazyLock<Setting> = LazyLock::new(|| {
    Setting::color("Color", "blur.glass.color", |c| c.blur.glass.color)
        .keywords(&["tint", "hue", "stained", "absorption"])
});

static FOLLOW_ACCENT: LazyLock<Setting> = LazyLock::new(|| {
    Setting::switch("Follow Accent Color", "blur.glass.follow-accent", |c| {
        c.blur.glass.follow_accent
    })
    .style(BoolStyle::Flag)
    .keywords(&["tint", "theme", "hue"])
});

static PATTERN_DEPTH: LazyLock<Setting> = LazyLock::new(|| {
    Setting::number("Pattern Depth", "blur.glass.pattern-depth", |c| {
        c.blur.glass.pattern_depth
    })
    .range(0., 1., 0.01)
    .digits(2)
    .slider()
    .format(percent)
    .keywords(&["texture", "relief"])
});

static POINTER_LIGHT: LazyLock<Setting> = LazyLock::new(|| {
    Setting::number("Pointer Light", "blur.glass.pointer-light", |c| {
        c.blur.glass.pointer_light
    })
    .range(0., 2., 0.01)
    .digits(2)
    .slider()
    .format(percent)
    .keywords(&["cursor", "mouse", "glint", "hover"])
});

fn pattern_name(pattern: GlassPattern) -> &'static str {
    match pattern {
        GlassPattern::Smooth => "smooth",
        GlassPattern::Reeded => "reeded",
        GlassPattern::Hammered => "hammered",
        GlassPattern::Frosted => "frosted",
    }
}

fn pattern_named(name: &str) -> GlassPattern {
    match name {
        "reeded" => GlassPattern::Reeded,
        "hammered" => GlassPattern::Hammered,
        "frosted" => GlassPattern::Frosted,
        _ => GlassPattern::Smooth,
    }
}

/// Glass's own settings, which neither Roughness nor Preset sets.
static GLASS: LazyLock<Vec<Setting>> = LazyLock::new(|| {
    vec![
        Setting::number("Bezel", "blur.glass.bezel", |c| c.blur.glass.bezel)
            .range(0., 64., 1.)
            .slider()
            .format(|x| format!("{x:.0} px"))
            .keywords(&["edge", "curve"]),
        Setting::number("Thickness", "blur.glass.thickness", |c| {
            c.blur.glass.thickness
        })
        .range(0., 160., 1.)
        .slider()
        .format(|x| format!("{x:.0} px"))
        .keywords(&["height", "depth"]),
        Setting::number("Refractive Index", "blur.glass.refractive-index", |c| {
            c.blur.glass.refractive_index
        })
        .range(1., 3., 0.01)
        .digits(2)
        .slider()
        .format(|x| format!("{x:.2}"))
        .keywords(&["refraction", "bend", "lens"]),
        Setting::number("Dispersion", "blur.glass.dispersion", |c| {
            c.blur.glass.dispersion
        })
        .range(0., 1., 0.01)
        .digits(2)
        .slider()
        .format(|x| format!("{x:.2}"))
        .keywords(&["chromatic aberration", "rainbow", "prism"]),
        Setting::number("Reflection", "blur.glass.reflection", |c| {
            c.blur.glass.reflection
        })
        .range(0., 1., 0.01)
        .digits(2)
        .slider()
        .format(|x| format!("{:.0} %", x * 100.))
        .keywords(&["fresnel", "rim", "edge", "glow"]),
    ]
});

/// A light's settings, at its place in glass's list.
struct LightSettings {
    off: Setting,
    intensity: Setting,
    angle: Setting,
    height: Setting,
    size: Setting,
}

/// The light at `i`, or the default one where there's none.
fn light(c: &Config, i: usize) -> GlassLight {
    c.blur.glass.lights[i].unwrap_or_default()
}

macro_rules! light_settings {
    ($i:literal) => {
        LightSettings {
            off: Setting::switch("Light", concat!("blur.glass.light.", $i, ".off"), |c| {
                !light(c, $i).on
            })
            .style(BoolStyle::Presence)
            .inverted(),
            intensity: Setting::number(
                "Intensity",
                concat!("blur.glass.light.", $i, ".intensity"),
                |c| light(c, $i).intensity,
            )
            .range(0., 2., 0.01)
            .digits(2)
            .slider()
            .format(percent)
            .keywords(&["brightness", "highlight", "glare"]),
            angle: Setting::number("Angle", concat!("blur.glass.light.", $i, ".angle"), |c| {
                light(c, $i).angle
            })
            .range(0., 360., 1.)
            .slider()
            .format(|x| format!("{x:.0}°"))
            .keywords(&["direction"]),
            height: Setting::number("Height", concat!("blur.glass.light.", $i, ".height"), |c| {
                light(c, $i).height
            })
            .range(0., 90., 1.)
            .slider()
            .format(|x| format!("{x:.0}°"))
            .keywords(&["elevation"]),
            size: Setting::number("Size", concat!("blur.glass.light.", $i, ".size"), |c| {
                light(c, $i).size
            })
            .range(0., 1., 0.01)
            .digits(2)
            .slider()
            .format(percent)
            .keywords(&["softness"]),
        }
    };
}

static LIGHTS: LazyLock<[LightSettings; MAX_GLASS_LIGHTS]> = LazyLock::new(|| {
    [
        light_settings!(0),
        light_settings!(1),
        light_settings!(2),
        light_settings!(3),
    ]
});

/// Whether glass has no lights: only one, switched off, as removing the last one leaves it.
fn no_lights(glass: &Glass) -> bool {
    let [first, rest @ ..] = &glass.lights;
    first.is_some_and(|light| !light.on) && rest.iter().all(Option::is_none)
}

/// Glass's lights, in order.
fn lights(glass: &Glass) -> Vec<GlassLight> {
    if no_lights(glass) {
        return Vec::new();
    }
    glass.lights.iter().flatten().copied().collect()
}

/// `lights` as glass has them, with no lights as one switched off.
fn light_list(lights: &[GlassLight]) -> [Option<GlassLight>; MAX_GLASS_LIGHTS] {
    let mut list = [None; MAX_GLASS_LIGHTS];
    if lights.is_empty() {
        list[0] = Some(GlassLight {
            on: false,
            ..GlassLight::default()
        });
    }
    for (slot, light) in list.iter_mut().zip(lights) {
        *slot = Some(*light);
    }
    list
}

/// An angle for a new light: the middle of the widest gap between the others, in whole degrees.
fn free_angle(lights: &[GlassLight]) -> f64 {
    let mut angles: Vec<f64> = lights.iter().map(|l| l.angle.rem_euclid(360.)).collect();
    angles.sort_by(f64::total_cmp);
    let Some(&first) = angles.first() else {
        return GlassLight::default().angle;
    };
    // Each angle with the gap after it, the last one's round to the first.
    let next = angles.iter().skip(1).copied().chain([first + 360.]);
    let (start, gap) = angles
        .iter()
        .zip(next)
        .map(|(a, b)| (*a, b - a))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .expect("there's an angle");
    (start + gap / 2.).round().rem_euclid(360.)
}

/// The settings Roughness sets, at a roughness from 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rough {
    passes: f64,
    offset: f64,
    sheen: f64,
}

/// How far the blur reaches, about `offset · 2^passes`, at a roughness: from nothing at 0, where
/// glass is clear, to 255 at 1, nearly evenly on a log scale.
fn reach(roughness: f64) -> f64 {
    256f64.powf(roughness) - 1.
}

/// The settings at `roughness`, taken to the hundredth, the slider's step: a drag stops between
/// steps, and only a hundredth's settings read back as that roughness. The passes are the ones
/// that put the offset nearest 2, so it stays between about 1.4 and 2.8: further apart, a pass's
/// samples skip what's between them, which shows as ghosting. The offset and sheen are in
/// hundredths, as they're saved. A rough surface isn't shiny, so sheen falls as roughness rises.
fn rough_at(roughness: f64) -> Rough {
    let roughness = hundredths(roughness);
    let reach = reach(roughness);
    let passes = (reach / 2.).log2().round().clamp(1., 8.);
    Rough {
        passes,
        offset: hundredths(reach / passes.exp2()),
        sheen: hundredths(1. - roughness),
    }
}

/// The roughness `blur`'s passes and offset reach as far as.
fn roughness_of(blur: &Blur) -> f64 {
    let reach = blur.offset * f64::from(blur.passes).exp2();
    ((reach + 1.).ln() / 256f64.ln()).clamp(0., 1.)
}

/// Where Roughness stands for `blur`, or `None` if its settings aren't those of any roughness.
/// Sheen only counts for glass.
fn roughness(blur: &Blur) -> Option<f64> {
    let roughness = hundredths(roughness_of(blur));
    let at = rough_at(roughness);
    let sheen_matches = blur.material == Material::Plain || close(blur.glass.sheen, at.sheen);
    (f64::from(blur.passes) == at.passes && close(blur.offset, at.offset) && sheen_matches)
        .then_some(roughness)
}

fn hundredths(x: f64) -> f64 {
    (x * 100.).round() / 100.
}

fn thousandths(x: f64) -> f64 {
    (x * 1000.).round() / 1000.
}

/// Whether two saved values are the same, given they're saved in hundredths at most.
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.006
}

/// A look for glass, from its Preset slider. Its light size is the first light's.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Preset {
    roughness: f64,
    saturation: f64,
    noise: f64,
    light_size: f64,
}

/// Clear, Regular and Hazy, at 0, 1 and 2 on the Preset slider: clear glass doesn't blur at all,
/// and above Hazy, Roughness has room for heavier blur still. Regular is bento's defaults.
const PRESETS: [(&str, Preset); 3] = [
    (
        "Clear",
        Preset {
            roughness: 0.,
            saturation: 1.,
            noise: 0.,
            light_size: 0.15,
        },
    ),
    (
        "Regular",
        Preset {
            roughness: 0.4,
            saturation: 1.15,
            noise: 0.01,
            light_size: 0.35,
        },
    ),
    (
        "Hazy",
        Preset {
            roughness: 0.8,
            saturation: 1.3,
            noise: 0.03,
            light_size: 0.6,
        },
    ),
];

/// The look at `position` on the Preset slider, between the two presets around it.
fn preset_at(position: f64) -> Preset {
    let last = (PRESETS.len() - 1) as f64;
    let position = position.clamp(0., last);
    let i = (position.floor() as usize).min(PRESETS.len() - 2);
    let t = position - i as f64;
    let (a, b) = (PRESETS[i].1, PRESETS[i + 1].1);
    let lerp = |a: f64, b: f64| a + (b - a) * t;
    Preset {
        roughness: lerp(a.roughness, b.roughness),
        saturation: lerp(a.saturation, b.saturation),
        noise: lerp(a.noise, b.noise),
        light_size: lerp(a.light_size, b.light_size),
    }
}

/// Where Preset stands for `blur`, or `None` if its settings aren't those of any position.
fn preset_position(blur: &Blur) -> Option<f64> {
    let roughness = roughness(blur)?;
    let first = PRESETS[0].1.roughness;
    let step = PRESETS[1].1.roughness - first;
    let position = (roughness - first) / step;
    if !(0. ..=(PRESETS.len() - 1) as f64 + 1e-9).contains(&position) {
        return None;
    }
    let at = preset_at(position);
    (close(blur.saturation, hundredths(at.saturation))
        && (blur.noise - thousandths(at.noise)).abs() < 0.0006
        && lights(&blur.glass)
            .first()
            .is_none_or(|light| close(light.size, hundredths(at.light_size))))
    .then_some(position)
}

/// The Preset position nearest `position` whose roughness Roughness can stand at, in hundredths,
/// so moving Preset never leaves Roughness reading Custom.
fn snap_preset(position: f64) -> f64 {
    let first = PRESETS[0].1.roughness;
    let step = PRESETS[1].1.roughness - first;
    (hundredths(first + position * step) - first) / step
}

/// The name of the preset nearest `position` on the Preset slider.
fn nearest_preset(position: f64) -> &'static str {
    let i = position.round().clamp(0., (PRESETS.len() - 1) as f64) as usize;
    PRESETS[i].0
}

/// A slider row of the page's own, like a setting's, with the label for its value.
fn slider_row(
    title: &str,
    min: f64,
    max: f64,
    step: f64,
) -> (adw::ActionRow, gtk::Scale, gtk::Label) {
    let (scale, value) = slider(min, max, step);
    let row = adw::ActionRow::builder().title(title).build();
    row.add_suffix(&value);
    row.add_suffix(&scale);
    (row, scale, value)
}

/// A slider and the label for its value, for a row's end.
fn slider(min: f64, max: f64, step: f64) -> (gtk::Scale, gtk::Label) {
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, step);
    scale.set_draw_value(false);
    scale.set_hexpand(true);
    scale.set_valign(gtk::Align::Center);
    scale.set_width_request(200);
    let value = gtk::Label::builder()
        .xalign(1.)
        .width_chars(8)
        .css_classes(["dim-label", "numeric"])
        .build();
    (scale, value)
}

pub fn build(ctx: &PageContext) -> gtk::Widget {
    let group = adw::PreferencesGroup::builder().title("Blur").build();
    ctx.anchor("appearance.blur", &group);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let live = Live::new(&ctx.store);
    let light_settings: &'static [LightSettings; MAX_GLASS_LIGHTS] = &LIGHTS;
    let angles = light_settings.each_ref().map(|light| &light.angle);
    content.append(&super::blur_preview::build(ctx, &live, angles));
    // Each control shows its value in the preview, and in the other controls, as it moves; the
    // config saves it after.
    let row = |setting: &'static Setting| number_row(setting, ctx, &live);
    // A setting Roughness sets too.
    let part = |setting: &'static Setting| number_row(setting, ctx, &live);
    let switch = {
        let live = live.clone();
        setting_row_watched(&SWITCH, ctx, move |input| {
            if let Input::Bool(on) = input {
                live.show(Unsaved::Setting(SWITCH.path, f64::from(u8::from(*on))));
            }
        })
    };

    let material = adw::ToggleGroup::builder()
        .halign(gtk::Align::Center)
        .width_request(240)
        .homogeneous(true)
        .build();
    for (name, label) in [("glass", "Glass"), ("plain", "Plain")] {
        material.add(adw::Toggle::builder().name(name).label(label).build());
    }
    ctx.anchor(MATERIAL_KEY, &material);
    content.append(&material);

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .margin_top(6)
        .css_classes(["boxed-list"])
        .build();
    list.append(&switch);

    // Its marks are bare, and its value label names the nearest preset, so its track lines up
    // with the other sliders'.
    let (preset_row, preset_scale, preset_value) =
        slider_row("Preset", 0., (PRESETS.len() - 1) as f64, 0.01);
    for i in 0..PRESETS.len() {
        preset_scale.add_mark(i as f64, gtk::PositionType::Bottom, None);
    }
    ctx.anchor(PRESET_KEY, &preset_row);
    list.append(&preset_row);
    let pattern = {
        let live = live.clone();
        setting_row_watched(&PATTERN, ctx, move |input| {
            if let Input::Choice(Some(name)) = input {
                live.show(Unsaved::Pattern(pattern_named(name)));
            }
        })
    };
    list.append(&pattern);
    let color = {
        let live = live.clone();
        setting_row_watched(&COLOR, ctx, move |input| {
            if let Input::Color(color) = input {
                live.show(Unsaved::Color(*color));
            }
        })
    };
    list.append(&color);
    let follow_accent = {
        let live = live.clone();
        setting_row_watched(&FOLLOW_ACCENT, ctx, move |input| {
            if let Input::Bool(on) = input {
                live.show(Unsaved::Setting(FOLLOW_ACCENT.path, f64::from(u8::from(*on))));
            }
        })
    };
    list.append(&follow_accent);
    // Glass that follows the accent doesn't have a colour of its own.
    let follow_color = {
        let color = color.downgrade();
        move |live: &Live, _: Option<&str>| {
            if let Some(color) = color.upgrade() {
                color.set_sensitive(!live.blur().glass.follow_accent);
            }
        }
    };
    follow_color(&live, None);
    live.connect(follow_color);

    let advanced = adw::ExpanderRow::builder().title("Advanced").build();
    // Roughness holds the settings it sets, so expanding it shows them under it.
    let (roughness_scale, roughness_value) = slider(0., 1., 0.01);
    let roughness_row = adw::ExpanderRow::builder().title("Roughness").build();
    // An expander row puts each suffix before the last, so the value goes in after its slider.
    roughness_row.add_suffix(&roughness_scale);
    roughness_row.add_suffix(&roughness_value);
    ctx.anchor(ROUGHNESS_KEY, &roughness_row);
    advanced.add_row(&roughness_row);
    let passes = part(&PASSES);
    let strength = part(&STRENGTH);
    let sheen = part(&SHEEN);
    for part in [&passes, &strength, &sheen] {
        part.add_css_class("part");
        roughness_row.add_row(part);
    }
    // Roughness is glass's, which it sets sheen for too; plain has only the blur, so there it's
    // Blur Intensity.
    let name_roughness = {
        let roughness_row = roughness_row.clone();
        move |glass: bool| {
            roughness_row.set_title(if glass { "Roughness" } else { "Blur Intensity" });
        }
    };
    advanced.add_row(&row(&SATURATION));
    advanced.add_row(&row(&NOISE));
    let mut glass_rows: Vec<gtk::Widget> = vec![
        preset_row.clone().upcast(),
        pattern,
        color,
        follow_accent,
        sheen.upcast(),
    ];
    for setting in GLASS.iter() {
        let row = row(setting);
        advanced.add_row(&row);
        glass_rows.push(row.upcast());
    }
    // Only a pattern has a depth.
    let pattern_depth = row(&PATTERN_DEPTH);
    advanced.add_row(&pattern_depth);
    glass_rows.push(pattern_depth.clone().upcast());
    let follow_pattern = {
        let pattern_depth = pattern_depth.downgrade();
        move |live: &Live, _: Option<&str>| {
            if let Some(pattern_depth) = pattern_depth.upgrade() {
                pattern_depth.set_sensitive(live.blur().glass.pattern != GlassPattern::Smooth);
            }
        }
    };
    follow_pattern(&live, None);
    live.connect(follow_pattern);
    let reset = adw::ButtonRow::builder().title("Reset to Defaults").build();
    advanced.add_row(&reset);
    list.append(&advanced);
    content.append(&list);
    group.add(&content);

    let lights_group = lights_group(ctx, &live);
    glass_rows.push(lights_group.clone().upcast());

    {
        let ctx = ctx.clone();
        reset.connect_activated(move |_| {
            ctx.store.edit("blur.advanced", |doc| {
                for path in ADVANCED_PATHS {
                    doc.remove(path);
                }
            });
        });
    }

    // Set while the controls follow the settings, so they don't write them back.
    let updating = Rc::new(Cell::new(false));
    {
        let (ctx, live, updating) = (ctx.clone(), live.clone(), updating.clone());
        let value = preset_value.clone();
        preset_scale.connect_value_changed(move |scale| {
            if updating.get() {
                return;
            }
            show_position(scale);
            value.set_label(nearest_preset(scale.value()));
            let preset = preset_at(snap_preset(scale.value()));
            let rough = rough_at(preset.roughness);
            let mut values = vec![
                (&*PASSES, rough.passes),
                (&*STRENGTH, rough.offset),
                (&*SHEEN, rough.sheen),
                (&*SATURATION, hundredths(preset.saturation)),
                (&*NOISE, thousandths(preset.noise)),
            ];
            if !lights(&live.blur().glass).is_empty() {
                values.push((&LIGHTS[0].size, hundredths(preset.light_size)));
            }
            edit_together(&ctx, &live, PRESET_KEY, values);
        });
    }
    {
        let (ctx, live, updating) = (ctx.clone(), live.clone(), updating.clone());
        let value = roughness_value.clone();
        roughness_scale.connect_value_changed(move |scale| {
            if updating.get() {
                return;
            }
            show_position(scale);
            value.set_label(&percent(scale.value()));
            let rough = rough_at(scale.value());
            edit_together(
                &ctx,
                &live,
                ROUGHNESS_KEY,
                vec![
                    (&*PASSES, rough.passes),
                    (&*STRENGTH, rough.offset),
                    (&*SHEEN, rough.sheen),
                ],
            );
        });
    }
    {
        let (ctx, live, updating) = (ctx.clone(), live.clone(), updating.clone());
        material.connect_active_name_notify(move |material| {
            let glass = material.active_name().as_deref() != Some("plain");
            for row in &glass_rows {
                row.set_visible(glass);
            }
            name_roughness(glass);
            if updating.get() {
                return;
            }
            live.show(Unsaved::Material(if glass {
                Material::Glass
            } else {
                Material::Plain
            }));
            let word = if glass { "glass" } else { "plain" };
            ctx.store.edit(MATERIAL_KEY, move |doc| {
                doc.set(MATERIAL_PATH, vec![Value::Str(word.to_owned())]);
            });
        });
    }

    let show = {
        let (ctx, updating, lights_group) = (ctx.clone(), updating.clone(), lights_group.clone());
        move || {
            let blur = ctx.store.config().blur;
            updating.set(true);
            if !ctx.store.is_pending(MATERIAL_KEY) {
                material.set_active_name(Some(match blur.material {
                    Material::Glass => "glass",
                    Material::Plain => "plain",
                }));
            }
            updating.set(false);
            for widget in [
                material.upcast_ref::<gtk::Widget>(),
                preset_row.upcast_ref(),
                advanced.upcast_ref(),
                lights_group.upcast_ref(),
            ] {
                widget.set_sensitive(!blur.off);
            }
            let defaults = Blur::default();
            reset.set_sensitive(
                blur.passes != defaults.passes
                    || blur.offset != defaults.offset
                    || blur.noise != defaults.noise
                    || blur.saturation != defaults.saturation
                    || blur.glass != defaults.glass,
            );
        }
    };
    show();
    ctx.store.subscribe(show);

    // Preset and Roughness follow the settings they set, and read Custom when no position of
    // theirs gives them.
    let follow = {
        let store = ctx.store.clone();
        let (preset_scale, preset_value) = (preset_scale.downgrade(), preset_value.downgrade());
        let (roughness_scale, roughness_value) =
            (roughness_scale.downgrade(), roughness_value.downgrade());
        move |live: &Live, key: Option<&str>| {
            let (
                Some(preset_scale),
                Some(preset_value),
                Some(roughness_scale),
                Some(roughness_value),
            ) = (
                preset_scale.upgrade(),
                preset_value.upgrade(),
                roughness_scale.upgrade(),
                roughness_value.upgrade(),
            )
            else {
                return;
            };
            let blur = live.blur();
            updating.set(true);
            // Not while it's the one moving, nor between its own saves.
            if key != Some(PRESET_KEY) && !store.is_pending(PRESET_KEY) {
                match preset_position(&blur) {
                    Some(position) => {
                        show_position(&preset_scale);
                        preset_scale.set_value(position);
                        preset_value.set_label(nearest_preset(position));
                    }
                    None => show_custom(&preset_scale, &preset_value),
                }
            }
            if key != Some(ROUGHNESS_KEY) && !store.is_pending(ROUGHNESS_KEY) {
                match roughness(&blur) {
                    Some(roughness) => {
                        show_position(&roughness_scale);
                        roughness_scale.set_value(roughness);
                        roughness_value.set_label(&percent(roughness));
                    }
                    None => show_custom(&roughness_scale, &roughness_value),
                }
            }
            updating.set(false);
        }
    };
    follow(&live, None);
    live.connect(follow);

    let page = gtk::Box::new(gtk::Orientation::Vertical, 24);
    page.append(&group);
    page.append(&lights_group);
    page.upcast()
}

/// Glass's lights: a row for each, which expands to its settings, and a row to add one.
fn lights_group(ctx: &PageContext, live: &Live) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Lights").build();
    ctx.anchor(LIGHTS_KEY, &group);
    group.add(&number_row(&POINTER_LIGHT, ctx, live));
    let none = adw::ActionRow::builder()
        .title("No Lights")
        .css_classes(["dim-label"])
        .build();
    group.add(&none);

    let rows: Vec<adw::ExpanderRow> = LIGHTS
        .iter()
        .enumerate()
        .map(|(i, settings)| {
            let row = light_row(i, settings, ctx, live);
            group.add(&row);
            row
        })
        .collect();

    let add = adw::ButtonRow::builder()
        .title("Add Light")
        .start_icon_name("list-add-symbolic")
        .build();
    group.add(&add);
    {
        let (ctx, live) = (ctx.clone(), live.clone());
        add.connect_activated(move |_| {
            let mut lights = lights(&live.blur().glass);
            if lights.is_empty() {
                // The light that's off comes back on, or the default one does.
                lights.push(GlassLight {
                    on: true,
                    ..live.blur().glass.lights[0].unwrap_or_default()
                });
                edit_lights(&ctx, &live, &lights, |doc| {
                    let first = [LIGHTS_PATH, &["0"]].concat();
                    let only_off = doc
                        .get(&first)
                        .is_none_or(|light| light.children.iter().all(|c| c.name == "off"));
                    if only_off {
                        doc.remove(LIGHTS_PATH);
                    } else {
                        doc.remove(&[&first[..], &["off"]].concat());
                    }
                });
                return;
            }
            if lights.len() == MAX_GLASS_LIGHTS {
                return;
            }
            let (index, angle) = (lights.len(), free_angle(&lights));
            lights.push(GlassLight {
                angle,
                ..GlassLight::default()
            });
            edit_lights(&ctx, &live, &lights, move |doc| {
                // The default light is only there while no light is written: write it first, so
                // the new one goes beside it.
                if doc.get(LIGHTS_PATH).is_none() {
                    doc.set(&[LIGHTS_PATH, &["0"]].concat(), Vec::new());
                }
                let index = index.to_string();
                let path = [LIGHTS_PATH, &[&index, "angle"]].concat();
                doc.set(&path, vec![Value::Int(angle as i64)]);
            });
        });
    }

    let follow = {
        let (none, add) = (none.downgrade(), add.downgrade());
        let rows: Vec<_> = rows.iter().map(|row| row.downgrade()).collect();
        move |live: &Live, _: Option<&str>| {
            let (Some(none), Some(add)) = (none.upgrade(), add.upgrade()) else {
                return;
            };
            let lights = lights(&live.blur().glass);
            none.set_visible(lights.is_empty());
            add.set_sensitive(lights.len() < MAX_GLASS_LIGHTS);
            for (i, row) in rows.iter().enumerate() {
                if let Some(row) = row.upgrade() {
                    row.set_visible(i < lights.len());
                    row.set_show_enable_switch(lights.len() > 1);
                }
            }
        }
    };
    follow(live, None);
    live.connect(follow);
    group
}

/// The row for the light at `i`: its switch, its settings, and a button to remove it.
fn light_row(
    i: usize,
    settings: &'static LightSettings,
    ctx: &PageContext,
    live: &Live,
) -> adw::ExpanderRow {
    let row = adw::ExpanderRow::builder()
        .title(format!("Light {}", i + 1))
        .show_enable_switch(true)
        .build();
    let remove = gtk::Button::builder()
        .icon_name("user-trash-symbolic")
        .tooltip_text("Remove Light")
        .valign(gtk::Align::Center)
        .css_classes(["flat", "circular"])
        .build();
    row.add_suffix(&remove);
    for setting in [
        &settings.intensity,
        &settings.angle,
        &settings.height,
        &settings.size,
    ] {
        row.add_row(&light_number_row(setting, ctx, live));
    }

    {
        let (ctx, live) = (ctx.clone(), live.clone());
        remove.connect_clicked(move |_| {
            let mut lights = lights(&live.blur().glass);
            if i >= lights.len() {
                return;
            }
            lights.remove(i);
            let last = lights.is_empty();
            edit_lights(&ctx, &live, &lights, move |doc| {
                let index = i.to_string();
                doc.remove(&[LIGHTS_PATH, &[&index]].concat());
                // An empty list would be the default light again.
                if last {
                    doc.set(&[LIGHTS_PATH, &["0", "off"]].concat(), Vec::new());
                }
            });
        });
    }

    // Set while it follows the light, so it doesn't save what it's shown.
    let updating = Rc::new(Cell::new(false));
    {
        let (ctx, live, updating) = (ctx.clone(), live.clone(), updating.clone());
        let off = &settings.off;
        row.connect_enable_expansion_notify(move |row| {
            if updating.get() {
                return;
            }
            let on = row.enables_expansion();
            live.show(Unsaved::Setting(off.path, f64::from(u8::from(!on))));
            ctx.store
                .edit(off.path, move |doc| off.write(doc, Input::Bool(on)));
        });
    }
    let follow = {
        let (store, row) = (ctx.store.clone(), row.downgrade());
        let off = settings.off.path;
        move |live: &Live, key: Option<&str>| {
            let Some(row) = row.upgrade() else {
                return;
            };
            // Not while it's the one moving, nor between its own saves.
            if key == Some(off) || store.is_pending(off) {
                return;
            }
            let Some(light) = live.blur().glass.lights[i] else {
                return;
            };
            updating.set(true);
            row.set_enable_expansion(light.on);
            updating.set(false);
        }
    };
    follow(live, None);
    live.connect(follow);
    row
}

/// Counts the edits that add or remove lights. A light's settings address it by its place in the
/// list, so their edits are saved under keys with the count: an edit made before a light is
/// removed and one made after then never replace each other, and each applies to the light it
/// meant, in order.
static LIGHT_LIST_VERSION: AtomicU32 = AtomicU32::new(0);

/// Saves an edit that adds or removes lights, showing `lights` in the preview right away.
fn edit_lights(
    ctx: &PageContext,
    live: &Live,
    lights: &[GlassLight],
    f: impl FnOnce(&mut Document) + Send + 'static,
) {
    let key = format!(
        "{LIGHTS_KEY}.{}",
        LIGHT_LIST_VERSION.fetch_add(1, Ordering::Relaxed)
    );
    live.show_edit(&key, &[Unsaved::Lights(light_list(lights))]);
    ctx.store.edit(&key, f);
}

/// Shows a slider that sets several settings as Custom, without its knob: the settings aren't
/// those of any of its positions. Grabbing it sets them again.
fn show_custom(scale: &gtk::Scale, value: &gtk::Label) {
    scale.add_css_class("custom");
    value.set_label("Custom");
}

/// Shows a slider's knob again, at a position of its.
fn show_position(scale: &gtk::Scale) {
    scale.remove_css_class("custom");
}

/// A setting's slider, which shows its value in the preview and the other controls as it moves,
/// and follows them as they move.
fn number_row(setting: &'static Setting, ctx: &PageContext, live: &Live) -> adw::ActionRow {
    slider_setting_row(setting, ctx, live, || setting.path.to_owned())
}

/// A light's setting's slider, like `number_row`, whose edits are saved under keys that change
/// when lights are added or removed.
fn light_number_row(setting: &'static Setting, ctx: &PageContext, live: &Live) -> adw::ActionRow {
    slider_setting_row(setting, ctx, live, || {
        format!(
            "{}@{}",
            setting.path,
            LIGHT_LIST_VERSION.load(Ordering::Relaxed)
        )
    })
}

/// A setting's slider, saving its edits under the key `key` gives at the time.
fn slider_setting_row(
    setting: &'static Setting,
    ctx: &PageContext,
    live: &Live,
    key: impl Fn() -> String + 'static,
) -> adw::ActionRow {
    let Control::Number {
        min,
        max,
        step,
        format,
        ..
    } = setting.control
    else {
        unreachable!("blur's settings are numbers");
    };
    let format = format.unwrap_or(|x| format!("{x}"));
    let (row, scale, value) = slider_row(setting.title, min, max, step);
    ctx.anchor(setting.path, &row);
    let key = Rc::new(key);

    // Set while it follows the others, so it doesn't save what it's shown.
    let updating = Rc::new(Cell::new(false));
    {
        let (ctx, live, updating, value, key) = (
            ctx.clone(),
            live.clone(),
            updating.clone(),
            value.clone(),
            key.clone(),
        );
        scale.connect_value_changed(move |scale| {
            let x = scale.value();
            value.set_label(&format(x));
            if updating.get() {
                return;
            }
            let key = key();
            live.show_edit(&key, &[Unsaved::Setting(setting.path, x)]);
            ctx.store
                .edit(&key, move |doc| setting.write(doc, Input::Number(x)));
        });
    }

    let follow = {
        let store = ctx.store.clone();
        let (scale, value) = (scale.downgrade(), value.downgrade());
        move |live: &Live, edit: Option<&str>| {
            let (Some(scale), Some(value)) = (scale.upgrade(), value.upgrade()) else {
                return;
            };
            // Not while it's the one moving, nor between its own saves.
            let key = key();
            if edit == Some(key.as_str()) || store.is_pending(&key) {
                return;
            }
            let Some(x) = live.shown(setting.path) else {
                return;
            };
            updating.set(true);
            scale.set_value(x);
            // The scale doesn't signal when the value stays the same.
            value.set_label(&format(x));
            updating.set(false);
        }
    };
    follow(live, None);
    live.connect(follow);
    row
}

fn percent(x: f64) -> String {
    format!("{:.0} %", x * 100.)
}

/// Saves several settings in one edit under `key`, showing them in the preview right away.
fn edit_together(
    ctx: &PageContext,
    live: &Live,
    key: &'static str,
    values: Vec<(&'static Setting, f64)>,
) {
    let changes: Vec<Unsaved> = values
        .iter()
        .map(|(setting, x)| Unsaved::Setting(setting.path, *x))
        .collect();
    live.show_edit(key, &changes);
    ctx.store.edit(key, move |doc| {
        for (setting, x) in values {
            setting.write(doc, Input::Number(x));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bentos_defaults_are_the_middle_roughness_and_the_regular_preset() {
        let blur = Blur::default();
        assert_eq!(roughness(&blur), Some(0.4));
        assert_eq!(preset_position(&blur), Some(1.));
    }

    #[test]
    fn roughness_round_trips_through_its_settings() {
        for i in 0..=100 {
            let r = f64::from(i) / 100.;
            let rough = rough_at(r);
            let mut blur = Blur::default();
            blur.passes = rough.passes as u8;
            blur.offset = rough.offset;
            blur.glass.sheen = rough.sheen;
            assert_eq!(roughness(&blur), Some(r), "{r}");
        }
    }

    #[test]
    fn roughness_reads_back_where_it_was_dragged() {
        // A dragged slider stops between its steps, like at 0.618.
        for i in 0..=1000 {
            let dragged = f64::from(i) / 1000.;
            let rough = rough_at(dragged);
            let mut blur = Blur::default();
            blur.passes = rough.passes as u8;
            blur.offset = rough.offset;
            blur.glass.sheen = rough.sheen;
            assert_eq!(roughness(&blur), Some(hundredths(dragged)), "{dragged}");
        }
    }

    #[test]
    fn changing_a_part_of_roughness_makes_it_custom() {
        let mut blur = Blur::default();
        blur.glass.sheen = 0.9;
        assert_eq!(roughness(&blur), None);
        assert_eq!(preset_position(&blur), None);
        // Plain has no sheen, so it doesn't count there.
        blur.material = Material::Plain;
        assert_eq!(roughness(&blur), Some(0.4));

        let mut blur = Blur::default();
        blur.saturation = 1.0;
        assert_eq!(roughness(&blur), Some(0.4));
        assert_eq!(preset_position(&blur), None);
    }

    #[test]
    fn presets_round_trip_through_their_settings() {
        for i in 0..=200 {
            let position = f64::from(i) / 100.;
            let preset = preset_at(snap_preset(position));
            let rough = rough_at(preset.roughness);
            let mut blur = Blur::default();
            blur.passes = rough.passes as u8;
            blur.offset = rough.offset;
            blur.glass.sheen = rough.sheen;
            blur.saturation = hundredths(preset.saturation);
            blur.noise = thousandths(preset.noise);
            blur.glass.lights[0].as_mut().unwrap().size = hundredths(preset.light_size);
            let found = preset_position(&blur).expect("a preset's settings are a preset");
            assert!(
                (found - position).abs() < 0.02,
                "{position} came back as {found}"
            );
        }
    }

    #[test]
    fn preset_sets_the_first_lights_size_and_leaves_no_lights_be() {
        let mut blur = Blur::default();
        blur.glass.lights[0].as_mut().unwrap().size = 0.9;
        assert_eq!(preset_position(&blur), None);

        // No lights: only the light that's off, whose size doesn't count.
        blur.glass.lights[0].as_mut().unwrap().on = false;
        assert!(lights(&blur.glass).is_empty());
        assert_eq!(preset_position(&blur), Some(1.));
    }

    #[test]
    fn no_lights_is_one_switched_off() {
        let glass = Glass {
            lights: light_list(&[]),
            ..Glass::default()
        };
        assert!(no_lights(&glass));
        assert!(!no_lights(&Glass::default()));

        // One that's off beside one that's on is a light.
        let off = GlassLight {
            on: false,
            ..GlassLight::default()
        };
        let glass = Glass {
            lights: light_list(&[off, GlassLight::default()]),
            ..Glass::default()
        };
        assert_eq!(lights(&glass), [off, GlassLight::default()]);
    }

    #[test]
    fn new_lights_go_in_the_widest_gap() {
        let at = |angle| GlassLight {
            angle,
            ..GlassLight::default()
        };
        assert_eq!(free_angle(&[]), 315.);
        assert_eq!(free_angle(&[at(315.)]), 135.);
        assert_eq!(free_angle(&[at(90.), at(0.)]), 225.);
        assert_eq!(free_angle(&[at(350.), at(10.), at(200.)]), 105.);
    }
}
