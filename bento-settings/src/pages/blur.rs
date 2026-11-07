//! Blur: the material bento draws behind windows and panels that ask for it, glass or frosted,
//! with a preview of it over the wallpaper.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::LazyLock;

use adw::prelude::*;
use niri_config::{Blur, Material};

use super::blur_preview::Unsaved;
use crate::kdl::Value;
use crate::model::{BoolStyle, Input, Setting};
use crate::page::{setting_row_watched, PageContext};

const MATERIAL_KEY: &str = "blur.material";
const MATERIAL_PATH: &[&str] = &["blur", "material"];

/// What resetting the advanced settings removes from the config.
const ADVANCED_PATHS: &[&[&str]] = &[
    &["blur", "passes"],
    &["blur", "noise"],
    &["blur", "saturation"],
    &["blur", "glass"],
];

static SWITCH: LazyLock<Setting> = LazyLock::new(|| {
    Setting::switch("Blur", "blur", |c| !c.blur.off)
        .style(BoolStyle::OnOff)
        .keywords(&["transparency", "translucency", "background effect"])
});

static STRENGTH: LazyLock<Setting> = LazyLock::new(|| {
    Setting::number("Strength", "blur.offset", |c| c.blur.offset)
        .range(0., 10., 0.1)
        .digits(1)
        .slider()
        .format(|x| format!("{x:.1}"))
        .keywords(&["radius", "amount", "offset"])
});

/// Settings both materials share.
static SHARED: LazyLock<Vec<Setting>> = LazyLock::new(|| {
    vec![
        Setting::number("Passes", "blur.passes", |c| f64::from(c.blur.passes))
            .range(1., 8., 1.)
            .slider()
            .format(|x| format!("{x:.0}"))
            .keywords(&["quality", "iterations"]),
        Setting::number("Noise", "blur.noise", |c| c.blur.noise)
            .range(0., 0.2, 0.01)
            .digits(2)
            .slider()
            .format(|x| format!("{x:.2}"))
            .keywords(&["grain", "dither"]),
        Setting::number("Saturation", "blur.saturation", |c| c.blur.saturation)
            .range(0., 3., 0.05)
            .digits(2)
            .slider()
            .format(|x| format!("{:.0} %", x * 100.))
            .keywords(&["vibrancy", "color"]),
    ]
});

/// Settings only glass has.
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
        Setting::number("Light Angle", "blur.glass.light-angle", |c| {
            c.blur.glass.light_angle
        })
        .range(0., 360., 1.)
        .slider()
        .format(|x| format!("{x:.0}°"))
        .keywords(&["light direction"]),
    ]
});

pub fn build(ctx: &PageContext) -> gtk::Widget {
    let group = adw::PreferencesGroup::builder().title("Blur").build();
    ctx.anchor("appearance.blur", &group);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let (preview, live) = super::blur_preview::build(ctx);
    content.append(&preview);
    // Each control shows its value in the preview as it moves; the config saves it after.
    let row = |setting: &'static Setting| {
        let live = live.clone();
        setting_row_watched(setting, ctx, move |input| {
            let value = match input {
                Input::Number(x) => *x,
                Input::Bool(on) => f64::from(u8::from(*on)),
                _ => return,
            };
            live.show(Unsaved::Setting(setting.path, value));
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
    list.append(&row(&SWITCH));
    let strength = row(&STRENGTH);
    list.append(&strength);

    let advanced = adw::ExpanderRow::builder().title("Advanced").build();
    for setting in SHARED.iter() {
        advanced.add_row(&row(setting));
    }
    let glass_rows: Vec<gtk::Widget> = GLASS
        .iter()
        .map(|setting| {
            let row = row(setting);
            advanced.add_row(&row);
            row
        })
        .collect();
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

    // Set while the toggles show the config, so they don't write it back.
    let updating = Rc::new(Cell::new(false));
    {
        let (ctx, updating) = (ctx.clone(), updating.clone());
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
        let ctx = ctx.clone();
        move || {
            let blur = ctx.store.config().blur;
            if !ctx.store.is_pending(MATERIAL_KEY) {
                updating.set(true);
                material.set_active_name(Some(match blur.material {
                    Material::Glass => "glass",
                    Material::Frosted => "frosted",
                }));
                updating.set(false);
            }
            for widget in [
                material.upcast_ref::<gtk::Widget>(),
                &strength,
                advanced.upcast_ref(),
            ] {
                widget.set_sensitive(!blur.off);
            }
            let defaults = Blur::default();
            reset.set_sensitive(
                blur.passes != defaults.passes
                    || blur.noise != defaults.noise
                    || blur.saturation != defaults.saturation
                    || blur.glass != defaults.glass,
            );
        }
    };
    show();
    ctx.store.subscribe(show);

    group.upcast()
}
