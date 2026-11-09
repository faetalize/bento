//! bento's own shell in bento: loads the shell's script, moves its scene on, draws it per output
//! with the Vulkan renderer, and hands the result and its glass to bento's compositing.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Instant, SystemTime};

use smithay::backend::allocator::Format as DrmFormat;
use smithay::backend::drm::DrmNode;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::ImportDma;
use smithay::utils::Transform;

use super::script::{self, Shell};
use super::vk::{Renderer, Target};
use super::{Item, Material, NodeId, Theme, CLICK, HOVER, SCROLL};
use crate::render_helpers::background_effect::BackgroundEffectElement;
use crate::render_helpers::framebuffer_effect::FramebufferEffect;
use crate::render_helpers::glass::{self, GlassFrame};
use crate::render_helpers::primary_gpu_texture::PrimaryGpuTextureRenderElement;
use crate::render_helpers::texture::{TextureBuffer, TextureRenderElement};

/// What bento's own shell keeps along an output's edges, top, right, bottom and left, that the
/// output's working area leaves out, as layer surfaces' exclusive zones are.
#[derive(Default)]
struct Reserved(Mutex<[f64; 4]>);

/// What bento's own shell keeps along `output`'s edges.
pub fn reserved_on(output: &smithay::output::Output) -> [f64; 4] {
    output
        .user_data()
        .get::<Reserved>()
        .map_or([0.; 4], |r| *r.0.lock().unwrap())
}

/// Sets what bento's own shell keeps along `output`'s edges, and says whether it changed.
pub fn set_reserved_on(output: &smithay::output::Output, edges: [f64; 4]) -> bool {
    output
        .user_data()
        .insert_if_missing_threadsafe(Reserved::default);
    let reserved = output.user_data().get::<Reserved>().unwrap();
    let mut current = reserved.0.lock().unwrap();
    let changed = *current != edges;
    *current = edges;
    changed
}

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
    /// When the shell's files had last changed when they were loaded, or failed to.
    loaded_mtimes: Option<Vec<Option<SystemTime>>>,
    renderer: Option<Renderer>,
    renderer_failed: bool,
    /// Each output's scene.
    outputs: HashMap<String, OutputScene>,
    last_advance: Option<Instant>,
    /// Goes up whenever the scene changed or moved.
    generation: u64,
    moving: bool,
    sent_workspaces: Option<serde_json::Value>,
    sent_fullscreen: Option<serde_json::Value>,
    sent_minimap: Option<serde_json::Value>,
    known_outputs: Vec<String>,
    /// Where the pointer is over the shell: the output and the point on it.
    pointer: Option<(String, [f32; 2])>,
    /// The node the pointer is over that hears it come and go.
    hovered: Option<NodeId>,
    /// Where the pointer is on its output, whatever is under it, unless that's above the shell.
    pointer_at: Option<(String, [f32; 2])>,
    /// The nodes the pointer is in the place of that hear it near.
    near: Vec<NodeId>,
    /// The data program, `bento-shell --data`, while it runs, and its stdin.
    data: Option<(std::process::Child, std::process::ChildStdin)>,
    /// When to start the data program again after it went.
    data_retry: Option<Instant>,
    /// The last of each kind of data, but notifications, to tell a shell loaded again.
    data_state: HashMap<String, serde_json::Value>,
    /// Presses that haven't ended: by the button, touch or tool, what they pressed, if anything,
    /// and where.
    presses: HashMap<u32, (Option<NodeId>, String, [f32; 2])>,
}

impl Drop for SceneHost {
    fn drop(&mut self) {
        if let Some((mut child, _)) = self.data.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl std::fmt::Debug for SceneHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SceneHost")
    }
}

impl SceneHost {
    /// Loads the shell, again when its files changed, onto `outputs`, each with its logical size,
    /// with bento's own shell and widgets read from `dir` if it's given. A shell that fails to
    /// load leaves the last one.
    pub fn ensure_loaded(
        &mut self,
        outputs: &[(String, (f32, f32))],
        dir: Option<&Path>,
        style: serde_json::Value,
    ) {
        let mtimes = script::mtimes(dir);
        if self.loaded_mtimes.as_ref() != Some(&mtimes) {
            self.loaded_mtimes = Some(mtimes);
            match Shell::load(outputs, dir, style.clone()) {
                Ok(shell) => {
                    for data in self.data_state.values() {
                        shell.send(data);
                    }
                    self.shell = Some(shell);
                    self.known_outputs = outputs.iter().map(|(n, _)| n.clone()).collect();
                    self.sent_workspaces = None;
                    self.sent_fullscreen = None;
                    self.sent_minimap = None;
                    self.generation += 1;
                }
                Err(err) => warn!("error loading the shell: {err:?}"),
            }
        }

        let Some(shell) = &self.shell else { return };
        let restyled = shell.scene.borrow().style != style;
        if restyled {
            shell.scene.borrow_mut().style = style.clone();
            shell.send(&serde_json::json!({ "type": "style", "style": style }));
        }
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

    /// Whether the data program should be started now.
    pub fn wants_data_program(&self, now: Instant) -> bool {
        self.shell.is_some()
            && self.data.is_none()
            && self.data_retry.is_none_or(|retry| now >= retry)
    }

    /// The data program started.
    pub fn data_started(&mut self, child: std::process::Child, stdin: std::process::ChildStdin) {
        self.data = Some((child, stdin));
        self.data_retry = None;
        // What the shell asked of the last one, like the weather for a place, it asks again.
        if let Some(shell) = &self.shell {
            shell.send(&serde_json::json!({ "type": "data-ready" }));
        }
    }

    /// The data program went: it's started again in a few seconds.
    pub fn data_ended(&mut self, now: Instant) {
        if let Some((mut child, _)) = self.data.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.data_retry = Some(now + std::time::Duration::from_secs(5));
    }

    /// A line the data program said: hands it to the shell.
    pub fn data_line(&mut self, line: &str) {
        let Ok(data) = serde_json::from_str::<serde_json::Value>(line) else {
            return;
        };
        let kind = data["type"].as_str().unwrap_or_default();
        if kind != "notification" {
            self.data_state.insert(String::from(kind), data.clone());
        }
        if let Some(shell) = &self.shell {
            shell.send(&data);
        }
    }

    /// Hands the data program the shell's commands.
    pub fn send_commands(&mut self) {
        let Some(shell) = &self.shell else { return };
        let commands = shell.take_commands();
        let Some((_, stdin)) = &mut self.data else {
            return;
        };
        for command in commands {
            use std::io::Write as _;
            if writeln!(stdin, "{command}").is_err() {
                break;
            }
        }
    }

    /// The bento actions the shell asked to run since this was last asked.
    pub fn take_actions(&self) -> Vec<serde_json::Value> {
        self.shell
            .as_ref()
            .map(|shell| shell.take_actions())
            .unwrap_or_default()
    }

    /// Tells the shell that its launcher's shortcut was pressed, with the output that has the
    /// focus.
    pub fn toggle_launcher(&self, output: Option<String>) {
        if let Some(shell) = &self.shell {
            shell.send(&serde_json::json!({ "type": "launcher", "output": output }));
        }
    }

    /// Whether the pointer is over the shell.
    pub fn pointer_over(&self) -> bool {
        self.pointer.is_some()
    }

    /// The pointer moved to `at`, the output and the point on it, if it's over the shell, or off
    /// it. Tells nodes it comes to or leaves that hear it, and says how the pointer looks there,
    /// if it's over the shell.
    pub fn pointer_moved(&mut self, at: Option<(&str, [f32; 2])>) -> Option<String> {
        let Some(shell) = &self.shell else {
            return None;
        };
        self.pointer = at.map(|(name, point)| (String::from(name), point));
        let (hovered, cursor) = match at {
            Some((name, point)) => {
                let scene = shell.scene.borrow();
                (
                    scene.hit(name, point, HOVER),
                    Some(
                        scene
                            .cursor(name, point)
                            .unwrap_or_else(|| String::from("default")),
                    ),
                )
            }
            None => (None, None),
        };
        if hovered != self.hovered {
            if let Some(id) = self.hovered {
                shell.send(&serde_json::json!({ "type": "hover", "id": id, "inside": false }));
            }
            if let Some(id) = hovered {
                shell.send(&serde_json::json!({ "type": "hover", "id": id, "inside": true }));
            }
            self.hovered = hovered;
        }
        cursor
    }

    /// The pointer is at `at`, the output and the point on it, whatever is under it there, or
    /// with `None` nowhere on the desktop: over something above the shell, or gone. Tells the
    /// nodes that hear it near which of them it came to and left, and says whether any did.
    pub fn pointer_at(&mut self, at: Option<(&str, [f32; 2])>) -> bool {
        self.pointer_at = at.map(|(name, point)| (String::from(name), point));
        let dragged = self.drag();
        self.update_near() || dragged
    }

    /// Tells what's pressed and hears drags where the pointer went on its output, and says
    /// whether anything was told.
    fn drag(&self) -> bool {
        let (Some(shell), Some((name, point))) = (&self.shell, &self.pointer_at) else {
            return false;
        };
        let mut dragged = false;
        for (pressed, output, _) in self.presses.values() {
            let Some(id) = pressed else { continue };
            if output == name && shell.scene.borrow().takes(*id, super::DRAG) {
                shell.send(&serde_json::json!({
                    "type": "drag", "id": id, "x": point[0], "y": point[1],
                }));
                dragged = true;
            }
        }
        dragged
    }

    /// Tells the nodes that hear the pointer near which of them it's now in the place of, as it
    /// moves and as they do, and says whether that changed.
    fn update_near(&mut self) -> bool {
        let Some(shell) = &self.shell else {
            return false;
        };
        let near = match &self.pointer_at {
            Some((name, point)) => shell.scene.borrow().near(name, *point),
            None => Vec::new(),
        };
        if near == self.near {
            return false;
        }
        let before = std::mem::replace(&mut self.near, near.clone());
        for id in before.iter().filter(|id| !near.contains(id)) {
            shell.send(&serde_json::json!({ "type": "near", "id": id, "inside": false }));
        }
        for id in near.iter().filter(|id| !before.contains(id)) {
            shell.send(&serde_json::json!({ "type": "near", "id": id, "inside": true }));
        }
        true
    }

    /// A press by `button`, 1, 2 or 3 for left, middle and right, or a touch or a tool's tip as
    /// the left one, keyed by `key` until it ends, at `at` on the shell, or elsewhere with
    /// `None`. A press elsewhere tells the shell, so it can close what's open. Says whether the
    /// shell took it.
    pub fn press(&mut self, key: u32, button: u8, at: Option<(&str, [f32; 2])>) -> bool {
        let Some(shell) = &self.shell else {
            return false;
        };
        let Some((name, point)) = at else {
            shell.send(&serde_json::json!({ "type": "outside", "button": button }));
            return false;
        };
        let hit = shell.scene.borrow().hit(name, point, CLICK);
        if let Some(id) = hit {
            shell.send(&serde_json::json!({
                "type": "press", "id": id, "button": button, "x": point[0], "y": point[1],
            }));
        }
        self.presses.insert(key, (hit, String::from(name), point));
        true
    }

    /// The press keyed by `key` ended where it was pressed, as a touch does: a click on what it
    /// pressed. Says whether the press was the shell's.
    pub fn release_where_pressed(&mut self, key: u32, button: u8) -> bool {
        let Some((_, name, point)) = self.presses.get(&key).cloned() else {
            return false;
        };
        self.release(key, button, Some((&name, point)))
    }

    /// The press keyed by `key` ended, with what it pressed under `at`, the output and the point,
    /// or not over the shell: a click on what it pressed if it's still there. Says whether the
    /// press was the shell's.
    pub fn release(&mut self, key: u32, button: u8, at: Option<(&str, [f32; 2])>) -> bool {
        let Some((pressed, _, _)) = self.presses.remove(&key) else {
            return false;
        };
        let Some(shell) = &self.shell else {
            return true;
        };
        if let (Some(id), Some((name, point))) = (pressed, at) {
            if shell.scene.borrow().hit(name, point, CLICK) == Some(id) {
                shell.send(&serde_json::json!({
                    "type": "click", "id": id, "button": button, "x": point[0], "y": point[1],
                }));
            }
        }
        true
    }

    /// Scrolls by `delta` where the pointer is, and says whether the shell took it.
    pub fn scroll(&mut self, delta: [f64; 2]) -> bool {
        let (Some(shell), Some((name, point))) = (&self.shell, &self.pointer) else {
            return false;
        };
        let hit = shell.scene.borrow().hit(name, *point, SCROLL);
        if let Some(id) = hit {
            shell.send(&serde_json::json!({
                "type": "scroll", "id": id, "dx": delta[0], "dy": delta[1],
            }));
        }
        true
    }

    /// Whether the shell wants the keyboard.
    pub fn wants_keyboard(&self) -> bool {
        self.shell
            .as_ref()
            .is_some_and(|shell| shell.scene.borrow().wants_keyboard)
    }

    /// Gives the shell a key, while it holds the keyboard.
    pub fn key(&self, key: &crate::layout::tiling::host::EngineKey) {
        if let Some(shell) = &self.shell {
            let [shift, ctrl, alt, logo] = key.modifiers;
            shell.send(&serde_json::json!({
                "type": "key",
                "key": key.name,
                "text": key.text,
                "pressed": key.pressed,
                "repeat": key.repeat,
                "shift": shift, "ctrl": ctrl, "alt": alt, "super": logo,
            }));
        }
    }

    /// What the shell keeps along output `output`'s edges: top, right, bottom and left.
    pub fn reserved(&self, output: &str) -> [f32; 4] {
        self.shell
            .as_ref()
            .map_or([0.; 4], |shell| shell.scene.borrow().reserved(output))
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

    /// Tells the shell which outputs show a fullscreen window, by name, when that changes.
    pub fn set_fullscreen(&mut self, outputs: serde_json::Value) {
        if self.sent_fullscreen.as_ref() == Some(&outputs) {
            return;
        }
        if let Some(shell) = &self.shell {
            shell.send(&serde_json::json!({ "type": "fullscreen", "outputs": outputs }));
        }
        self.sent_fullscreen = Some(outputs);
    }

    /// Tells the shell where the tiles of each output's workspace sit around its view, by the
    /// output's name, when that changes.
    pub fn set_minimap(&mut self, outputs: serde_json::Value) {
        if self.sent_minimap.as_ref() == Some(&outputs) {
            return;
        }
        if let Some(shell) = &self.shell {
            shell.send(&serde_json::json!({ "type": "minimap", "outputs": outputs }));
        }
        self.sent_minimap = Some(outputs);
    }

    /// Tells the shell of something bento was asked for that's the shell's to do, like a
    /// shortcut for one of its panels, with the output that has the focus.
    pub fn event(&self, kind: &str, output: Option<String>) {
        if let Some(shell) = &self.shell {
            shell.send(&serde_json::json!({ "type": kind, "output": output }));
        }
    }

    /// When the shell next needs to run, for its timers.
    pub fn next_timer(&self) -> Option<Instant> {
        self.shell.as_ref()?.next_timer()
    }

    /// Runs due timers, moves the scene on to `now`, with springs that settle in `response`
    /// seconds where a node doesn't say, and says whether it's still moving.
    pub fn advance_with(&mut self, now: Instant, slowdown: Option<f64>, response: f64) -> bool {
        if let Some(shell) = &self.shell {
            shell.scene.borrow_mut().base_response = response;
        }
        self.advance(now, slowdown)
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
        let settled = scene.take_settled();
        drop(scene);
        for id in settled {
            shell.send(&serde_json::json!({ "type": "settled", "id": id }));
        }
        // What hears the pointer near may have moved under it, and what it does then is drawn.
        let heard = self.update_near();
        moving || heard
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
        icon_theme: &str,
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
        vk.set_icon_theme(icon_theme);

        let scene = shell.scene.borrow();
        {
            let items = scene.draw(output);
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
            state.drawn = Some((self.generation, scale));
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
            // Showing nothing, it takes no images, and nothing is drawn over what's below.
            if items.is_empty() {
                state.shown = None;
                for (target, _) in state.targets.drain(..) {
                    vk.destroy_target(target);
                }
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
                            break;
                        }
                    };
                    match renderer.import_dmabuf(&target.dmabuf, None) {
                        Ok(texture) => state.targets.push((target, texture)),
                        Err(err) => {
                            warn!("error showing the scene's image: {err:?}");
                            vk.destroy_target(target);
                            break;
                        }
                    }
                }
                if state.targets.is_empty() {
                    return;
                }
                state.next = 0;
            }

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
        }
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::layout::tiling::host::EngineKey;

    fn settle(host: &mut SceneHost, now: &mut Instant) {
        for _ in 0..30 {
            *now += Duration::from_millis(100);
            host.advance(*now, Some(1.));
        }
    }

    #[test]
    fn the_launcher_opens_where_the_focus_is_takes_typing_and_closes_on_escape() {
        let mut host = SceneHost::default();
        let outputs = [
            (String::from("DP-1"), (1920., 1080.)),
            (String::from("DP-2"), (2560., 1440.)),
        ];
        host.ensure_loaded(&outputs, None, serde_json::json!({ "gaps": 8 }));
        let mut now = Instant::now();
        settle(&mut host, &mut now);
        // How much the launcher draws on an output: what's below the islands.
        let overlay = |host: &SceneHost, output: &str| {
            host.shell
                .as_ref()
                .unwrap()
                .scene
                .borrow()
                .draw(output)
                .into_iter()
                .filter(|item| match item {
                    Item::Shape { rect, .. } | Item::Image { rect, .. } => rect[1] > 100.,
                    Item::Text { origin, .. } => origin[1] > 100.,
                })
                .count()
        };

        host.toggle_launcher(Some(String::from("DP-2")));
        settle(&mut host, &mut now);
        assert!(overlay(&host, "DP-2") > 0, "it opens on the focused output");
        // In the middle of the output at its tallest, 544 high, and only the search bar until
        // something's typed.
        let panel = glass_below_islands(&host, "DP-2").pop().unwrap();
        assert!((panel[1] - (1440. - 544.) / 2.).abs() < 1., "{panel:?}");
        assert!(panel[3] < 100., "only the bar: {panel:?}");
        assert_eq!(overlay(&host, "DP-1"), 0);
        assert!(host.wants_keyboard());

        let key = |name: &str, text: &str| EngineKey {
            name: String::from(name),
            text: String::from(text),
            modifiers: [false; 4],
            pressed: true,
            repeat: false,
        };
        // Something no app is called: still only the bar, with what's typed in it.
        let before = overlay(&host, "DP-2");
        for c in "zqxj".chars() {
            host.key(&key(&c.to_string(), &c.to_string()));
        }
        settle(&mut host, &mut now);
        assert!(
            overlay(&host, "DP-2") > 0 && overlay(&host, "DP-2") <= before,
            "only the bar"
        );

        host.key(&key("Escape", ""));
        settle(&mut host, &mut now);
        assert_eq!(overlay(&host, "DP-2"), 0, "Escape closes it");
        assert!(!host.wants_keyboard());

        // Opened again while it shrinks away, it's the same launcher turning around.
        host.toggle_launcher(Some(String::from("DP-2")));
        now += std::time::Duration::from_millis(30);
        host.advance(now, Some(1.));
        host.toggle_launcher(Some(String::from("DP-2")));
        host.toggle_launcher(Some(String::from("DP-2")));
        now += std::time::Duration::from_millis(30);
        host.advance(now, Some(1.));
        assert_eq!(glass_below_islands(&host, "DP-2").len(), 1, "one launcher");
    }

    /// The glass drawn on `output` below its islands, which is the launcher's.
    fn glass_below_islands(host: &SceneHost, output: &str) -> Vec<[f32; 4]> {
        host.shell
            .as_ref()
            .unwrap()
            .scene
            .borrow()
            .draw(output)
            .into_iter()
            .filter_map(|item| match item {
                Item::Shape {
                    rect,
                    material: Material::Glass,
                    ..
                } if rect[1] > 100. => Some(rect),
                _ => None,
            })
            .collect()
    }

    /// The launcher's glass on DP-1, if it shows.
    fn launcher_glass(host: &SceneHost) -> Option<[f32; 4]> {
        glass_below_islands(host, "DP-1").pop()
    }

    #[test]
    fn the_launcher_never_jumps_typing_as_it_opens_and_turns_around_where_it_is() {
        let mut host = SceneHost::default();
        let outputs = [(String::from("DP-1"), (1920., 1080.))];
        host.ensure_loaded(&outputs, None, serde_json::json!({ "gaps": 8 }));
        // Close to real time, which the script's timers run on.
        let mut now = Instant::now();
        let mut step = |host: &mut SceneHost| {
            now += std::time::Duration::from_millis(16);
            host.advance(now, Some(1.));
        };
        host.toggle_launcher(Some(String::from("DP-1")));
        step(&mut host);
        let mut last = launcher_glass(&host).unwrap();
        let mut speed = [0f32; 4];
        for frame in 0..60 {
            // Typing as it opens: the results come and go.
            if frame == 4 || frame == 9 {
                let key = if frame == 4 { "q" } else { "BackSpace" };
                host.key(&EngineKey {
                    name: String::from(key),
                    text: if frame == 4 {
                        String::from("q")
                    } else {
                        String::new()
                    },
                    modifiers: [false; 4],
                    pressed: true,
                    repeat: false,
                });
            }
            step(&mut host);
            let drawn = launcher_glass(&host).unwrap();
            for i in 0..4 {
                let now = drawn[i] - last[i];
                assert!(
                    (now - speed[i]).abs() < 60.,
                    "it jumped on frame {frame}: {last:?} to {drawn:?}"
                );
                speed[i] = now;
            }
            last = drawn;
        }

        // Closed and opened again on the way, it goes on from where it is.
        host.toggle_launcher(None);
        for _ in 0..12 {
            step(&mut host);
        }
        let closing = launcher_glass(&host).unwrap();
        host.toggle_launcher(Some(String::from("DP-1")));
        step(&mut host);
        let turning = launcher_glass(&host).unwrap();
        for i in 0..4 {
            assert!(
                (turning[i] - closing[i]).abs() < 60.,
                "{closing:?} to {turning:?}"
            );
        }
    }

    #[test]
    fn the_islands_leave_for_a_fullscreen_window_and_the_pointer_at_the_top_brings_them_back() {
        let mut host = SceneHost::default();
        let outputs = [
            (String::from("DP-1"), (1920., 1080.)),
            (String::from("DP-2"), (2560., 1440.)),
        ];
        host.ensure_loaded(&outputs, None, serde_json::json!({ "gaps": 8 }));
        let mut now = Instant::now();
        settle(&mut host, &mut now);
        let drawn = |host: &SceneHost, output: &str| {
            host.shell
                .as_ref()
                .unwrap()
                .scene
                .borrow()
                .draw(output)
                .len()
        };
        assert!(drawn(&host, "DP-1") > 0);

        host.set_fullscreen(serde_json::json!({ "DP-1": true, "DP-2": false }));
        settle(&mut host, &mut now);
        // Nothing is drawn, so nothing is over the window, and it has the pointer everywhere.
        assert_eq!(drawn(&host, "DP-1"), 0, "they're gone");
        assert!(!host.covers("DP-1", [960., 26.]));
        assert!(!host.covers("DP-1", [960., 0.]));
        assert!(drawn(&host, "DP-2") > 0, "only where the window is");

        // The pointer at the top edge brings them back, and on them it keeps them.
        host.pointer_at(Some(("DP-1", [960., 0.])));
        settle(&mut host, &mut now);
        assert!(drawn(&host, "DP-1") > 0, "they're back");
        assert!(host.covers("DP-1", [960., 26.]));
        host.pointer_at(Some(("DP-1", [960., 30.])));
        settle(&mut host, &mut now);
        assert!(drawn(&host, "DP-1") > 0, "they stay under the pointer");

        // A moment after it leaves them, they go again.
        host.pointer_at(Some(("DP-1", [960., 400.])));
        settle(&mut host, &mut now);
        assert_eq!(drawn(&host, "DP-1"), 0, "they're gone again");

        // The calendar, opened, keeps them, and they're back for good without the window.
        host.pointer_at(Some(("DP-1", [960., 0.])));
        settle(&mut host, &mut now);
        host.press(1, 1, Some(("DP-1", [960., 26.])));
        host.release(1, 1, Some(("DP-1", [960., 26.])));
        host.pointer_at(Some(("DP-1", [960., 900.])));
        settle(&mut host, &mut now);
        assert!(drawn(&host, "DP-1") > 0, "the calendar keeps them");
        host.key(&EngineKey {
            name: String::from("Escape"),
            text: String::new(),
            modifiers: [false; 4],
            pressed: true,
            repeat: false,
        });
        settle(&mut host, &mut now);
        assert_eq!(drawn(&host, "DP-1"), 0, "closed, they go");
        host.set_fullscreen(serde_json::json!({ "DP-1": false, "DP-2": false }));
        settle(&mut host, &mut now);
        assert!(drawn(&host, "DP-1") > 0);
        assert!(host.covers("DP-1", [960., 26.]));
    }

    #[test]
    fn with_animations_off_the_launcher_is_there_at_once() {
        let mut host = SceneHost::default();
        let outputs = [(String::from("DP-1"), (1920., 1080.))];
        let style = serde_json::json!({ "gaps": 8, "animations": false, "slowdown": 1 });
        host.ensure_loaded(&outputs, None, style);
        host.toggle_launcher(Some(String::from("DP-1")));
        host.advance(Instant::now(), None);
        let glass = launcher_glass(&host).unwrap();
        assert_eq!(
            glass[2], 720.,
            "stretched to its full width, not a circle: {glass:?}"
        );
    }

    #[test]
    fn the_caret_keeps_up_with_typing() {
        let mut host = SceneHost::default();
        let outputs = [(String::from("DP-1"), (1920., 1080.))];
        host.ensure_loaded(&outputs, None, serde_json::json!({ "gaps": 8 }));
        // The script's timers, like the caret's blink, run on real time, so this one stays close
        // to it and settles the springs without them.
        let mut now = Instant::now();
        host.toggle_launcher(Some(String::from("DP-1")));
        host.advance(now, Some(1.));
        // Its content shows once it stretches.
        now += std::time::Duration::from_millis(200);
        host.advance(now, Some(1.));
        let settle_springs = |host: &SceneHost| {
            let shell = host.shell.as_ref().unwrap();
            for _ in 0..300 {
                shell.scene.borrow_mut().advance(1. / 60., Some(1.));
            }
        };
        settle_springs(&host);
        let caret = |host: &SceneHost| {
            host.shell
                .as_ref()
                .unwrap()
                .scene
                .borrow()
                .draw("DP-1")
                .into_iter()
                .find_map(|item| match item {
                    // Below the islands, where the launcher is.
                    Item::Shape { rect, .. } if rect[2] < 3. && rect[1] > 100. => Some(rect[0]),
                    _ => None,
                })
        };
        let mut press = |host: &mut SceneHost, name: &str, text: &str| {
            host.key(&EngineKey {
                name: name.to_string(),
                text: text.to_string(),
                modifiers: [false; 4],
                pressed: true,
                repeat: false,
            });
            now += std::time::Duration::from_millis(16);
            host.advance(now, Some(1.));
        };
        press(&mut host, "w", "w");
        let start = caret(&host).expect("a caret, lit while typing");
        for _ in 0..4 {
            press(&mut host, "w", "w");
        }
        let typed = caret(&host).unwrap();
        // A tenth of a second after the last letter, with the caret still lit.
        for _ in 0..6 {
            host.shell
                .as_ref()
                .unwrap()
                .scene
                .borrow_mut()
                .advance(1. / 60., Some(1.));
        }
        let caught_up = caret(&host).unwrap();
        settle_springs(&host);
        let settled = caret(&host).unwrap();
        assert!(settled > start + 20.);
        // Even with a letter every frame, it's less than a letter behind, and catches up.
        assert!(
            (typed - settled).abs() < 8.,
            "left behind at {typed}, not {settled}"
        );
        assert!(
            (caught_up - settled).abs() < 0.5,
            "still behind at {caught_up}"
        );
    }

    #[test]
    fn what_the_data_program_says_reaches_the_shell_and_a_reloaded_one() {
        let mut host = SceneHost::default();
        let outputs = [(String::from("DP-1"), (1920., 1080.))];
        host.ensure_loaded(&outputs, None, serde_json::json!({ "gaps": 8 }));
        host.data_line(r#"{"type":"network","connected":true,"icon":"network-wired-symbolic"}"#);
        let mut now = Instant::now();
        settle(&mut host, &mut now);
        let shows_wired =
            |host: &SceneHost| {
                host.shell.as_ref().unwrap().scene.borrow().draw("DP-1").iter().any(|item| {
                matches!(item, Item::Image { source: super::super::ImageSource::Icon(name), .. }
                    if name == "network-wired-symbolic")
            })
            };
        assert!(shows_wired(&host));

        // Loaded again, the shell hears the last of it straight away.
        host.loaded_mtimes = None;
        host.ensure_loaded(&outputs, None, serde_json::json!({ "gaps": 8 }));
        settle(&mut host, &mut now);
        assert!(shows_wired(&host));
    }

    #[test]
    fn a_click_opens_the_calendar_and_escape_or_a_press_elsewhere_closes_it() {
        let mut host = SceneHost::default();
        let outputs = [(String::from("DP-1"), (1920., 1080.))];
        host.ensure_loaded(&outputs, None, serde_json::json!({ "gaps": 8 }));
        let mut now = Instant::now();
        settle(&mut host, &mut now);

        // The clock's island is a pill: its rounded corner isn't the shell's.
        assert!(host.covers("DP-1", [960., 26.]));
        let island = host
            .shell
            .as_ref()
            .unwrap()
            .scene
            .borrow()
            .draw("DP-1")
            .into_iter()
            .filter_map(|item| match item {
                Item::Shape {
                    rect,
                    material: Material::Glass,
                    ..
                } => Some(rect),
                _ => None,
            })
            .nth(1)
            .unwrap();
        assert!(!host.covers("DP-1", [island[0] + 1., island[1] + 1.]));

        let click = |host: &mut SceneHost| {
            let at = Some(("DP-1", [960., 26.]));
            assert!(host.press(272, 1, at));
            assert!(host.release(272, 1, at));
        };
        click(&mut host);
        settle(&mut host, &mut now);
        assert!(
            host.wants_keyboard(),
            "the open calendar takes the keyboard"
        );

        host.key(&EngineKey {
            name: String::from("Escape"),
            text: String::new(),
            modifiers: [false; 4],
            pressed: true,
            repeat: false,
        });
        assert!(!host.wants_keyboard(), "Escape closes it");

        click(&mut host);
        settle(&mut host, &mut now);
        assert!(host.wants_keyboard());
        assert!(
            !host.press(272, 1, None),
            "a press elsewhere isn't the shell's"
        );
        assert!(!host.wants_keyboard(), "and closes it");
    }
}
