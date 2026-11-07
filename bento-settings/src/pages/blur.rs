//! Blur: the material bento draws behind windows and panels that ask for it, glass or plain,
//! with a preview of it over the wallpaper.
//!
//! Two sliders set several settings at once: Roughness sets how far the blur reaches (passes and
//! strength) and how much glass spreads the light it reflects (sheen), and Preset sets a whole
//! look. Each reads Custom, without its knob, while the settings it sets don't match any of its
//! positions, as after changing one of them under Advanced.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::LazyLock;

use adw::prelude::*;
use niri_config::{Blur, Material};

use super::blur_live::{Live, Unsaved};
use crate::kdl::Value;
use crate::model::{BoolStyle, Control, Input, Setting};
use crate::page::{setting_row_watched, PageContext};

const MATERIAL_KEY: &str = "blur.material";
/// The store's key for Roughness's edits, which set passes, offset and sheen together.
const ROUGHNESS_KEY: &str = "blur.roughness";
/// The store's key for the Preset slider's edits, which set a whole look for glass.
const PRESET_KEY: &str = "blur.glass.preset";
const MATERIAL_PATH: &[&str] = &["blur", "material"];

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

static LIGHT_SIZE: LazyLock<Setting> = LazyLock::new(|| {
    Setting::number("Light Size", "blur.glass.light-size", |c| {
        c.blur.glass.light_size
    })
    .range(0., 1., 0.01)
    .digits(2)
    .slider()
    .format(|x| format!("{:.0} %", x * 100.))
    .keywords(&["light", "softness", "highlight"])
});

static LIGHT_ANGLE: LazyLock<Setting> = LazyLock::new(|| {
    Setting::number("Light Angle", "blur.glass.light-angle", |c| {
        c.blur.glass.light_angle
    })
    .range(0., 360., 1.)
    .slider()
    .format(|x| format!("{x:.0}°"))
    .keywords(&["light direction"])
});

/// Glass's shape and its light, which neither Roughness nor Preset sets.
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
        Setting::number("Highlight", "blur.glass.highlight", |c| {
            c.blur.glass.highlight
        })
        .range(0., 2., 0.01)
        .digits(2)
        .slider()
        .format(|x| format!("{x:.2}"))
        .keywords(&["glare", "edge light", "shine"]),
    ]
});

/// The settings Roughness sets, at a roughness from 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rough {
    passes: f64,
    offset: f64,
    sheen: f64,
}

/// How far the blur reaches, about `offset · 2^passes`, at a roughness: from 4 to 136, evenly on
/// a log scale, with bento's defaults, 3 passes of 3, in the middle.
fn reach(roughness: f64) -> f64 {
    24. * (5. * (roughness - 0.5)).exp2()
}

/// The settings at `roughness`. The passes are the ones that put the offset nearest 3, where
/// the blur looks smoothest, and the offset and sheen are in hundredths, as they're saved.
fn rough_at(roughness: f64) -> Rough {
    let reach = reach(roughness);
    let passes = (reach.log2() - 3f64.log2()).round().clamp(1., 8.);
    Rough {
        passes,
        offset: hundredths(reach / passes.exp2()),
        sheen: hundredths(roughness),
    }
}

/// The roughness `blur`'s passes and offset reach as far as.
fn roughness_of(blur: &Blur) -> f64 {
    let reach = blur.offset * f64::from(blur.passes).exp2();
    ((reach / 24.).log2() / 5. + 0.5).clamp(0., 1.)
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

/// A look for glass, from its Preset slider.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Preset {
    roughness: f64,
    saturation: f64,
    noise: f64,
    light_size: f64,
}

/// Clear, Regular and Hazy, at 0, 1 and 2 on the Preset slider. Regular is bento's defaults.
const PRESETS: [(&str, Preset); 3] = [
    (
        "Clear",
        Preset {
            roughness: 0.2,
            saturation: 1.2,
            noise: 0.,
            light_size: 0.2,
        },
    ),
    (
        "Regular",
        Preset {
            roughness: 0.5,
            saturation: 1.5,
            noise: 0.02,
            light_size: 0.4,
        },
    ),
    (
        "Hazy",
        Preset {
            roughness: 0.8,
            saturation: 1.6,
            noise: 0.04,
            light_size: 0.7,
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
        && close(blur.glass.light_size, hundredths(at.light_size)))
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
    let row = adw::ActionRow::builder().title(title).build();
    row.add_suffix(&value);
    row.add_suffix(&scale);
    (row, scale, value)
}

pub fn build(ctx: &PageContext) -> gtk::Widget {
    let group = adw::PreferencesGroup::builder().title("Blur").build();
    ctx.anchor("appearance.blur", &group);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let live = Live::new(&ctx.store);
    content.append(&super::blur_preview::build(ctx, &live, &LIGHT_ANGLE));
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

    let advanced = adw::ExpanderRow::builder().title("Advanced").build();
    let (roughness_row, roughness_scale, roughness_value) = slider_row("Roughness", 0., 1., 0.01);
    ctx.anchor(ROUGHNESS_KEY, &roughness_row);
    advanced.add_row(&roughness_row);
    let passes = part(&PASSES);
    let strength = part(&STRENGTH);
    let sheen = part(&SHEEN);
    advanced.add_row(&passes);
    advanced.add_row(&strength);
    advanced.add_row(&sheen);
    // Roughness is glass's, which it sets sheen for too; plain has only the blur, so there it's
    // Blur Intensity.
    let name_roughness = {
        let (roughness_row, passes, strength, sheen) = (
            roughness_row.clone(),
            passes.clone(),
            strength.clone(),
            sheen.clone(),
        );
        move |glass: bool| {
            let (title, subtitle) = if glass {
                (
                    "Roughness",
                    "How far the blur reaches, and how glass spreads its light",
                )
            } else {
                ("Blur Intensity", "How far the blur reaches")
            };
            roughness_row.set_title(title);
            roughness_row.set_subtitle(subtitle);
            for part in [&passes, &strength, &sheen] {
                part.set_subtitle(&format!("Set by {title}, with the others"));
            }
        }
    };
    advanced.add_row(&row(&SATURATION));
    advanced.add_row(&row(&NOISE));
    let mut glass_rows: Vec<gtk::Widget> = vec![preset_row.clone().upcast(), sheen.upcast()];
    for setting in GLASS.iter().chain([&*LIGHT_SIZE, &*LIGHT_ANGLE]) {
        let row = row(setting);
        advanced.add_row(&row);
        glass_rows.push(row.upcast());
    }
    let reset = adw::ButtonRow::builder().title("Reset to Defaults").build();
    advanced.add_row(&reset);
    list.append(&advanced);
    content.append(&list);
    group.add(&content);

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
            edit_together(
                &ctx,
                &live,
                PRESET_KEY,
                [
                    (&PASSES, rough.passes),
                    (&STRENGTH, rough.offset),
                    (&SHEEN, rough.sheen),
                    (&SATURATION, hundredths(preset.saturation)),
                    (&NOISE, thousandths(preset.noise)),
                    (&LIGHT_SIZE, hundredths(preset.light_size)),
                ],
            );
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
                [
                    (&PASSES, rough.passes),
                    (&STRENGTH, rough.offset),
                    (&SHEEN, rough.sheen),
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
        let (ctx, updating) = (ctx.clone(), updating.clone());
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
        move |live: &Live, key: Option<&'static str>| {
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

    group.upcast()
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

    // Set while it follows the others, so it doesn't save what it's shown.
    let updating = Rc::new(Cell::new(false));
    {
        let (ctx, live, updating, value) =
            (ctx.clone(), live.clone(), updating.clone(), value.clone());
        scale.connect_value_changed(move |scale| {
            let x = scale.value();
            value.set_label(&format(x));
            if updating.get() {
                return;
            }
            live.show(Unsaved::Setting(setting.path, x));
            ctx.store.edit(setting.path, move |doc| {
                setting.write(doc, Input::Number(x))
            });
        });
    }

    let follow = {
        let store = ctx.store.clone();
        let (scale, value) = (scale.downgrade(), value.downgrade());
        move |live: &Live, key: Option<&'static str>| {
            let (Some(scale), Some(value)) = (scale.upgrade(), value.upgrade()) else {
                return;
            };
            // Not while it's the one moving, nor between its own saves.
            if key == Some(setting.path) || store.is_pending(setting.path) {
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
fn edit_together<const N: usize>(
    ctx: &PageContext,
    live: &Live,
    key: &'static str,
    values: [(&'static LazyLock<Setting>, f64); N],
) {
    let values = values.map(|(setting, x)| (&**setting, x));
    live.show_edit(
        key,
        &values.map(|(setting, x)| Unsaved::Setting(setting.path, x)),
    );
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
        assert_eq!(roughness(&blur), Some(0.5));
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
    fn changing_a_part_of_roughness_makes_it_custom() {
        let mut blur = Blur::default();
        blur.glass.sheen = 0.9;
        assert_eq!(roughness(&blur), None);
        assert_eq!(preset_position(&blur), None);
        // Plain has no sheen, so it doesn't count there.
        blur.material = Material::Plain;
        assert_eq!(roughness(&blur), Some(0.5));

        let mut blur = Blur::default();
        blur.saturation = 1.0;
        assert_eq!(roughness(&blur), Some(0.5));
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
            blur.glass.light_size = hundredths(preset.light_size);
            let found = preset_position(&blur).expect("a preset's settings are a preset");
            assert!(
                (found - position).abs() < 0.02,
                "{position} came back as {found}"
            );
        }
    }
}
