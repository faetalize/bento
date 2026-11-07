//! Blur: the material bento draws behind windows and panels that ask for it, glass or frosted,
//! with a preview of it over the wallpaper.

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
/// The store's key for Blur Amount's edits, which set passes and offset together.
const AMOUNT_KEY: &str = "blur.amount";
/// The store's key for the Preset slider's edits, which set what makes glass clear or hazy.
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

/// Passes and Strength, which Blur Amount sets together.
static STRENGTH: LazyLock<Setting> = LazyLock::new(|| {
    Setting::number("Strength", "blur.offset", |c| c.blur.offset)
        .range(0., 10., 0.01)
        .digits(2)
        .slider()
        .format(|x| format!("{x:.2}"))
        .keywords(&["radius", "offset"])
});

static PASSES: LazyLock<Setting> = LazyLock::new(|| {
    Setting::number("Passes", "blur.passes", |c| f64::from(c.blur.passes))
        .range(1., 8., 1.)
        .slider()
        .format(|x| format!("{x:.0}"))
        .keywords(&["quality", "iterations"])
});

static NOISE: LazyLock<Setting> = LazyLock::new(|| {
    Setting::number("Noise", "blur.noise", |c| c.blur.noise)
        .range(0., 0.2, 0.01)
        .digits(2)
        .slider()
        .format(|x| format!("{x:.2}"))
        .keywords(&["grain", "dither"])
});

static SATURATION: LazyLock<Setting> = LazyLock::new(|| {
    Setting::number("Saturation", "blur.saturation", |c| c.blur.saturation)
        .range(0., 3., 0.05)
        .digits(2)
        .slider()
        .format(|x| format!("{:.0} %", x * 100.))
        .keywords(&["vibrancy", "color"])
});

static CLARITY: LazyLock<Setting> = LazyLock::new(|| {
    Setting::number("Clarity", "blur.glass.clarity", |c| c.blur.glass.clarity)
        .range(0., 1., 0.01)
        .digits(2)
        .slider()
        .format(|x| format!("{:.0} %", x * 100.))
        .keywords(&["sharp", "clear", "haze"])
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

/// The rest of the settings only glass has.
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
        .range(0., 2., 0.05)
        .digits(2)
        .slider()
        .format(|x| format!("{x:.2}"))
        .keywords(&["glare", "edge light", "shine"]),
    ]
});

/// How far the blur reaches, about `offset · 2^passes`, at a Blur Amount from 0 to 1: from 4 to
/// 128, evenly on a log scale.
fn reach(amount: f64) -> f64 {
    4. * (5. * amount).exp2()
}

/// The Blur Amount that `blur` reaches as far as.
fn amount(blur: &Blur) -> f64 {
    let reach = blur.offset * f64::from(blur.passes).exp2();
    ((reach / 4.).log2() / 5.).clamp(0., 1.)
}

/// The passes and offset that reach `reach`, with the offset in hundredths as it's saved.
fn passes_and_offset(reach: f64) -> (f64, f64) {
    let passes = (reach.log2() - 1.).round().clamp(1., 8.);
    (passes, hundredths(reach / passes.exp2()))
}

fn hundredths(x: f64) -> f64 {
    (x * 100.).round() / 100.
}

/// A look for glass, from its Preset slider.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Preset {
    clarity: f64,
    reach: f64,
    noise: f64,
    saturation: f64,
}

/// Clear, Regular and Hazy, at 0, 1 and 2 on the Preset slider.
const PRESETS: [(&str, Preset); 3] = [
    (
        "Clear",
        Preset {
            clarity: 1.,
            reach: 8.,
            noise: 0.,
            saturation: 1.2,
        },
    ),
    (
        "Regular",
        Preset {
            clarity: 0.5,
            reach: 24.,
            noise: 0.02,
            saturation: 1.4,
        },
    ),
    (
        "Hazy",
        Preset {
            clarity: 0.,
            reach: 72.,
            noise: 0.04,
            saturation: 1.5,
        },
    ),
];

/// The look at `position` on the Preset slider, between the two presets around it. The reach
/// changes evenly on a log scale, like Blur Amount.
fn preset_at(position: f64) -> Preset {
    let last = (PRESETS.len() - 1) as f64;
    let position = position.clamp(0., last);
    let i = (position.floor() as usize).min(PRESETS.len() - 2);
    let t = position - i as f64;
    let (a, b) = (PRESETS[i].1, PRESETS[i + 1].1);
    let lerp = |a: f64, b: f64| a + (b - a) * t;
    Preset {
        clarity: lerp(a.clarity, b.clarity),
        reach: lerp(a.reach.log2(), b.reach.log2()).exp2(),
        noise: lerp(a.noise, b.noise),
        saturation: lerp(a.saturation, b.saturation),
    }
}

/// The name of the preset nearest `position` on the Preset slider.
fn nearest_preset(position: f64) -> &'static str {
    let i = position.round().clamp(0., (PRESETS.len() - 1) as f64) as usize;
    PRESETS[i].0
}

/// Where `clarity` puts the Preset slider.
fn preset_position(clarity: f64) -> f64 {
    for (i, pair) in PRESETS.windows(2).enumerate() {
        let (a, b) = (pair[0].1.clarity, pair[1].1.clarity);
        if (b..=a).contains(&clarity) {
            return i as f64 + (a - clarity) / (a - b);
        }
    }
    if clarity > PRESETS[0].1.clarity {
        0.
    } else {
        (PRESETS.len() - 1) as f64
    }
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
    // A raw slider that Blur Amount also sets.
    let coupled = |setting: &'static Setting, other: &str| {
        let row = number_row(setting, ctx, &live);
        row.set_subtitle(&format!("Blur Amount sets this together with {other}"));
        row
    };
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
    for (name, label) in [("glass", "Glass"), ("frosted", "Frosted")] {
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
    // with Blur Amount's.
    let (preset_row, preset_scale, preset_value) =
        slider_row("Preset", 0., (PRESETS.len() - 1) as f64, 0.01);
    for i in 0..PRESETS.len() {
        preset_scale.add_mark(i as f64, gtk::PositionType::Bottom, None);
    }
    ctx.anchor(PRESET_KEY, &preset_row);
    list.append(&preset_row);

    let (amount_row, amount_scale, amount_value) = slider_row("Blur Amount", 0., 1., 0.01);
    ctx.anchor(AMOUNT_KEY, &amount_row);
    list.append(&amount_row);

    let advanced = adw::ExpanderRow::builder().title("Advanced").build();
    advanced.add_row(&coupled(&PASSES, "Strength"));
    advanced.add_row(&coupled(&STRENGTH, "Passes"));
    advanced.add_row(&row(&NOISE));
    advanced.add_row(&row(&SATURATION));
    let mut glass_rows = vec![preset_row.clone().upcast::<gtk::Widget>()];
    for setting in std::iter::once(&*CLARITY)
        .chain(GLASS.iter())
        .chain([&*LIGHT_ANGLE])
    {
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

    // Set while the controls show the config, so they don't write it back.
    let updating = Rc::new(Cell::new(false));
    {
        let (ctx, live, updating) = (ctx.clone(), live.clone(), updating.clone());
        let value = preset_value.clone();
        preset_scale.connect_value_changed(move |scale| {
            value.set_label(nearest_preset(scale.value()));
            if updating.get() {
                return;
            }
            let preset = preset_at(scale.value());
            let (passes, offset) = passes_and_offset(preset.reach);
            edit_together(
                &ctx,
                &live,
                PRESET_KEY,
                [
                    (&CLARITY, hundredths(preset.clarity)),
                    (&PASSES, passes),
                    (&STRENGTH, offset),
                    (&NOISE, hundredths(preset.noise)),
                    (&SATURATION, hundredths(preset.saturation)),
                ],
            );
        });
    }
    {
        let (ctx, live, updating) = (ctx.clone(), live.clone(), updating.clone());
        let value = amount_value.clone();
        amount_scale.connect_value_changed(move |scale| {
            value.set_label(&percent(scale.value()));
            if updating.get() {
                return;
            }
            let (passes, offset) = passes_and_offset(reach(scale.value()));
            edit_together(
                &ctx,
                &live,
                AMOUNT_KEY,
                [(&PASSES, passes), (&STRENGTH, offset)],
            );
        });
    }
    {
        let (ctx, live, updating) = (ctx.clone(), live.clone(), updating.clone());
        material.connect_active_name_notify(move |material| {
            let glass = material.active_name().as_deref() != Some("frosted");
            for row in &glass_rows {
                row.set_visible(glass);
            }
            if updating.get() {
                return;
            }
            live.show(Unsaved::Material(if glass {
                Material::Glass
            } else {
                Material::Frosted
            }));
            let word = if glass { "glass" } else { "frosted" };
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
                    Material::Frosted => "frosted",
                }));
            }
            updating.set(false);
            for widget in [
                material.upcast_ref::<gtk::Widget>(),
                preset_row.upcast_ref(),
                amount_row.upcast_ref(),
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

    let follow = {
        let store = ctx.store.clone();
        let (preset_scale, preset_value) = (preset_scale.downgrade(), preset_value.downgrade());
        let (amount_scale, amount_value) = (amount_scale.downgrade(), amount_value.downgrade());
        move |live: &Live, key: Option<&'static str>| {
            let (Some(preset_scale), Some(preset_value), Some(amount_scale), Some(amount_value)) = (
                preset_scale.upgrade(),
                preset_value.upgrade(),
                amount_scale.upgrade(),
                amount_value.upgrade(),
            ) else {
                return;
            };
            let shown = |path| live.shown(path).unwrap_or_default();
            updating.set(true);
            // Not while it's the one moving, nor between its own saves.
            if key != Some(PRESET_KEY) && !store.is_pending(PRESET_KEY) {
                let position = preset_position(shown(CLARITY.path));
                preset_scale.set_value(position);
                // The scale doesn't signal when the value stays the same.
                preset_value.set_label(nearest_preset(position));
            }
            if key != Some(AMOUNT_KEY) && !store.is_pending(AMOUNT_KEY) {
                let blur = Blur {
                    passes: shown(PASSES.path) as u8,
                    offset: shown(STRENGTH.path),
                    ..Blur::default()
                };
                // Set the label too: the scale doesn't signal when the value stays the same.
                let current = amount(&blur);
                amount_scale.set_value(current);
                amount_value.set_label(&percent(current));
            }
            updating.set(false);
        }
    };
    follow(&live, None);
    live.connect(follow);

    group.upcast()
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
    fn blur_amount_round_trips() {
        for amount_in in [0., 0.25, 0.5, 0.75, 1.] {
            let (passes, offset) = passes_and_offset(reach(amount_in));
            let blur = Blur {
                passes: passes as u8,
                offset,
                ..Blur::default()
            };
            assert!((amount(&blur) - amount_in).abs() < 0.005, "{amount_in}");
        }
        assert_eq!(passes_and_offset(4.), (1., 2.));
        assert_eq!(passes_and_offset(128.), (6., 2.));
    }

    #[test]
    fn presets_follow_clarity() {
        for (i, (_, preset)) in PRESETS.iter().enumerate() {
            let at = preset_at(i as f64);
            assert_eq!((at.clarity, at.noise), (preset.clarity, preset.noise));
            assert!((at.reach - preset.reach).abs() < 1e-9);
            assert_eq!(preset_position(preset.clarity), i as f64);
        }
        assert!((preset_position(preset_at(1.5).clarity) - 1.5).abs() < 1e-9);
        assert!((preset_at(0.5).reach - 8. * 3f64.sqrt()).abs() < 1e-9);
    }
}
