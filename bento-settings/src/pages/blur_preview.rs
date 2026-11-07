//! A piece of the screen at its real size, with a pane of the blur material over the wallpaper.
//!
//! It draws with bento's own shaders: the background is blurred by the same passes with the same
//! offset, and the pane is drawn by the glass or plain shader with the uniforms bento gives it.
//! The pane can be dragged around, to watch the glass bend what's behind it, and on glass a handle
//! beside it for each light that's on moves that light.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::OnceLock;

use adw::prelude::*;
use glow::HasContext;
use gtk::gdk_pixbuf::Pixbuf;
use gtk::{gdk, gio, glib};
use niri_config::{Blur, CornerRadius, Glass, Material, MAX_GLASS_LIGHTS};

use super::blur_live::{Live, Unsaved};
use super::layout::every_window;
use crate::model::{Input, Setting};
use crate::page::{keep_alive, PageContext};
use crate::store::Store;

/// The preview's height, in logical pixels.
const HEIGHT: i32 = 240;
/// The widest the preview gets, in logical pixels. The wallpaper is kept for this much of the
/// middle of the screen.
const MAX_WIDTH: i32 = 800;
/// The pane: a small window, in logical pixels.
const PANE: (f64, f64) = (240., 150.);
/// The screen the wallpaper covers when the monitor isn't known, in logical pixels.
const SCREEN: (i32, i32) = (1920, 1080);
/// A light's handle, in logical pixels.
const LIGHT: i32 = 28;
/// The space between the pane and a light's handle, in logical pixels.
const LIGHT_GAP: f64 = 8.;

const GLASS: &str = include_str!("../../../src/render_helpers/shaders/glass.frag");
/// bento's `postprocess_and_clip` program.
const PLAIN: &str = concat!(
    include_str!("../../../src/render_helpers/shaders/clipped_surface.frag"),
    include_str!("../../../src/render_helpers/shaders/rounding_alpha.frag"),
    include_str!("../../../src/render_helpers/shaders/postprocess.frag"),
);
const BLUR_VERTEX: &str = include_str!("../../../src/render_helpers/shaders/blur.vert");
const BLUR_DOWN: &str = include_str!("../../../src/render_helpers/shaders/blur_down.frag");
const BLUR_UP: &str = include_str!("../../../src/render_helpers/shaders/blur_up.frag");

/// Feeds the texture shaders `v_coords` across the whole target, with the top row at 0. GL counts
/// rows from the bottom, so drawing on screen flips.
const VERTEX: &str = "#version 100
attribute vec2 vert;
uniform float flip;
varying vec2 v_coords;
void main() {
    v_coords = vert;
    vec2 position = vert * 2.0 - 1.0;
    gl_Position = vec4(position.x, position.y * flip, 0.0, 1.0);
}
";

/// Draws part of a texture.
const COPY: &str = "#version 100
precision highp float;
uniform sampler2D tex;
// x, y, width and height of the part, in texture coordinates.
uniform vec4 crop;
varying vec2 v_coords;
void main() {
    gl_FragColor = texture2D(tex, crop.xy + v_coords * crop.zw);
}
";

/// The picture shown without a wallpaper: a wash of color under fine checks and thin rings, so
/// that both the blur and the bending show.
const PATTERN: &str = "#version 100
precision highp float;
// The preview's size, in logical pixels.
uniform vec2 size;
uniform float dark;
varying vec2 v_coords;
void main() {
    vec2 p = (v_coords - 0.5) * size;
    float t = clamp(0.5 + (p.x + 0.6 * p.y) / 900.0, 0.0, 1.0);
    vec3 warm = mix(vec3(0.98, 0.62, 0.45), vec3(0.62, 0.22, 0.46), dark);
    vec3 cool = mix(vec3(0.42, 0.66, 0.98), vec3(0.12, 0.24, 0.55), dark);
    vec3 color = mix(cool, warm, t);
    vec2 cell = floor(p / 16.0);
    color *= mix(0.55, 1.0, mod(cell.x + cell.y, 2.0));
    float ring = abs(mod(length(p), 40.0) - 20.0);
    color = mix(color, vec3(1.0), (1.0 - smoothstep(0.5, 1.5, ring)) * 0.85);
    gl_FragColor = vec4(color, 1.0);
}
";

/// Builds the preview. Dragging a light's handle saves its angle, the setting in `light_angles` at
/// its place.
pub fn build(
    ctx: &PageContext,
    live: &Live,
    light_angles: [&'static Setting; MAX_GLASS_LIGHTS],
) -> gtk::Widget {
    let area = gtk::GLArea::builder()
        .allowed_apis(gdk::GLAPI::GLES)
        .hexpand(true)
        .build();
    let overlay = gtk::Overlay::builder().child(&area).build();
    let lights: Vec<gtk::Image> = (1..=MAX_GLASS_LIGHTS)
        .map(|n| {
            let light = gtk::Image::builder()
                .icon_name("ph-sun-symbolic")
                .width_request(LIGHT)
                .height_request(LIGHT)
                .tooltip_text(format!("Light {n}"))
                .css_classes(["blur-light"])
                .build();
            light.set_cursor_from_name(Some("grab"));
            overlay.add_overlay(&light);
            light
        })
        .collect();
    let fallback = gtk::Label::builder()
        .label("Preview Unavailable")
        .css_classes(["dim-label"])
        .build();
    let stack = gtk::Stack::builder()
        .height_request(HEIGHT)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["blur-preview"])
        .build();
    stack.add_named(&overlay, Some("preview"));
    stack.add_named(&fallback, Some("fallback"));

    let preview = Rc::new(Preview {
        area: area.clone(),
        overlay: overlay.clone(),
        lights,
        stack: stack.clone(),
        store: ctx.store.clone(),
        gl: RefCell::new(None),
        center: Cell::new((0.5, 0.5)),
        picture: RefCell::new(None),
        wanted: RefCell::new(None),
        live: live.clone(),
    });

    {
        let preview = Rc::downgrade(&preview);
        area.connect_realize(move |area| {
            if let Some(preview) = preview.upgrade() {
                preview.realize(area);
            }
        });
    }
    {
        let preview = Rc::downgrade(&preview);
        area.connect_unrealize(move |area| {
            area.make_current();
            if let Some(gl) = preview.upgrade().and_then(|p| p.gl.take()) {
                gl.destroy();
            }
        });
    }
    {
        let preview = Rc::downgrade(&preview);
        area.connect_render(move |_, _| {
            if let Some(preview) = preview.upgrade() {
                if let Err(err) = preview.render() {
                    preview.fail(&err);
                }
            }
            glib::Propagation::Stop
        });
    }
    {
        let preview = Rc::downgrade(&preview);
        area.connect_scale_factor_notify(move |_| {
            if let Some(preview) = preview.upgrade() {
                preview.update_picture();
            }
        });
    }
    {
        let preview = Rc::downgrade(&preview);
        overlay.connect_get_child_position(move |_, widget| {
            let preview = preview.upgrade()?;
            let i = preview.lights.iter().position(|light| light == widget)?;
            let (x, y) = preview.light_position(i);
            let half = f64::from(LIGHT) / 2.;
            Some(gdk::Rectangle::new(
                (x - half).round() as i32,
                (y - half).round() as i32,
                LIGHT,
                LIGHT,
            ))
        });
    }
    drag(&preview);
    drag_light(&preview, ctx, light_angles);

    let redraw = Rc::downgrade(&preview);
    ctx.store.subscribe(move || {
        if let Some(preview) = redraw.upgrade() {
            preview.update_picture();
        }
    });
    let redraw = Rc::downgrade(&preview);
    live.connect(move |_, _| {
        if let Some(preview) = redraw.upgrade() {
            preview.changed();
        }
    });
    preview.changed();
    let redraw = Rc::downgrade(&preview);
    adw::StyleManager::default().connect_dark_notify(move |_| {
        if let Some(preview) = redraw.upgrade() {
            preview.area.queue_render();
        }
    });

    keep_alive(&stack, preview);
    stack.upcast()
}

/// Dragging moves the pane. Pressing beside it brings it under the pointer first.
fn drag(preview: &Rc<Preview>) {
    let area = &preview.area;
    let start = Rc::new(Cell::new((0., 0.)));
    let drag = gtk::GestureDrag::new();
    {
        let (preview, start) = (Rc::downgrade(preview), start.clone());
        drag.connect_drag_begin(move |gesture, x, y| {
            let Some(preview) = preview.upgrade() else {
                return;
            };
            if !preview.over_pane(x, y) {
                preview.move_pane(x, y);
            }
            let (px, py, pw, ph) = preview.pane();
            start.set((px + pw / 2., py + ph / 2.));
            gesture.set_state(gtk::EventSequenceState::Claimed);
            preview.area.set_cursor_from_name(Some("grabbing"));
        });
    }
    {
        let preview = Rc::downgrade(preview);
        drag.connect_drag_update(move |_, dx, dy| {
            if let Some(preview) = preview.upgrade() {
                let (x, y) = start.get();
                preview.move_pane(x + dx, y + dy);
            }
        });
    }
    drag.connect_drag_end(|gesture, _, _| {
        if let Some(widget) = gesture.widget() {
            widget.set_cursor_from_name(Some("grab"));
        }
    });
    area.add_controller(drag);

    let motion = gtk::EventControllerMotion::new();
    let hover = Rc::downgrade(preview);
    motion.connect_motion(move |_, x, y| {
        if let Some(preview) = hover.upgrade() {
            let over = preview.over_pane(x, y);
            preview.area.set_cursor_from_name(over.then_some("grab"));
        }
    });
    area.add_controller(motion);
}

/// Dragging a light's handle points that light from the pane's middle towards the pointer.
fn drag_light(
    preview: &Rc<Preview>,
    ctx: &PageContext,
    settings: [&'static Setting; MAX_GLASS_LIGHTS],
) {
    let start = Rc::new(Cell::new((0., 0.)));
    // The light whose handle is being dragged.
    let dragged = Rc::new(Cell::new(0));
    let drag = gtk::GestureDrag::new();
    {
        let (preview, start, dragged) = (Rc::downgrade(preview), start.clone(), dragged.clone());
        drag.connect_drag_begin(move |gesture, x, y| {
            let Some(preview) = preview.upgrade() else {
                return;
            };
            let picked = preview.overlay.pick(x, y, gtk::PickFlags::DEFAULT);
            let i = picked.and_then(|widget| {
                preview.lights.iter().position(|light| {
                    let light = light.upcast_ref::<gtk::Widget>();
                    widget == *light || widget.is_ancestor(light)
                })
            });
            let Some(i) = i else {
                gesture.set_state(gtk::EventSequenceState::Denied);
                return;
            };
            dragged.set(i);
            start.set(preview.light_position(i));
            gesture.set_state(gtk::EventSequenceState::Claimed);
            preview.lights[i].set_cursor_from_name(Some("grabbing"));
        });
    }
    {
        let (preview, store, dragged) =
            (Rc::downgrade(preview), ctx.store.clone(), dragged.clone());
        drag.connect_drag_update(move |_, dx, dy| {
            let Some(preview) = preview.upgrade() else {
                return;
            };
            let (i, setting) = (dragged.get(), settings[dragged.get()]);
            let (x, y) = start.get();
            let (px, py, pw, ph) = preview.pane();
            let (dx, dy) = (x + dx - (px + pw / 2.), y + dy - (py + ph / 2.));
            if dx.hypot(dy) < 1. {
                return;
            }
            // Degrees clockwise from the top, in whole degrees like the setting.
            let angle = dx.atan2(-dy).to_degrees().round().rem_euclid(360.);
            let light = preview.look().blur.glass.lights[i];
            if light.is_none_or(|light| angle == light.angle) {
                return;
            }
            preview.live.show(Unsaved::Setting(setting.path, angle));
            store.edit(setting.path, move |doc| {
                setting.write(doc, Input::Number(angle))
            });
        });
    }
    {
        let preview = Rc::downgrade(preview);
        drag.connect_drag_end(move |_, _, _| {
            if let Some(preview) = preview.upgrade() {
                preview.lights[dragged.get()].set_cursor_from_name(Some("grab"));
            }
        });
    }
    preview.overlay.add_controller(drag);
}

struct Preview {
    area: gtk::GLArea,
    overlay: gtk::Overlay,
    /// The lights' handles, one for each place in glass's list.
    lights: Vec<gtk::Image>,
    stack: gtk::Stack,
    store: Store,
    gl: RefCell<Option<Gl>>,
    /// The pane's middle, as a fraction of the preview's size.
    center: Cell<(f64, f64)>,
    /// The wallpaper, or `None` for the built-in picture.
    picture: RefCell<Option<Rc<Picture>>>,
    /// The wallpaper being shown or loaded.
    wanted: RefCell<Option<Wanted>>,
    /// The settings, with the changes not saved yet.
    live: Live,
}

/// A wallpaper, with the screen's logical size and the scale it's loaded for.
type Wanted = (PathBuf, (i32, i32), i32);

/// What the preview draws, from the config.
struct Look {
    blur: Blur,
    radius: CornerRadius,
    dark: bool,
}

impl Preview {
    fn realize(self: &Rc<Self>, area: &gtk::GLArea) {
        area.make_current();
        if let Some(err) = area.error() {
            self.fail(&err.to_string());
            return;
        }
        match Gl::new() {
            Ok(gl) => {
                self.gl.replace(Some(gl));
            }
            Err(err) => {
                self.fail(&err);
                return;
            }
        }

        // The screen is known once the window shows on it.
        if let Some(surface) = area.native().and_then(|native| native.surface()) {
            let preview = Rc::downgrade(self);
            surface.connect_enter_monitor(move |_, _| {
                if let Some(preview) = preview.upgrade() {
                    preview.update_picture();
                }
            });
        }
        // Whatever was uploaded went with the old context.
        self.wanted.replace(None);
        self.update_picture();
    }

    fn fail(&self, err: &str) {
        glib::g_warning!("bento-settings", "blur preview: {err}");
        self.gl.replace(None);
        let stack = self.stack.clone();
        // Not while GTK is realizing or drawing the area.
        glib::idle_add_local_once(move || stack.set_visible_child_name("fallback"));
    }

    fn look(&self) -> Look {
        let blur = self.live.blur();
        let config = self.store.config();
        Look {
            blur,
            radius: every_window(&config, |rule| rule.geometry_corner_radius).unwrap_or_default(),
            dark: adw::StyleManager::default().is_dark(),
        }
    }

    /// The screen's size in logical pixels.
    fn screen(&self) -> (i32, i32) {
        self.area
            .native()
            .and_then(|native| native.surface())
            .and_then(|surface| self.area.display().monitor_at_surface(&surface))
            .map(|monitor| {
                let geometry = monitor.geometry();
                (geometry.width(), geometry.height())
            })
            .filter(|(w, h)| *w > 0 && *h > 0)
            .unwrap_or(SCREEN)
    }

    /// Loads the wallpaper if it changed, covering the screen the way bento does.
    fn update_picture(self: &Rc<Self>) {
        if !self.area.is_realized() {
            return;
        }
        let path = super::wallpaper::current(&self.store);
        let Some(path) = path else {
            if self.wanted.take().is_some() || self.picture.borrow().is_some() {
                self.picture.replace(None);
                self.area.queue_render();
            }
            return;
        };
        let scale = self.area.scale_factor();
        let wanted = (path, self.screen(), scale);
        if self.wanted.borrow().as_ref() == Some(&wanted) {
            return;
        }
        self.wanted.replace(Some(wanted.clone()));

        let preview = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let (path, (sw, sh), scale) = wanted.clone();
            let picture = gio::spawn_blocking(move || {
                decode(
                    &path,
                    (sw * scale, sh * scale),
                    (MAX_WIDTH * scale, HEIGHT * scale),
                )
            })
            .await
            .ok()
            .flatten();
            let Some(preview) = preview.upgrade() else {
                return;
            };
            // A newer wallpaper is on its way.
            if preview.wanted.borrow().as_ref() != Some(&wanted) {
                return;
            }
            preview.picture.replace(picture.map(Rc::new));
            preview.area.queue_render();
        });
    }

    /// Shows what the controls or the config changed.
    fn changed(&self) {
        let blur = self.look().blur;
        // Only glass has lights, and only those that are on have a handle.
        let glass = !blur.off && blur.material == Material::Glass;
        for (handle, light) in self.lights.iter().zip(blur.glass.lights) {
            handle.set_visible(glass && light.is_some_and(|light| light.on));
        }
        self.overlay.queue_allocate();
        self.area.queue_render();
    }

    /// The middle of the handle of the light at `i`, in logical pixels: beside the pane, towards
    /// the light, and inside the preview.
    fn light_position(&self, i: usize) -> (f64, f64) {
        let (px, py, pw, ph) = self.pane();
        let light = self.look().blur.glass.lights[i].unwrap_or_default();
        let angle = light.angle.to_radians();
        let direction = (angle.sin(), -angle.cos());
        let reach = beside(
            (pw / 2., ph / 2.),
            direction,
            LIGHT_GAP + f64::from(LIGHT) / 2.,
        );
        let (width, height) = (f64::from(self.area.width()), f64::from(self.area.height()));
        let margin = f64::from(LIGHT) / 2. + 4.;
        let x = px + pw / 2. + direction.0 * reach;
        let y = py + ph / 2. + direction.1 * reach;
        (
            x.clamp(margin, (width - margin).max(margin)),
            y.clamp(margin, (height - margin).max(margin)),
        )
    }

    /// The pane's x, y, width and height, in logical pixels.
    fn pane(&self) -> (f64, f64, f64, f64) {
        let (width, height) = (f64::from(self.area.width()), f64::from(self.area.height()));
        let pw = PANE.0.min(width - 24.).max(40.);
        let ph = PANE.1.min(height - 24.).max(40.);
        let (cx, cy) = self.center.get();
        let x = (cx * width - pw / 2.).clamp(0., (width - pw).max(0.));
        let y = (cy * height - ph / 2.).clamp(0., (height - ph).max(0.));
        (x, y, pw, ph)
    }

    fn over_pane(&self, x: f64, y: f64) -> bool {
        let (px, py, pw, ph) = self.pane();
        (px..px + pw).contains(&x) && (py..py + ph).contains(&y)
    }

    /// Moves the pane's middle to `x`, `y`, keeping it inside.
    fn move_pane(&self, x: f64, y: f64) {
        let (width, height) = (f64::from(self.area.width()), f64::from(self.area.height()));
        if width <= 0. || height <= 0. {
            return;
        }
        let (_, _, pw, ph) = self.pane();
        let cx = x.clamp(pw / 2., (width - pw / 2.).max(pw / 2.));
        let cy = y.clamp(ph / 2., (height - ph / 2.).max(ph / 2.));
        self.center.set((cx / width, cy / height));
        self.overlay.queue_allocate();
        self.area.queue_render();
    }

    fn render(&self) -> Result<(), String> {
        let mut gl = self.gl.borrow_mut();
        let Some(gl) = gl.as_mut() else {
            return Ok(());
        };
        let look = self.look();
        let scale = self.area.scale_factor();
        let (lw, lh) = (self.area.width(), self.area.height());
        let (w, h) = (lw * scale, lh * scale);
        if w <= 0 || h <= 0 {
            return Ok(());
        }

        let blur = !look.blur.off;
        let passes = usize::from(look.blur.passes.clamp(1, 31));
        gl.upload(self.picture.borrow().as_ref())?;
        gl.prepare(w, h, blur.then_some(passes))?;

        // What's behind, at the preview's size in physical pixels.
        let background = gl.background.as_ref().map(|t| t.texture);
        gl.bind_target(background, w, h);
        match &gl.wallpaper {
            Some((texture, picture)) => {
                let cw = (w as f32 / picture.width as f32).min(1.);
                let ch = (h as f32 / picture.height as f32).min(1.);
                let crop = [(1. - cw) / 2., (1. - ch) / 2., cw, ch];
                gl.draw(
                    gl.copy,
                    Some(*texture),
                    1.,
                    &[("crop", Uniform::Vec4(crop))],
                );
            }
            None => gl.draw(
                gl.pattern,
                None,
                1.,
                &[
                    ("size", Uniform::Vec2([lw as f32, lh as f32])),
                    ("dark", Uniform::Float(if look.dark { 1. } else { 0. })),
                ],
            ),
        }

        let behind = if blur {
            gl.blur(passes, look.blur.offset as f32);
            gl.steps[0].texture
        } else {
            gl.background.as_ref().unwrap().texture
        };

        self.area.attach_buffers();
        unsafe { gl.gl.viewport(0, 0, w, h) };
        let whole = [("crop", Uniform::Vec4([0., 0., 1., 1.]))];
        gl.draw(gl.copy, background, -1., &whole);

        unsafe {
            gl.gl.enable(glow::BLEND);
            gl.gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
        }
        let pane = self.pane();
        let (program, uniforms) = pane_uniforms(gl, &look, pane, (lw, lh), scale);
        gl.draw(program, Some(behind), -1., &uniforms);
        unsafe { gl.gl.disable(glow::BLEND) };
        Ok(())
    }
}

/// The pane's program and uniforms, as `background_effect.rs` and `glass.rs` set them.
fn pane_uniforms(
    gl: &Gl,
    look: &Look,
    (px, py, pw, ph): (f64, f64, f64, f64),
    (lw, lh): (i32, i32),
    scale: i32,
) -> (glow::Program, Vec<(&'static str, Uniform)>) {
    let (lw, lh) = (f64::from(lw), f64::from(lh));
    // From the preview's texture coordinates to the pane's [0, 1], and back.
    let input_to_geo = [lw / pw, 0., 0., 0., lh / ph, 0., -px / pw, -py / ph, 1.].map(|x| x as f32);
    let geo_to_input = [pw / lw, 0., 0., 0., ph / lh, 0., px / lw, py / lh, 1.].map(|x| x as f32);
    let (pw, ph) = (pw as f32, ph as f32);
    let radius = look.radius.expanded_by(1.).fit_to(pw, ph);

    let blur = !look.blur.off;
    let (noise, saturation) = if blur {
        (look.blur.noise as f32, look.blur.saturation as f32)
    } else {
        (0., 1.)
    };
    let mut uniforms = vec![
        ("niri_scale", Uniform::Float(scale as f32)),
        ("geo_size", Uniform::Vec2([pw, ph])),
        ("input_to_geo", Uniform::Mat3(input_to_geo)),
        ("noise", Uniform::Float(noise)),
        ("saturation", Uniform::Float(saturation)),
        ("bg_color", Uniform::Vec4([0.; 4])),
    ];

    if !(blur && look.blur.material == Material::Glass) {
        uniforms.push(("corner_radius", Uniform::Vec4(<[f32; 4]>::from(radius))));
        return (gl.plain, uniforms);
    }

    let glass = look.blur.glass;
    let (lights, sizes) = light_uniforms(&glass);
    uniforms.extend([
        ("geo_to_input", Uniform::Mat3(geo_to_input)),
        ("shape_count", Uniform::Float(1.)),
        ("shape0", Uniform::Vec4([0., 0., pw, ph])),
        ("radii0", Uniform::Vec4(<[f32; 4]>::from(radius))),
        ("bezel", Uniform::Float(glass.bezel as f32)),
        ("thickness", Uniform::Float(glass.thickness as f32)),
        (
            "refractive_index",
            Uniform::Float(glass.refractive_index as f32),
        ),
        ("dispersion", Uniform::Float(glass.dispersion as f32)),
        ("reflection", Uniform::Float(glass.reflection as f32)),
        ("sheen", Uniform::Float(glass.sheen as f32)),
        ("light_count", Uniform::Float(lights.len() as f32)),
        ("light_sizes", Uniform::Vec4(sizes)),
    ]);
    for (i, name) in ["light0", "light1", "light2", "light3"]
        .into_iter()
        .enumerate()
    {
        let light = lights.get(i).copied().unwrap_or_default();
        uniforms.push((name, Uniform::Vec4(light)));
    }
    for name in [
        "shape1", "shape2", "shape3", "shape4", "shape5", "shape6", "shape7", "radii1", "radii2",
        "radii3", "radii4", "radii5", "radii6", "radii7",
    ] {
        uniforms.push((name, Uniform::Vec4([0.; 4])));
    }
    (gl.glass, uniforms)
}

/// The lights that shine, each towards it over the screen with its intensity, and their sizes, as
/// `glass.rs` gives them.
fn light_uniforms(glass: &Glass) -> (Vec<[f32; 4]>, [f32; 4]) {
    let mut sizes = [0.; 4];
    let lights = glass
        .lights
        .iter()
        .flatten()
        .filter(|light| light.on && light.intensity > 0.)
        .enumerate()
        .map(|(i, light)| {
            sizes[i] = light.size as f32;
            let (angle, height) = (light.angle.to_radians(), light.height.to_radians());
            [
                (angle.sin() * height.cos()) as f32,
                (-angle.cos() * height.cos()) as f32,
                height.sin() as f32,
                light.intensity as f32,
            ]
        })
        .collect();
    (lights, sizes)
}

/// How far from the middle of a box with half its size `half`, along `direction`, a point is
/// `gap` outside it.
fn beside((hx, hy): (f64, f64), direction: (f64, f64), gap: f64) -> f64 {
    let (ax, ay) = (direction.0.abs(), direction.1.abs());
    // Past a side.
    let to_side = (hx + gap) / ax;
    if to_side * ay <= hy {
        return to_side;
    }
    let to_top = (hy + gap) / ay;
    if to_top * ax <= hx {
        return to_top;
    }
    // Round a corner: `gap` from it.
    let b = ax * hx + ay * hy;
    b + (b * b - (hx * hx + hy * hy - gap * gap)).max(0.).sqrt()
}

/// The middle of the wallpaper as it covers the screen, in physical pixels: premultiplied RGBA,
/// top row first.
struct Picture {
    width: i32,
    height: i32,
    pixels: Vec<u8>,
}

/// Scales the picture at `path` to cover `screen` and keeps up to `region` of its middle.
fn decode(path: &Path, screen: (i32, i32), region: (i32, i32)) -> Option<Picture> {
    let (_, width, height) = Pixbuf::file_info(path)?;
    if width <= 0 || height <= 0 {
        return None;
    }
    let cover =
        (f64::from(screen.0) / f64::from(width)).max(f64::from(screen.1) / f64::from(height));
    let pixbuf = Pixbuf::from_file_at_scale(
        path,
        (f64::from(width) * cover).round().max(1.) as i32,
        (f64::from(height) * cover).round().max(1.) as i32,
        false,
    )
    .ok()?;

    let (sw, sh) = (pixbuf.width(), pixbuf.height());
    let (rw, rh) = (region.0.min(sw), region.1.min(sh));
    let (x0, y0) = (((sw - rw) / 2) as usize, ((sh - rh) / 2) as usize);
    let channels = pixbuf.n_channels() as usize;
    let stride = pixbuf.rowstride() as usize;
    let bytes = pixbuf.read_pixel_bytes();
    let mut pixels = Vec::with_capacity(rw as usize * rh as usize * 4);
    for y in y0..y0 + rh as usize {
        let row = &bytes[y * stride..];
        for x in x0..x0 + rw as usize {
            let p = &row[x * channels..x * channels + channels];
            let a = if channels == 4 { p[3] } else { 255 };
            let premultiply = |c: u8| (u16::from(c) * u16::from(a) / 255) as u8;
            pixels.extend([premultiply(p[0]), premultiply(p[1]), premultiply(p[2]), a]);
        }
    }
    Some(Picture {
        width: rw,
        height: rh,
        pixels,
    })
}

#[derive(Debug, Clone, Copy)]
enum Uniform {
    Float(f32),
    Vec2([f32; 2]),
    Vec4([f32; 4]),
    Mat3([f32; 9]),
}

struct Target {
    texture: glow::Texture,
    width: i32,
    height: i32,
}

/// The GL side of the preview. Its methods expect the area's context to be current, which it is
/// while GTK realizes and renders the area.
struct Gl {
    gl: glow::Context,
    quad: glow::Buffer,
    framebuffer: glow::Framebuffer,
    copy: glow::Program,
    pattern: glow::Program,
    down: glow::Program,
    up: glow::Program,
    glass: glow::Program,
    plain: glow::Program,
    /// The wallpaper's texture and the picture it holds.
    wallpaper: Option<(glow::Texture, Rc<Picture>)>,
    /// What's behind the pane.
    background: Option<Target>,
    /// The blurred background followed by the blur's smaller steps, like bento's.
    steps: Vec<Target>,
}

/// libepoxy, which GTK loads GL with. It finds each function for whichever context is current.
fn epoxy() -> Result<&'static libloading::Library, String> {
    static EPOXY: OnceLock<Result<libloading::Library, String>> = OnceLock::new();
    EPOXY
        .get_or_init(|| {
            unsafe { libloading::Library::new("libepoxy.so.0") }.map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}

impl Gl {
    fn new() -> Result<Self, String> {
        let epoxy = epoxy()?;
        // SAFETY: epoxy's `epoxy_glFoo` symbols are variables holding the function pointers.
        let gl = unsafe {
            glow::Context::from_loader_function(|name| {
                epoxy
                    .get::<*const *const c_void>(format!("epoxy_{name}").as_bytes())
                    .map_or(std::ptr::null(), |symbol| **symbol)
            })
        };

        unsafe {
            let quad = gl.create_buffer()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(quad));
            let vertices: [f32; 8] = [0., 0., 1., 0., 0., 1., 1., 1.];
            let bytes: Vec<u8> = vertices.iter().flat_map(|v| v.to_ne_bytes()).collect();
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes, glow::STATIC_DRAW);
            let framebuffer = gl.create_framebuffer()?;

            let compile = |name: &str, vertex: &str, fragment: &str| {
                program(&gl, vertex, fragment).map_err(|err| format!("{name} shader: {err}"))
            };
            let copy = compile("copy", VERTEX, COPY)?;
            let pattern = compile("pattern", VERTEX, PATTERN)?;
            let down = compile("blur_down", BLUR_VERTEX, BLUR_DOWN)?;
            let up = compile("blur_up", BLUR_VERTEX, BLUR_UP)?;
            let glass = compile("glass", VERTEX, GLASS)?;
            let plain = compile("postprocess_and_clip", VERTEX, PLAIN)?;

            Ok(Self {
                gl,
                quad,
                framebuffer,
                copy,
                pattern,
                down,
                up,
                glass,
                plain,
                wallpaper: None,
                background: None,
                steps: Vec::new(),
            })
        }
    }

    fn destroy(mut self) {
        let gl = &self.gl;
        unsafe {
            for program in [
                self.copy,
                self.pattern,
                self.down,
                self.up,
                self.glass,
                self.plain,
            ] {
                gl.delete_program(program);
            }
            gl.delete_buffer(self.quad);
            gl.delete_framebuffer(self.framebuffer);
            if let Some((texture, _)) = self.wallpaper.take() {
                gl.delete_texture(texture);
            }
            for target in self
                .background
                .take()
                .into_iter()
                .chain(self.steps.drain(..))
            {
                gl.delete_texture(target.texture);
            }
        }
    }

    /// Puts the picture in the wallpaper texture, if it isn't there already.
    fn upload(&mut self, picture: Option<&Rc<Picture>>) -> Result<(), String> {
        let current = self.wallpaper.as_ref().map(|(_, p)| p);
        if current.map(Rc::as_ptr) == picture.map(Rc::as_ptr) {
            return Ok(());
        }
        if let Some((texture, _)) = self.wallpaper.take() {
            unsafe { self.gl.delete_texture(texture) };
        }
        if let Some(picture) = picture {
            let texture = texture(
                &self.gl,
                picture.width,
                picture.height,
                Some(&picture.pixels),
            )?;
            self.wallpaper = Some((texture, picture.clone()));
        }
        Ok(())
    }

    /// Makes the background and, with `passes`, the blur's textures for this size.
    fn prepare(&mut self, width: i32, height: i32, passes: Option<usize>) -> Result<(), String> {
        let gl = &self.gl;
        if self
            .background
            .as_ref()
            .is_some_and(|t| (t.width, t.height) != (width, height))
        {
            let old = self
                .background
                .take()
                .into_iter()
                .chain(self.steps.drain(..));
            for target in old {
                unsafe { gl.delete_texture(target.texture) };
            }
        }
        if self.background.is_none() {
            self.background = Some(Target {
                texture: texture(gl, width, height, None)?,
                width,
                height,
            });
        }

        let Some(passes) = passes else {
            return Ok(());
        };
        let (mut w, mut h) = (width, height);
        for i in 0..=passes {
            if self.steps.len() <= i {
                self.steps.push(Target {
                    texture: texture(gl, w, h, None)?,
                    width: w,
                    height: h,
                });
            }
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
        for target in self.steps.drain(passes + 1..) {
            unsafe { gl.delete_texture(target.texture) };
        }
        Ok(())
    }

    /// Draws into `texture`, or into the area's own buffer with `None`.
    fn bind_target(&self, texture: Option<glow::Texture>, width: i32, height: i32) {
        unsafe {
            self.gl
                .bind_framebuffer(glow::FRAMEBUFFER, Some(self.framebuffer));
            self.gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                texture,
                0,
            );
            self.gl.viewport(0, 0, width, height);
            self.gl.disable(glow::BLEND);
            self.gl.disable(glow::SCISSOR_TEST);
        }
    }

    /// Blurs the background into `steps[0]` the way `Blur::render` does: down through each step,
    /// then back up.
    fn blur(&self, passes: usize, offset: f32) {
        let background = self.background.as_ref().unwrap();
        let sources = std::iter::once(background).chain(&self.steps[1..]);
        for (src, dst) in sources.zip(&self.steps[1..=passes]) {
            self.bind_target(Some(dst.texture), dst.width, dst.height);
            // Going down, half_pixel is half of the destination's pixel.
            let half_pixel = [0.5 / dst.width as f32, 0.5 / dst.height as f32];
            self.draw(
                self.down,
                Some(src.texture),
                1.,
                &[
                    ("half_pixel", Uniform::Vec2(half_pixel)),
                    ("offset", Uniform::Float(offset)),
                ],
            );
        }
        for i in (0..passes).rev() {
            let (src, dst) = (&self.steps[i + 1], &self.steps[i]);
            self.bind_target(Some(dst.texture), dst.width, dst.height);
            // Going up, half_pixel is half of the source's pixel.
            let half_pixel = [0.5 / src.width as f32, 0.5 / src.height as f32];
            self.draw(
                self.up,
                Some(src.texture),
                1.,
                &[
                    ("half_pixel", Uniform::Vec2(half_pixel)),
                    ("offset", Uniform::Float(offset)),
                ],
            );
        }
    }

    /// Draws `texture` through `program` over the whole target. `flip` is -1 on screen.
    fn draw(
        &self,
        program: glow::Program,
        texture: Option<glow::Texture>,
        flip: f32,
        uniforms: &[(&str, Uniform)],
    ) {
        let gl = &self.gl;
        unsafe {
            gl.use_program(Some(program));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, texture);
            let common = [
                ("alpha", Uniform::Float(1.)),
                ("flip", Uniform::Float(flip)),
            ];
            let location = |name| gl.get_uniform_location(program, name);
            gl.uniform_1_i32(location("tex").as_ref(), 0);
            for (name, value) in common.iter().chain(uniforms) {
                let location = location(name);
                let location = location.as_ref();
                match *value {
                    Uniform::Float(x) => gl.uniform_1_f32(location, x),
                    Uniform::Vec2([x, y]) => gl.uniform_2_f32(location, x, y),
                    Uniform::Vec4([x, y, z, w]) => gl.uniform_4_f32(location, x, y, z, w),
                    Uniform::Mat3(m) => gl.uniform_matrix_3_f32_slice(location, false, &m),
                }
            }
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.quad));
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 0, 0);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.disable_vertex_attrib_array(0);
        }
    }
}

fn program(gl: &glow::Context, vertex: &str, fragment: &str) -> Result<glow::Program, String> {
    unsafe {
        let program = gl.create_program()?;
        for (kind, source) in [
            (glow::VERTEX_SHADER, vertex),
            (glow::FRAGMENT_SHADER, fragment),
        ] {
            let shader = gl.create_shader(kind)?;
            // smithay fills the defines line for its variants; the plain one has none.
            gl.shader_source(shader, &source.replace("//_DEFINES_", ""));
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                let log = gl.get_shader_info_log(shader);
                gl.delete_shader(shader);
                gl.delete_program(program);
                return Err(log);
            }
            gl.attach_shader(program, shader);
            gl.delete_shader(shader);
        }
        gl.bind_attrib_location(program, 0, "vert");
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            let log = gl.get_program_info_log(program);
            gl.delete_program(program);
            return Err(log);
        }
        Ok(program)
    }
}

fn texture(
    gl: &glow::Context,
    width: i32,
    height: i32,
    pixels: Option<&[u8]>,
) -> Result<glow::Texture, String> {
    unsafe {
        let texture = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        for (parameter, value) in [
            (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
            (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
            (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
            (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, value as i32);
        }
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA as i32,
            width,
            height,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(pixels),
        );
        Ok(texture)
    }
}
