//! A miniature screen showing how windows look with the current settings: gaps, reserved space at
//! the edges, corners, focus ring, border, shadow and title bars.

use std::f64::consts::PI;

use adw::prelude::*;
use gtk::cairo::Context;
use niri_config::{Color, Config};

use super::layout::every_window;
use crate::page::PageContext;

/// The screen the preview stands for, in logical pixels. A laptop-sized screen keeps gaps and
/// corners readable at preview size, where a large monitor would make them hair-thin.
const SCREEN: (f64, f64) = (1280., 800.);
const WIDTH: i32 = 420;
/// Height of an app's own title bar, in logical pixels.
const TITLE_BAR: f64 = 46.;

struct Look {
    gaps: f64,
    /// Left, right, top and bottom.
    struts: [f64; 4],
    radius: f64,
    title_bars: bool,
    background: Color,
    /// Width, and the colors for the focused window and the others.
    ring: Option<(f64, Color)>,
    border: Option<(f64, Color, Color)>,
    /// Softness, spread, offset and color.
    shadow: Option<(f64, f64, (f64, f64), Color)>,
}

fn look(c: &Config) -> Look {
    let layout = &c.layout;
    let struts = &layout.struts;
    Look {
        gaps: layout.gaps,
        struts: [struts.left.0, struts.right.0, struts.top.0, struts.bottom.0],
        radius: every_window(c, |rule| rule.geometry_corner_radius)
            .map_or(0., |radius| f64::from(radius.top_left)),
        title_bars: !c.prefer_no_csd,
        background: layout.background_color,
        ring: (!layout.focus_ring.off)
            .then_some((layout.focus_ring.width, layout.focus_ring.active_color)),
        border: (!layout.border.off).then_some((
            layout.border.width,
            layout.border.active_color,
            layout.border.inactive_color,
        )),
        shadow: layout.shadow.on.then_some((
            layout.shadow.softness,
            layout.shadow.spread,
            (layout.shadow.offset.x.0, layout.shadow.offset.y.0),
            layout.shadow.color,
        )),
    }
}

pub fn build(ctx: &PageContext) -> gtk::Widget {
    let area = gtk::DrawingArea::builder()
        .content_width(WIDTH)
        .content_height(WIDTH * 10 / 16)
        .halign(gtk::Align::Center)
        .css_classes(["preview"])
        .build();

    let store = ctx.store.clone();
    area.set_draw_func(move |_, cr, width, height| {
        let dark = adw::StyleManager::default().is_dark();
        draw(
            cr,
            f64::from(width),
            f64::from(height),
            &look(&store.config()),
            dark,
        );
    });

    ctx.anchor("layout.preview", &area);
    let redraw = area.downgrade();
    ctx.store.subscribe(move || {
        if let Some(area) = redraw.upgrade() {
            area.queue_draw();
        }
    });
    let redraw = area.downgrade();
    adw::StyleManager::default().connect_dark_notify(move |_| {
        if let Some(area) = redraw.upgrade() {
            area.queue_draw();
        }
    });

    area.upcast()
}

fn draw(cr: &Context, width: f64, height: f64, look: &Look, dark: bool) {
    // The monitor: a bezel around the screen.
    let bezel = 6.;
    rounded(cr, 0., 0., width, height, 12.);
    set(
        cr,
        if dark {
            (0.08, 0.08, 0.09, 1.)
        } else {
            (0.16, 0.16, 0.18, 1.)
        },
    );
    let _ = cr.fill();

    let (x0, y0) = (bezel, bezel);
    let (sw, sh) = (width - 2. * bezel, height - 2. * bezel);
    let s = sw / SCREEN.0;

    cr.save().ok();
    rounded(cr, x0, y0, sw, sh, 7.);
    cr.clip();
    let [r, g, b, a] = look.background.to_array_unpremul();
    if a < 0.05 {
        set(cr, (0.22, 0.24, 0.3, 1.));
    } else {
        set(cr, (f64::from(r), f64::from(g), f64::from(b), 1.));
    }
    let _ = cr.paint();

    // Reserved space along the edges, which windows stay out of.
    let [left, right, top, bottom] = look.struts.map(|x| x.max(0.) * s);
    set(cr, (0.55, 0.65, 1., 0.18));
    for (x, y, w, h) in [
        (x0, y0, left, sh),
        (x0 + sw - right, y0, right, sh),
        (x0, y0, sw, top),
        (x0, y0 + sh - bottom, sw, bottom),
    ] {
        if w > 0. && h > 0. {
            cr.rectangle(x, y, w, h);
            let _ = cr.fill();
        }
    }

    // Two windows side by side in what's left, the first one focused.
    let gap = look.gaps * s;
    let (ax, ay) = (x0 + left + gap, y0 + top + gap);
    let (aw, ah) = (sw - left - right - 2. * gap, sh - top - bottom - 2. * gap);
    if aw > 4. && ah > 4. {
        let first = ((aw - gap) * 0.58).max(1.);
        window(cr, ax, ay, first, ah, s, look, dark, true);
        window(
            cr,
            ax + first + gap,
            ay,
            aw - gap - first,
            ah,
            s,
            look,
            dark,
            false,
        );
    }
    cr.restore().ok();
}

#[allow(clippy::too_many_arguments)]
fn window(
    cr: &Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    s: f64,
    look: &Look,
    dark: bool,
    focused: bool,
) {
    if w <= 1. || h <= 1. {
        return;
    }
    let radius = look.radius * s;

    // A soft shadow, approximated with stacked translucent outlines.
    if let Some((softness, spread, (dx, dy), color)) = look.shadow {
        let [r, g, b, a] = color.to_array_unpremul().map(f64::from);
        let steps = 10;
        for i in 0..steps {
            let t = f64::from(i) / f64::from(steps);
            let grow = (spread + softness * t) * s;
            rounded(
                cr,
                x + dx * s - grow,
                y + dy * s - grow,
                w + 2. * grow,
                h + 2. * grow,
                radius + grow,
            );
            set(cr, (r, g, b, a * (1. - t) / f64::from(steps) * 2.));
            let _ = cr.fill();
        }
    }

    // The focus ring sits outside the window and only marks the focused one.
    if let (Some((width, color)), true) = (look.ring, focused) {
        let ring = width * s;
        rounded(
            cr,
            x - ring,
            y - ring,
            w + 2. * ring,
            h + 2. * ring,
            radius + ring,
        );
        set_color(cr, color);
        let _ = cr.fill();
    }

    // The border takes its space from the window.
    let (mut x, mut y, mut w, mut h, mut radius) = (x, y, w, h, radius);
    if let Some((width, active, inactive)) = look.border {
        rounded(cr, x, y, w, h, radius);
        set_color(cr, if focused { active } else { inactive });
        let _ = cr.fill();
        let b = width * s;
        (x, y, w, h, radius) = (x + b, y + b, w - 2. * b, h - 2. * b, (radius - b).max(0.));
        if w <= 1. || h <= 1. {
            return;
        }
    }

    cr.save().ok();
    rounded(cr, x, y, w, h, radius);
    cr.clip();
    set(
        cr,
        if dark {
            (0.17, 0.17, 0.19, 1.)
        } else {
            (1., 1., 1., 1.)
        },
    );
    let _ = cr.paint();

    let mut top = y;
    if look.title_bars {
        let bar = TITLE_BAR * s;
        cr.rectangle(x, y, w, bar);
        set(
            cr,
            if dark {
                (0.22, 0.22, 0.25, 1.)
            } else {
                (0.92, 0.92, 0.93, 1.)
            },
        );
        let _ = cr.fill();
        top += bar;
    }

    // A few lines of "text".
    let ink = if dark {
        (1., 1., 1., 0.1)
    } else {
        (0., 0., 0., 0.08)
    };
    let line = 10. * s;
    let mut ly = top + 3. * line;
    for fraction in [0.7, 0.5, 0.62, 0.4] {
        if ly + line > y + h {
            break;
        }
        rounded(
            cr,
            x + 3. * line,
            ly,
            (w - 6. * line) * fraction,
            line,
            line / 2.,
        );
        set(cr, ink);
        let _ = cr.fill();
        ly += 2.2 * line;
    }
    cr.restore().ok();
}

fn rounded(cr: &Context, x: f64, y: f64, w: f64, h: f64, radius: f64) {
    let r = radius.min(w / 2.).min(h / 2.).max(0.);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2., 0.);
    cr.arc(x + w - r, y + h - r, r, 0., PI / 2.);
    cr.arc(x + r, y + h - r, r, PI / 2., PI);
    cr.arc(x + r, y + r, r, PI, 3. * PI / 2.);
    cr.close_path();
}

fn set(cr: &Context, (r, g, b, a): (f64, f64, f64, f64)) {
    cr.set_source_rgba(r, g, b, a);
}

fn set_color(cr: &Context, color: Color) {
    let [r, g, b, a] = color.to_array_unpremul().map(f64::from);
    cr.set_source_rgba(r, g, b, a);
}
