//! bento's own shell in bento: loads the shell's script, moves its scene on, draws it per output
//! with the Vulkan renderer, and hands the result and its glass to bento's compositing.

use std::collections::HashMap;
use std::time::{Instant, SystemTime};

use smithay::backend::allocator::Format as DrmFormat;
use smithay::backend::drm::DrmNode;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::ImportDma;
use smithay::utils::Transform;

use super::script::{self, Shell};
use super::vk::{Renderer, Target};
use super::{Item, Material, Theme, CLICK};
use crate::render_helpers::background_effect::BackgroundEffectElement;
use crate::render_helpers::framebuffer_effect::FramebufferEffect;
use crate::render_helpers::glass::{self, GlassFrame};
use crate::render_helpers::primary_gpu_texture::PrimaryGpuTextureRenderElement;
use crate::render_helpers::texture::{TextureBuffer, TextureRenderElement};

/// How close islands come for their glass to melt together.
const MELT: f32 = 24.;
/// Images per output, so the scene never draws into one bento's compositing is still reading.
const TARGETS: usize = 3;

struct OutputScene {
    targets: Vec<(Target, GlesTexture)>,
    next: usize,
    /// What was last drawn, to show.
    shown: Option<TextureBuffer<GlesTexture>>,
    /// The glass under what was last drawn.
    glass: Vec<([f32; 4], [f32; 4])>,
    effect: FramebufferEffect,
    /// The generation of the scene last drawn, and at what scale.
    drawn: Option<(u64, f64)>,
}

#[derive(Default)]
pub struct SceneHost {
    shell: Option<Shell>,
    /// When the user's shell file was last loaded, or failed to.
    loaded_mtime: Option<Option<SystemTime>>,
    renderer: Option<Renderer>,
    renderer_failed: bool,
    outputs: HashMap<String, OutputScene>,
    last_advance: Option<Instant>,
    /// Goes up whenever the scene changed or moved.
    generation: u64,
    moving: bool,
    sent_workspaces: Option<serde_json::Value>,
    known_outputs: Vec<String>,
}

impl std::fmt::Debug for SceneHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SceneHost")
    }
}

impl SceneHost {
    /// Loads the shell, again when its file changed, onto `outputs`, each with its logical size.
    /// A shell that fails to load leaves the last one.
    pub fn ensure_loaded(&mut self, outputs: &[(String, (f32, f32))]) {
        let mtime = script::user_mtime();
        if self.loaded_mtime != Some(mtime) {
            self.loaded_mtime = Some(mtime);
            match Shell::load(outputs) {
                Ok(shell) => {
                    self.shell = Some(shell);
                    self.known_outputs = outputs.iter().map(|(n, _)| n.clone()).collect();
                    self.sent_workspaces = None;
                    self.generation += 1;
                }
                Err(err) => warn!("error loading the shell: {err:?}"),
            }
        }

        let Some(shell) = &self.shell else { return };
        let mut scene = shell.scene.borrow_mut();
        for (name, size) in outputs {
            scene.set_output(name, *size);
        }
        let gone: Vec<String> = self
            .known_outputs
            .iter()
            .filter(|name| !outputs.iter().any(|(n, _)| n == *name))
            .cloned()
            .collect();
        for name in &gone {
            scene.remove_output(name);
            self.outputs.remove(name);
        }
        let added: Vec<String> = outputs
            .iter()
            .map(|(n, _)| n.clone())
            .filter(|n| !self.known_outputs.contains(n))
            .collect();
        drop(scene);
        if !added.is_empty() || !gone.is_empty() {
            self.known_outputs = outputs.iter().map(|(n, _)| n.clone()).collect();
            shell.send(&serde_json::json!({ "type": "outputs", "outputs": added }));
        }
    }

    /// Tells the shell the workspaces, by output, if they changed.
    pub fn set_workspaces(&mut self, outputs: serde_json::Value) {
        if self.sent_workspaces.as_ref() == Some(&outputs) {
            return;
        }
        if let Some(shell) = &self.shell {
            shell.send(&serde_json::json!({ "type": "workspaces", "outputs": outputs }));
        }
        self.sent_workspaces = Some(outputs);
    }

    /// When the shell next needs to run, for its timers.
    pub fn next_timer(&self) -> Option<Instant> {
        self.shell.as_ref()?.next_timer()
    }

    /// Runs due timers, moves the scene on to `now`, and says whether it's still moving.
    pub fn advance(&mut self, now: Instant, slowdown: Option<f64>) -> bool {
        let Some(shell) = &self.shell else {
            return false;
        };
        shell.run_timers(now);
        let (dark, accent) = glass::style();
        let mut scene = shell.scene.borrow_mut();
        scene.set_theme(Theme::new(dark, accent));
        scene.lay_out();
        let dt = self
            .last_advance
            .replace(now)
            .map_or(0., |last| now.saturating_duration_since(last).as_secs_f64())
            .min(0.1);
        let moving = scene.advance(dt, slowdown);
        if moving || self.moving || scene.take_changed() {
            self.generation += 1;
        }
        self.moving = moving;
        moving
    }

    /// Clicks at `point` on output `output`, if the shell takes clicks there, and says whether it
    /// did.
    pub fn click(&self, output: &str, point: [f32; 2]) -> bool {
        let Some(shell) = &self.shell else {
            return false;
        };
        let hit = shell.scene.borrow().hit(output, point, CLICK);
        match hit {
            Some(id) => {
                shell.send(&serde_json::json!({ "type": "click", "id": id }));
                true
            }
            None => false,
        }
    }

    /// Whether the shell covers `point` on output `output`, so it takes the pointer there.
    pub fn covers(&self, output: &str, point: [f32; 2]) -> bool {
        self.shell
            .as_ref()
            .is_some_and(|shell| shell.scene.borrow().covers(output, point))
    }

    /// Draws output `output`'s scene, `size` physical pixels at `scale`, if it changed.
    pub fn prepare(
        &mut self,
        renderer: &mut GlesRenderer,
        node: Option<DrmNode>,
        output: &str,
        size: (u32, u32),
        scale: f64,
    ) {
        let Some(shell) = &self.shell else { return };
        if self.renderer.is_none() && !self.renderer_failed {
            let formats: Vec<DrmFormat> = renderer.dmabuf_formats().iter().copied().collect();
            match Renderer::new(node, &formats) {
                Ok(new) => self.renderer = Some(new),
                Err(err) => {
                    warn!("error starting the scene's Vulkan renderer: {err:?}");
                    self.renderer_failed = true;
                }
            }
        }
        let Some(vk) = &mut self.renderer else { return };

        let state = self
            .outputs
            .entry(String::from(output))
            .or_insert_with(|| OutputScene {
                targets: Vec::new(),
                next: 0,
                shown: None,
                glass: Vec::new(),
                effect: FramebufferEffect::new(),
                drawn: None,
            });
        if state.drawn == Some((self.generation, scale)) {
            return;
        }
        if state.targets.first().is_some_and(|(t, _)| t.size != size) {
            for (target, _) in state.targets.drain(..) {
                vk.destroy_target(target);
            }
            state.shown = None;
        }
        if state.targets.is_empty() {
            for _ in 0..TARGETS {
                let target = match vk.create_target(size) {
                    Ok(target) => target,
                    Err(err) => {
                        warn!("error making the scene's image: {err:?}");
                        return;
                    }
                };
                let texture = match renderer.import_dmabuf(&target.dmabuf, None) {
                    Ok(texture) => texture,
                    Err(err) => {
                        warn!("error showing the scene's image: {err:?}");
                        vk.destroy_target(target);
                        return;
                    }
                };
                state.targets.push((target, texture));
            }
        }

        let items = shell.scene.borrow().draw(output);
        state.glass = items
            .iter()
            .filter_map(|item| match item {
                Item::Shape {
                    rect,
                    radii,
                    material: Material::Glass | Material::Acrylic | Material::Mica,
                    opacity,
                    ..
                } if *opacity > 0.01 => Some((*rect, *radii)),
                _ => None,
            })
            .collect();

        let (target, texture) = &state.targets[state.next];
        if let Err(err) = vk.render(target, &items, scale as f32) {
            warn!("error drawing the scene: {err:?}");
            return;
        }
        state.shown = Some(TextureBuffer::from_texture(
            renderer,
            texture.clone(),
            scale,
            Transform::Normal,
            Vec::new(),
        ));
        state.next = (state.next + 1) % state.targets.len();
        state.drawn = Some((self.generation, scale));
    }

    /// Pushes what output `output`'s scene shows, over its glass.
    pub fn render_content(
        &self,
        output: &str,
        push: &mut dyn FnMut(PrimaryGpuTextureRenderElement),
    ) {
        let Some(buffer) = self.outputs.get(output).and_then(|s| s.shown.as_ref()) else {
            return;
        };
        push(PrimaryGpuTextureRenderElement(
            TextureRenderElement::from_texture_buffer(
                buffer.clone(),
                (0., 0.),
                1.,
                None,
                None,
                Kind::Unspecified,
            ),
        ));
    }

    /// Pushes the glass under output `output`'s scene.
    pub fn render_glass(
        &self,
        output: &str,
        scale: f64,
        blur: niri_config::Blur,
        frame: GlassFrame,
        push: &mut dyn FnMut(BackgroundEffectElement),
    ) {
        let Some(state) = self.outputs.get(output) else {
            return;
        };
        if !state.glass.is_empty() {
            glass::render_rects(
                &state.effect,
                None,
                &state.glass,
                MELT,
                scale,
                blur,
                frame,
                push,
            );
        }
    }
}
