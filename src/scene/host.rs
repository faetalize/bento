//! bento's own shell in bento: loads the shell's script, moves its scene on, draws it per output
//! with the Vulkan renderer, and hands the result and its glass to bento's compositing.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Instant, SystemTime};

use smithay::backend::allocator::Format as DrmFormat;
use smithay::backend::drm::DrmNode;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{GlesError, GlesFrame, GlesRenderer, GlesTexture};
use smithay::backend::renderer::utils::{
    CommitCounter, DamageBag, DamageSet, DamageSnapshot, OpaqueRegions,
};
use smithay::backend::renderer::ImportDma;
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Physical, Rectangle, Scale, Transform};

use super::script::{self, Shell};
use super::vk::{Renderer, Target};
use super::{Item, Material, NodeId, Theme, CLICK, HOVER, SCROLL};
use crate::backend::tty::{TtyFrame, TtyRenderer, TtyRendererError};
use crate::render_helpers::background_effect::BackgroundEffectElement;
use crate::render_helpers::framebuffer_effect::FramebufferEffect;
use crate::render_helpers::glass::{self, GlassFrame};
use crate::render_helpers::primary_gpu_texture::PrimaryGpuTextureRenderElement;
use crate::render_helpers::renderer::AsGlesFrame as _;
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
    /// What it was drawn from, to tell what the next frame changes.
    items: Vec<Item>,
    /// What bento's compositing knows the scene's picture by, from frame to frame, and where it
    /// changed in each: only there is the screen drawn again.
    id: Id,
    damage: DamageBag<i32, Physical>,
    /// The glass under what was last drawn.
    glass: Vec<([f32; 4], [f32; 4])>,
    effect: FramebufferEffect,
    /// The generation of the scene last looked at, at what scale, and with which of the icons
    /// and images that were loading.
    drawn: Option<(u64, f64, u64)>,
}

/// The most rectangles a frame's changes are told as; more are told as the one around them all.
const MAX_DAMAGE: usize = 16;

/// The scene's picture for an output, as bento's compositing draws it: the whole output's size,
/// and changed only where the scene's items did.
#[derive(Debug, Clone)]
pub struct SceneElement {
    texture: PrimaryGpuTextureRenderElement,
    id: Id,
    damage: DamageSnapshot<i32, Physical>,
}

impl Element for SceneElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.damage.current_commit()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.texture.geometry(scale)
    }

    fn transform(&self) -> Transform {
        self.texture.transform()
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.texture.src()
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        // The picture is the output's size in its pixels, at its top left, so where it changed
        // is where the output does.
        self.damage.damage_since(commit).unwrap_or_else(|| {
            DamageSet::from_slice(&[Rectangle::from_size(self.geometry(scale).size)])
        })
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        self.texture.opaque_regions(scale)
    }

    fn alpha(&self) -> f32 {
        self.texture.alpha()
    }

    fn kind(&self) -> Kind {
        self.texture.kind()
    }
}

impl RenderElement<GlesRenderer> for SceneElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        RenderElement::<GlesRenderer>::draw(
            &self.texture,
            frame,
            src,
            dst,
            damage,
            opaque_regions,
            cache,
        )
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

impl<'render> RenderElement<TtyRenderer<'render>> for SceneElement {
    fn draw(
        &self,
        frame: &mut TtyFrame<'_, '_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), TtyRendererError<'render>> {
        let gles_frame = frame.as_gles_frame();
        RenderElement::<GlesRenderer>::draw(
            &self.texture,
            gles_frame,
            src,
            dst,
            damage,
            opaque_regions,
            cache,
        )?;
        Ok(())
    }

    fn underlying_storage(
        &self,
        _renderer: &mut TtyRenderer<'render>,
    ) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

/// Where the scene's picture changes from `old` to `new`, in the output's pixels at `scale`:
/// around each item that isn't the one it was, where it was and where it is. With `loaded`,
/// around the icons, images and text too, some of which have come in since.
fn changed(old: &[Item], new: &[Item], scale: f64, loaded: bool) -> Vec<Rectangle<i32, Physical>> {
    let mut rects: Vec<Rectangle<i32, Physical>> = Vec::new();
    let mut add = |item: &Item| {
        let [x, y, w, h] = item.bounds().map(f64::from);
        if w <= 0. || h <= 0. {
            return;
        }
        let (x0, y0) = ((x * scale).floor() as i32, (y * scale).floor() as i32);
        let (x1, y1) = (
            ((x + w) * scale).ceil() as i32,
            ((y + h) * scale).ceil() as i32,
        );
        rects.push(Rectangle::new((x0, y0).into(), (x1 - x0, y1 - y0).into()));
    };
    let both = old.len().min(new.len());
    for (was, is) in old.iter().zip(new) {
        if !was.same(is) {
            add(was);
            add(is);
        } else if loaded && !matches!(is, Item::Shape { .. }) {
            add(is);
        }
    }
    for item in old[both..].iter().chain(&new[both..]) {
        add(item);
    }
    if rects.len() > MAX_DAMAGE {
        let around = rects
            .iter()
            .copied()
            .reduce(|all, rect| all.merge(rect))
            .unwrap();
        rects = vec![around];
    }
    rects
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

    /// Hands the data program the shell's commands. One with `"token": true` gets an activation
    /// token from `token` in its place, which lets the app it reaches, like the one a
    /// notification's action goes to, bring its window forward.
    pub fn send_commands(&mut self, mut token: impl FnMut() -> String) {
        let Some(shell) = &self.shell else { return };
        let commands = shell.take_commands();
        let Some((_, stdin)) = &mut self.data else {
            return;
        };
        for mut command in commands {
            if command["token"] == serde_json::Value::Bool(true) {
                command["token"] = serde_json::Value::String(token());
            }
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
    ) -> bool {
        let Some(shell) = &self.shell else {
            return false;
        };
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
        let Some(vk) = &mut self.renderer else {
            return false;
        };
        vk.set_icon_theme(icon_theme);
        // Icons and images loaded since show from this frame on, on every output.
        let came = vk.poll();
        // What the script says it will show is loaded before it does.
        let preloads = std::mem::take(&mut shell.scene.borrow_mut().preloads);
        for (source, size) in preloads {
            vk.preload(source, (f64::from(size) * scale).round().max(1.) as u32);
        }

        let scene = shell.scene.borrow();
        let items = scene.draw(output);
        let state = self
            .outputs
            .entry(String::from(output))
            .or_insert_with(|| OutputScene {
                targets: Vec::new(),
                next: 0,
                shown: None,
                items: Vec::new(),
                id: Id::new(),
                damage: DamageBag::default(),
                glass: Vec::new(),
                effect: FramebufferEffect::new(),
                drawn: None,
            });
        let now = (self.generation, scale, vk.epoch());
        if state.drawn == Some(now) {
            return came;
        }
        let rescaled = state.drawn.is_none_or(|(_, before, _)| before != scale);
        let images = state.drawn.is_some_and(|(_, _, epoch)| epoch != vk.epoch());
        state.drawn = Some(now);
        // The scene changed, but not in what this output shows: its picture stays.
        let same = items.len() == state.items.len()
            && items.iter().zip(&state.items).all(|(a, b)| a.same(b));
        if same && !rescaled && !images && (state.shown.is_some() || items.is_empty()) {
            return came;
        }

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
            state.items = items;
            for (target, _) in state.targets.drain(..) {
                vk.destroy_target(target);
            }
            return came;
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
                return came;
            }
            state.next = 0;
        }

        let (target, texture) = &state.targets[state.next];
        if let Err(err) = vk.render(target, &items, scale as f32) {
            warn!("error drawing the scene: {err:?}");
            return came;
        }
        // All of it is new after nothing showed or at another scale; else, where it changed.
        if state.shown.is_none() || rescaled {
            state.damage.reset();
        } else {
            state
                .damage
                .add(changed(&state.items, &items, scale, images));
        }
        state.items = items;
        state.shown = Some(TextureBuffer::from_texture(
            renderer,
            texture.clone(),
            scale,
            Transform::Normal,
            Vec::new(),
        ));
        state.next = (state.next + 1) % state.targets.len();
        came
    }

    /// Whether icons or images the scene shows are still being loaded: it's drawn again when
    /// they're there.
    pub fn loading(&self) -> bool {
        self.renderer.as_ref().is_some_and(Renderer::loading)
    }

    /// Pushes what output `output`'s scene shows, over its glass.
    pub fn render_content(&self, output: &str, push: &mut dyn FnMut(SceneElement)) {
        let Some(state) = self.outputs.get(output) else {
            return;
        };
        let Some(buffer) = &state.shown else {
            return;
        };
        push(SceneElement {
            texture: PrimaryGpuTextureRenderElement(TextureRenderElement::from_texture_buffer(
                buffer.clone(),
                (0., 0.),
                1.,
                None,
                None,
                Kind::Unspecified,
            )),
            id: state.id.clone(),
            damage: state.damage.snapshot(),
        });
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

    use niri_config::Action;

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

    /// The middle of the first text saying `wanted`, or icon called it, that shows on `output`.
    fn middle_of(host: &SceneHost, output: &str, wanted: &str) -> [f32; 2] {
        let found = host
            .shell
            .as_ref()
            .unwrap()
            .scene
            .borrow()
            .find(output, wanted);
        let frame = found
            .first()
            .unwrap_or_else(|| panic!("{wanted:?} doesn't show on {output}"));
        [frame[0] + frame[2] / 2., frame[1] + frame[3] / 2.]
    }

    fn shows(host: &SceneHost, output: &str, wanted: &str) -> bool {
        let scene = host.shell.as_ref().unwrap().scene.borrow();
        !scene.find(output, wanted).is_empty()
    }

    fn click(host: &mut SceneHost, output: &str, point: [f32; 2]) {
        assert!(host.covers(output, point), "nothing to click at {point:?}");
        host.press(1, 1, Some((output, point)));
        host.release(1, 1, Some((output, point)));
    }

    /// The bento actions the shell asked for since this was last asked, as bento runs them: one
    /// that isn't an action fails the test.
    fn runs(host: &SceneHost) -> Vec<niri_config::Action> {
        host.take_actions()
            .into_iter()
            .map(|request| {
                let request = crate::ipc::action::ActionRequest {
                    name: request["name"].as_str().unwrap_or_default().to_owned(),
                    args: request["args"].as_array().cloned().unwrap_or_default(),
                    properties: request["properties"]
                        .as_object()
                        .cloned()
                        .unwrap_or_default(),
                    window: request["window"].as_u64(),
                    workspace: request["workspace"].as_u64(),
                };
                crate::ipc::action::parse(request, |_| None).unwrap()
            })
            .collect()
    }

    /// What the shell told the data program to do since this was last asked.
    fn commands(host: &SceneHost) -> Vec<serde_json::Value> {
        host.shell.as_ref().unwrap().take_commands()
    }

    fn type_keys(host: &mut SceneHost, keys: &[&str]) {
        for key in keys {
            let text = if key.chars().count() == 1 { key } else { "" };
            host.key(&EngineKey {
                name: String::from(*key),
                text: String::from(text),
                modifiers: [false; 4],
                pressed: true,
                repeat: false,
            });
        }
    }

    fn loaded(style: serde_json::Value) -> (SceneHost, Instant) {
        let mut host = SceneHost::default();
        let outputs = [(String::from("DP-1"), (1920., 1080.))];
        host.ensure_loaded(&outputs, None, style);
        host.set_workspaces(serde_json::json!({
            "DP-1": [
                { "index": 0, "name": null, "active": true, "focused": true, "id": 7 },
                { "index": 1, "name": "web", "active": false, "focused": false, "id": 8 },
            ],
        }));
        let mut now = Instant::now();
        settle(&mut host, &mut now);
        (host, now)
    }

    #[test]
    fn the_minimap_shows_the_tiles_and_takes_scrolls_and_clicks() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        let shapes = |host: &SceneHost| {
            let scene = host.shell.as_ref().unwrap().scene.borrow();
            let drawn = scene.draw("DP-1");
            // The minimap's pieces: small shapes in the left island.
            drawn
                .into_iter()
                .filter_map(|item| match item {
                    Item::Shape { rect, .. } if rect[3] <= 14. && rect[0] < 600. => Some(rect),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert!(
            shapes(&host).is_empty(),
            "without tiles there's no minimap: {:?}",
            shapes(&host)
        );

        let tile = |id: u64, x: f64, active: bool| {
            serde_json::json!({
                "id": id, "x": x, "y": 0, "w": 960, "h": 1080, "active": active,
                "floating": false,
            })
        };
        host.set_minimap(serde_json::json!({
            "DP-1": {
                "view": [1920, 1080],
                "tiles": [tile(1, 0., true), tile(2, 960., false), tile(3, 1920., false)],
            },
        }));
        settle(&mut host, &mut now);
        // The track isn't drawn itself: the view and the three tiles are.
        let pieces = shapes(&host);
        assert_eq!(pieces.len(), 4, "{pieces:?}");
        // The view shows two of the three tiles' width.
        let view = pieces[0];
        let last = pieces[3];
        assert!(
            (view[2] / (last[0] + last[2] - view[0]) - 2. / 3.).abs() < 0.05,
            "{pieces:?}"
        );

        // Scrolling over it focuses the next tile that way, and a click on a tile its window.
        let over = [last[0] + last[2] / 2., last[1] + last[3] / 2.];
        host.pointer_moved(Some(("DP-1", over)));
        host.scroll([0., 15.]);
        let actions = runs(&host);
        assert_eq!(actions, vec![Action::FocusWindow(2)]);
        host.scroll([0., -15.]);
        assert!(runs(&host).is_empty(), "nothing's left of the first");
        click(&mut host, "DP-1", over);
        assert_eq!(runs(&host), vec![Action::FocusWindow(3)]);
    }

    #[test]
    fn the_workspace_editor_renames_as_its_typed_and_takes_the_name_away() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        host.event("edit-workspace", Some(String::from("DP-1")));
        settle(&mut host, &mut now);
        assert!(host.wants_keyboard());
        assert!(shows(&host, "DP-1", "Workspace 1"));

        type_keys(&mut host, &["d", "e", "v"]);
        let actions = host.take_actions();
        let last = actions.last().unwrap();
        assert_eq!(actions.len(), 3, "renamed with each letter");
        assert_eq!(last["name"], "set-workspace-name");
        assert_eq!(last["args"], serde_json::json!(["dev"]));
        assert_eq!(last["workspace"], 7);

        // Another workspace's name isn't taken from it, and the editor says so.
        type_keys(&mut host, &["BackSpace", "BackSpace", "BackSpace"]);
        assert_eq!(
            host.take_actions().last().unwrap()["name"],
            "unset-workspace-name"
        );
        type_keys(&mut host, &["W", "e", "b"]);
        let actions = host.take_actions();
        assert_eq!(actions.last().unwrap()["args"], serde_json::json!(["We"]));
        settle(&mut host, &mut now);
        assert!(shows(
            &host,
            "DP-1",
            "\"web\" is another workspace already."
        ));

        // Undo puts back the name it had, which was none, and Enter closes it.
        settle(&mut host, &mut now);
        let undo = middle_of(&host, "DP-1", "Undo");
        click(&mut host, "DP-1", undo);
        assert_eq!(
            host.take_actions().last().unwrap()["name"],
            "unset-workspace-name"
        );
        type_keys(&mut host, &["Return"]);
        settle(&mut host, &mut now);
        assert!(!host.wants_keyboard());
        assert!(!shows(&host, "DP-1", "Workspace 1"));
    }

    #[test]
    fn the_workspace_editor_keeps_a_name_until_its_changed_and_undoes_taking_it_away() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        host.set_workspaces(serde_json::json!({
            "DP-1": [{ "index": 0, "name": "mail", "active": true, "focused": true, "id": 9 }],
        }));
        host.event("edit-workspace", Some(String::from("DP-1")));
        settle(&mut host, &mut now);
        // Opening it changes nothing.
        assert!(host.take_actions().is_empty());

        let remove = middle_of(&host, "DP-1", "Remove Name");
        click(&mut host, "DP-1", remove);
        let actions = host.take_actions();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["name"], "unset-workspace-name");
        assert_eq!(actions[0]["workspace"], 9);

        settle(&mut host, &mut now);
        let undo = middle_of(&host, "DP-1", "Undo");
        click(&mut host, "DP-1", undo);
        let actions = host.take_actions();
        assert_eq!(actions[0]["name"], "set-workspace-name");
        assert_eq!(actions[0]["args"], serde_json::json!(["mail"]));

        // The shortcut again closes it.
        host.event("edit-workspace", Some(String::from("DP-1")));
        settle(&mut host, &mut now);
        assert!(!host.wants_keyboard());
    }

    const DEVICES: &str = r#"{"type":"audio-devices","outputs":[
        {"name":"a","label":"Speakers","icon":"audio-speakers-symbolic","active":true,"volume":0.4,"muted":false},
        {"name":"b","label":"Headphones","icon":"audio-headphones-symbolic","active":false,"volume":0.8,"muted":false}],
        "inputs":[{"name":"m","label":"Microphone","icon":"audio-input-microphone-symbolic","active":true,"volume":1.0,"muted":false}]}"#;

    /// Opens the control center by a click on the right island's status.
    fn open_control_center(host: &mut SceneHost, now: &mut Instant) {
        let status = middle_of(host, "DP-1", "network-offline-symbolic");
        click(host, "DP-1", status);
        settle(host, now);
        assert!(host.wants_keyboard(), "the control center is open");
    }

    #[test]
    fn the_control_center_sets_the_sound_the_brightness_and_the_style() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        host.data_line(&DEVICES.replace('\n', " "));
        host.data_line(
            r#"{"type":"brightness","displays":[{"id":"DP-1","label":"Monitor","value":0.3}]}"#,
        );
        host.data_line(r#"{"type":"theme","dark":true}"#);
        commands(&host);
        open_control_center(&mut host, &mut now);
        // Opening it reads the brightness again, since a monitor's buttons change it too.
        assert!(commands(&host)
            .iter()
            .any(|c| c["cmd"] == "brightness-refresh"));

        // The list shows the default device; a click on it opens the list, and a click on
        // another makes it the default one.
        assert!(!shows(&host, "DP-1", "Headphones"));
        let speakers = middle_of(&host, "DP-1", "Speakers");
        click(&mut host, "DP-1", speakers);
        settle(&mut host, &mut now);
        let headphones = middle_of(&host, "DP-1", "Headphones");
        click(&mut host, "DP-1", headphones);
        let sent = commands(&host);
        assert_eq!(
            sent[0],
            serde_json::json!({ "cmd": "device", "kind": "output", "name": "b" })
        );

        // A press halfway along the brightness slider sets it to a half.
        let sun = middle_of(&host, "DP-1", "display-brightness-symbolic");
        let halfway = [sun[0] - 14. + 170., sun[1]];
        host.press(1, 1, Some(("DP-1", halfway)));
        // Dragged, it follows the pointer.
        host.pointer_at(Some(("DP-1", [halfway[0] + 85., halfway[1]])));
        host.release(1, 1, Some(("DP-1", halfway)));
        let sent = commands(&host);
        assert_eq!(sent[0]["cmd"], "brightness");
        assert_eq!(sent[0]["id"], "DP-1");
        assert!(
            (sent[0]["value"].as_f64().unwrap() - 0.5).abs() < 0.01,
            "{sent:?}"
        );
        assert!(
            (sent[1]["value"].as_f64().unwrap() - 0.75).abs() < 0.01,
            "{sent:?}"
        );

        // The style's button turns the dark style off while it's on.
        let dark = middle_of(&host, "DP-1", "Dark");
        click(&mut host, "DP-1", dark);
        assert_eq!(
            commands(&host)[0],
            serde_json::json!({ "cmd": "theme", "dark": false })
        );

        // Escape closes it.
        type_keys(&mut host, &["Escape"]);
        settle(&mut host, &mut now);
        assert!(!host.wants_keyboard());
    }

    #[test]
    fn the_clocks_time_grows_into_the_calendars_and_the_date_travels_to_its_own() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        let at = clock_texts(&host);
        let seen = |host: &SceneHost, text: &str| {
            host.shell
                .as_ref()
                .unwrap()
                .scene
                .borrow()
                .seen("DP-1", text)
        };
        let short_date = seen(&host, &at.1)[0].0;
        click(&mut host, "DP-1", [960., 26.]);

        // On the way, it's the island's time that moves, grows and shows, all the way.
        let mut lefts = Vec::new();
        for _ in 0..8 {
            now += std::time::Duration::from_millis(16);
            host.advance(now, Some(1.));
            let times = seen(&host, &at.0);
            let small = times.iter().find(|(frame, ..)| frame[3] < 30.).copied();
            let (frame, opacity, _) = small.unwrap_or(times[0]);
            assert!(opacity > 0.99, "it shows on the way: {times:?}");
            for (other, opacity, _) in &times {
                if *other != frame {
                    assert!(*opacity < 0.01, "only one time shows on the way: {times:?}");
                }
            }
            lefts.push(frame[0]);
        }
        settle(&mut host, &mut now);

        // There, the island's time stays, as big as the calendar's room for it, and it's the
        // only time that shows.
        let times = seen(&host, &at.0);
        let shown: Vec<_> = times
            .iter()
            .filter(|(_, opacity, _)| *opacity > 0.99)
            .collect();
        assert_eq!(shown.len(), 1, "{times:?}");
        let (room, _, _) = *times
            .iter()
            .find(|(_, opacity, _)| *opacity < 0.01)
            .unwrap();
        let grown = shown[0].0;
        // It's the island's time grown, as wide as its figures, not the calendar's room, which
        // is as wide as its column.
        assert!(grown[2] < room[2] - 50., "{grown:?} is the room {room:?}");
        // Its start, top and height: the calendar lays its room out as wide as its column.
        for i in [0, 1, 3] {
            assert!((grown[i] - room[i]).abs() < 1.5, "{grown:?} in {room:?}");
        }
        assert!(
            lefts[0] > room[0] && lefts[7] < lefts[0],
            "{lefts:?} to {room:?}"
        );

        // The date travelled to the start of the calendar's date, not to its middle.
        let long = seen(&host, &at.2)[0].0;
        let travelled = seen(&host, &at.1)[0].0;
        assert!(
            (travelled[0] - long[0]).abs() < 1.5,
            "{travelled:?} to {long:?}"
        );
        assert!(travelled[0] < short_date[0]);

        // Closed, they go home.
        type_keys(&mut host, &["Escape"]);
        settle(&mut host, &mut now);
        let back = seen(&host, &at.1)[0];
        assert!(
            (back.0[0] - short_date[0]).abs() < 1.5 && back.1 > 0.99,
            "{back:?}"
        );
    }

    /// The island's time, its date and the calendar's date, as the clock shows them now.
    fn clock_texts(host: &SceneHost) -> (String, String, String) {
        let scene = host.shell.as_ref().unwrap().scene.borrow();
        let mut texts = Vec::new();
        fn walk(scene: &super::super::Scene, id: u32, texts: &mut Vec<String>) {
            if let Some(node) = scene.node(id) {
                if let super::super::Kind::Text { text, .. } = &node.kind {
                    texts.push(text.clone());
                }
                for child in scene.held(id) {
                    walk(scene, child, texts);
                }
            }
        }
        walk(&scene, scene.root("DP-1").unwrap(), &mut texts);
        let time = texts
            .iter()
            .find(|t| t.len() == 5 && t.as_bytes()[2] == b':')
            .unwrap()
            .clone();
        let short = texts
            .iter()
            .find(|t| t.split(' ').count() == 3 && t.len() <= 10)
            .unwrap()
            .clone();
        let long = texts.iter().find(|t| t.contains(", ")).unwrap().clone();
        (time, short, long)
    }

    #[test]
    fn the_control_center_follows_the_volume_changed_elsewhere() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        host.data_line(&DEVICES.replace('\n', " "));
        open_control_center(&mut host, &mut now);
        // How much of the sound slider shows filled: its level is the track's width slid left,
        // so it's where the level ends, from where the track starts.
        let fill = |host: &SceneHost| {
            let icon = middle_of(host, "DP-1", "audio-volume-medium-symbolic");
            let scene = host.shell.as_ref().unwrap().scene.borrow();
            let bars: Vec<[f32; 4]> = scene
                .draw("DP-1")
                .into_iter()
                .filter_map(|item| match item {
                    Item::Shape { rect, .. }
                        if rect[3] == 28. && (rect[1] + 14. - icon[1]).abs() < 2. =>
                    {
                        Some(rect)
                    }
                    _ => None,
                })
                .collect();
            let track = bars.iter().map(|bar| bar[0]).fold(f32::MIN, f32::max);
            let level = bars.iter().find(|bar| bar[0] < track).unwrap();
            level[0] + level[2] - track
        };
        let before = fill(&host);
        // A knob or a media key turns it up.
        host.data_line(&DEVICES.replace('\n', " ").replace(
            r#""active":true,"volume":0.4"#,
            r#""active":true,"volume":0.6"#,
        ));
        settle(&mut host, &mut now);
        let after = fill(&host);
        assert!((after / before - 1.5).abs() < 0.05, "{before} to {after}");
    }

    /// Steps `host` on a frame, at 16 ms.
    fn frame(host: &mut SceneHost, now: &mut Instant) {
        *now += std::time::Duration::from_millis(16);
        host.advance(*now, Some(1.));
    }

    #[test]
    fn what_the_island_holds_stays_put_as_it_opens_and_closes() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        host.data_line(&DEVICES.replace('\n', " "));
        settle(&mut host, &mut now);
        let at = |host: &SceneHost| {
            let scene = host.shell.as_ref().unwrap().scene.borrow();
            scene.seen("DP-1", "network-offline-symbolic")[0].0
        };
        let before = at(&host);
        let status = middle_of(&host, "DP-1", "network-offline-symbolic");
        click(&mut host, "DP-1", status);
        for _ in 0..20 {
            frame(&mut host, &mut now);
            assert_eq!(at(&host), before, "opening");
        }
        type_keys(&mut host, &["Escape"]);
        for _ in 0..20 {
            frame(&mut host, &mut now);
            assert_eq!(at(&host), before, "closing");
        }
    }

    #[test]
    fn a_panel_closes_in_one_motion() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        let glass = |host: &SceneHost| {
            let scene = host.shell.as_ref().unwrap().scene.borrow();
            scene
                .draw("DP-1")
                .into_iter()
                .filter_map(|item| match item {
                    Item::Shape {
                        rect,
                        material: Material::Glass,
                        ..
                    } => Some(rect[3]),
                    _ => None,
                })
                .fold(0., f32::max)
        };
        click(&mut host, "DP-1", [960., 26.]);
        settle(&mut host, &mut now);
        let open = glass(&host);
        type_keys(&mut host, &["Escape"]);
        frame(&mut host, &mut now);
        frame(&mut host, &mut now);
        assert!(
            glass(&host) < open - 5.,
            "the glass starts shrinking at once"
        );
    }

    #[test]
    fn a_list_opens_like_a_drawer_and_a_slider_keeps_its_level_as_it_moves() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        // The microphone at half, so that its level shows.
        host.data_line(&DEVICES.replace('\n', " ").replace(
            r#""active":true,"volume":1.0"#,
            r#""active":true,"volume":0.5"#,
        ));
        open_control_center(&mut host, &mut now);
        // The microphone's slider, below the outputs, and how much of it shows filled.
        let mic = |host: &SceneHost| {
            let scene = host.shell.as_ref().unwrap().scene.borrow();
            let icon = scene.seen("DP-1", "microphone-sensitivity-high-symbolic")[0].0;
            let bars: Vec<[f32; 4]> = scene
                .draw("DP-1")
                .into_iter()
                .filter_map(|item| match item {
                    Item::Shape { rect, .. }
                        if rect[3] == 28.
                            && (rect[1] + 14. - (icon[1] + icon[3] / 2.)).abs() < 2. =>
                    {
                        Some(rect)
                    }
                    _ => None,
                })
                .collect();
            let track = bars.iter().map(|bar| bar[0]).fold(f32::MIN, f32::max);
            let level = bars.iter().find(|bar| bar[0] < track).unwrap();
            (bars[0][1], level[0] + level[2] - track)
        };
        let (_, filled) = mic(&host);
        let speakers = middle_of(&host, "DP-1", "Speakers");
        click(&mut host, "DP-1", speakers);
        let mut tops = Vec::new();
        let mut listed = false;
        for _ in 0..20 {
            frame(&mut host, &mut now);
            let (now_top, now_filled) = mic(&host);
            assert!(
                (now_filled - filled).abs() < 1.,
                "the level moves with its slider"
            );
            tops.push(now_top);
            // What the list holds shows only inside it, cut to the list, which is as wide as
            // the panel's column, and that ends above the slider it pushes down.
            let scene = host.shell.as_ref().unwrap().scene.borrow();
            for item in scene.draw("DP-1") {
                if let Item::Text {
                    clip: Some(clip), ..
                } = item
                {
                    // The outputs' list, which starts above the slider; the inputs' is below it.
                    if clip.rect[2] == 340. && clip.rect[1] < now_top {
                        listed = true;
                        let bottom = clip.rect[1] + clip.rect[3];
                        assert!(bottom <= now_top + 0.5, "{clip:?} over {now_top}");
                    }
                }
            }
        }
        assert!(listed, "the list cuts what it holds to itself");
        // The slider below was pushed down little by little.
        assert!(
            tops[0] < tops[19] && tops[1] - tops[0] < tops[19] - tops[0],
            "{tops:?}"
        );
        assert!(shows(&host, "DP-1", "Headphones"));
    }

    #[test]
    fn pinned_outputs_get_a_switcher_that_goes_around_them() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        host.data_line(&DEVICES.replace('\n', " "));
        settle(&mut host, &mut now);
        // Without pinned outputs there's no switcher, and its shortcut says what to do.
        assert!(!shows(&host, "DP-1", "audio-speakers-symbolic"));
        commands(&host);
        host.event("switch-audio-output", None);
        let sent = commands(&host);
        assert_eq!(sent[0]["cmd"], "notify");
        assert!(sent[0]["body"]
            .as_str()
            .unwrap()
            .contains("No outputs are pinned"));

        // Both outputs pinned, in the control center, with its list open.
        open_control_center(&mut host, &mut now);
        let speakers = middle_of(&host, "DP-1", "Speakers");
        click(&mut host, "DP-1", speakers);
        settle(&mut host, &mut now);
        for row in 0..2 {
            let scene = host.shell.as_ref().unwrap().scene.borrow();
            let pin = scene.find("DP-1", "view-pin-symbolic")[row];
            drop(scene);
            let at = [pin[0] + pin[2] / 2., pin[1] + pin[3] / 2.];
            click(&mut host, "DP-1", at);
            settle(&mut host, &mut now);
        }
        type_keys(&mut host, &["Escape"]);
        settle(&mut host, &mut now);
        commands(&host);

        // The switcher shows in the island, as the output that plays, and a click on it makes
        // the next pinned output the default one and says so.
        let switcher = middle_of(&host, "DP-1", "audio-speakers-symbolic");
        click(&mut host, "DP-1", switcher);
        let sent = commands(&host);
        assert_eq!(
            sent[0],
            serde_json::json!({ "cmd": "device", "kind": "output", "name": "b" })
        );
        assert_eq!(sent[1]["cmd"], "notify");
        assert_eq!(sent[1]["summary"], "Sound Output Switched");
        assert_eq!(sent[1]["body"], "Headphones");

        // From the last one it goes around to the first, by its shortcut too.
        host.data_line(
            &DEVICES
                .replace('\n', " ")
                .replace(
                    r#""active":true,"volume":0.4"#,
                    r#""active":false,"volume":0.4"#,
                )
                .replace(
                    r#""active":false,"volume":0.8"#,
                    r#""active":true,"volume":0.8"#,
                ),
        );
        host.event("switch-audio-output", None);
        assert_eq!(commands(&host)[0]["name"], "a");
    }

    #[test]
    fn the_screenshot_button_runs_its_command_and_shows_its_notification_after() {
        let style = serde_json::json!({
            "gaps": 8,
            "config": {
                "screenshot": { "command": "grim", "notification": "Shot", "body": "Saved." },
            },
        });
        let (mut host, mut now) = loaded(style);
        open_control_center(&mut host, &mut now);
        commands(&host);
        let button = middle_of(&host, "DP-1", "Screenshot");
        click(&mut host, "DP-1", button);
        // The control center closes first, to stay out of the picture.
        assert!(!host.wants_keyboard());
        settle(&mut host, &mut now);
        let sent = commands(&host);
        assert_eq!(sent[0]["cmd"], "run");
        assert_eq!(sent[0]["command"], "grim");
        assert!(host.take_actions().is_empty(), "not bento's own screenshot");

        // Its notification shows once the command is done, and not if it failed.
        let id = sent[0]["id"].clone();
        host.data_line(&serde_json::json!({ "type": "ran", "id": id, "code": 0 }).to_string());
        let sent = commands(&host);
        assert_eq!(sent[0]["cmd"], "notify");
        assert_eq!(sent[0]["summary"], "Shot");
        assert_eq!(sent[0]["body"], "Saved.");

        // The record button, with no command set, opens the settings to set one.
        open_control_center(&mut host, &mut now);
        let record = middle_of(&host, "DP-1", "Record");
        click(&mut host, "DP-1", record);
        let actions = host.take_actions();
        assert_eq!(actions[0]["name"], "spawn");
        assert_eq!(
            actions[0]["args"],
            serde_json::json!(["bento-settings", "control-center"])
        );
    }

    #[test]
    fn without_a_command_the_screenshot_button_takes_bentos_own() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        open_control_center(&mut host, &mut now);
        let button = middle_of(&host, "DP-1", "Screenshot");
        click(&mut host, "DP-1", button);
        settle(&mut host, &mut now);
        assert_eq!(host.take_actions()[0]["name"], "screenshot");
        assert!(commands(&host).iter().all(|c| c["cmd"] != "run"));
    }

    #[test]
    fn the_calendar_shows_the_weather_or_where_to_set_the_place() {
        // Without a place, it asks for none and links to the settings.
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        let sent = commands(&host);
        assert!(
            sent.contains(&serde_json::json!({ "cmd": "weather" })),
            "{sent:?}"
        );
        click(&mut host, "DP-1", [960., 26.]);
        settle(&mut host, &mut now);
        let link = middle_of(&host, "DP-1", "Set Location");
        click(&mut host, "DP-1", link);
        let actions = host.take_actions();
        assert_eq!(
            actions[0]["args"],
            serde_json::json!(["bento-settings", "location"])
        );

        // With one, the data program is asked for the weather there, and the calendar shows it.
        let place =
            serde_json::json!({ "name": "Reykjavik", "latitude": 64.15, "longitude": -21.94 });
        let style = serde_json::json!({ "gaps": 8, "config": { "location": place } });
        let outputs = [(String::from("DP-1"), (1920., 1080.))];
        host.ensure_loaded(&outputs, None, style);
        let asked = commands(&host)
            .into_iter()
            .find(|c| c["cmd"] == "weather")
            .expect("it asks for the weather");
        assert_eq!(asked["latitude"], 64.15);
        assert_eq!(asked["fahrenheit"], false);
        host.data_line(
            r#"{"type":"weather","ok":true,"temperature":4,"feels":0,"humidity":66,"wind":12,
            "windUnit":"km/h","unit":"°C","label":"Mostly clear","icon":"weather-clear-night-symbolic",
            "days":[{"date":"2026-11-09","label":"Overcast","icon":"weather-overcast-symbolic","min":-1,"max":4}]}"#
                .replace('\n', " ")
                .as_str(),
        );
        click(&mut host, "DP-1", [960., 26.]);
        settle(&mut host, &mut now);
        assert!(shows(&host, "DP-1", "4°C"));
        assert!(shows(&host, "DP-1", "Mostly clear"));
        assert!(shows(&host, "DP-1", "Reykjavik"));
        assert!(shows(&host, "DP-1", "4° -1°"));
        // The month is this one, with today in it.
        assert!(shows(&host, "DP-1", "Mo"));
    }

    #[test]
    fn a_notification_shows_as_a_toast_until_its_closed_or_its_time_is_up() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        let toast = r#"{"type":"notification","event":"shown","id":5,"app":"Mail",
            "summary":"New <b>message</b>","body":"Hello &amp; welcome","actions":[],
            "urgency":1,"timeout":60000,"image":null,"appIcon":null}"#
            .replace('\n', " ");
        host.data_line(&toast);
        settle(&mut host, &mut now);
        // Its markup is read, not shown.
        assert!(shows(&host, "DP-1", "New message"));
        assert!(shows(&host, "DP-1", "Hello & welcome"));

        // A click on one that isn't for anything sends it away, and so does its time running
        // out.
        let at = middle_of(&host, "DP-1", "New message");
        commands(&host);
        click(&mut host, "DP-1", at);
        let sent = commands(&host);
        assert_eq!(sent, [serde_json::json!({ "cmd": "dismiss", "id": 5 })]);
        host.data_line(r#"{"type":"notification","event":"closed","id":5}"#);
        settle(&mut host, &mut now);
        assert!(!shows(&host, "DP-1", "New message"));

        // One that says how long it stays is told to go once that's up.
        host.data_line(&toast.replace("60000", "1000").replace(":5,", ":6,"));
        assert!(commands(&host).is_empty());
        settle(&mut host, &mut now);
        assert!(commands(&host).contains(&serde_json::json!({ "cmd": "expire", "id": 6 })));
    }

    #[test]
    fn a_notifications_actions_are_buttons_and_reach_the_app_with_a_token() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        let toast = r#"{"type":"notification","event":"shown","id":7,"app":"Chat",
            "summary":"Sam","body":"Lunch?","actions":[["default",""],["reply","Reply"]],
            "urgency":1,"timeout":60000,"image":null,"appIcon":null}"#
            .replace('\n', " ");
        host.data_line(&toast);
        settle(&mut host, &mut now);
        commands(&host);

        // Its default action is its own click, and the others are its buttons.
        let at = middle_of(&host, "DP-1", "Lunch?");
        click(&mut host, "DP-1", at);
        let invoke = |key: &str| {
            serde_json::json!({ "cmd": "invoke", "id": 7, "key": key, "token": true })
        };
        assert_eq!(commands(&host), [invoke("default")]);
        let at = middle_of(&host, "DP-1", "Reply");
        click(&mut host, "DP-1", at);
        assert_eq!(commands(&host), [invoke("reply")]);

        // bento puts an activation token in for the app, so it may bring its window forward.
        let mut program = std::process::Command::new("cat")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = program.stdin.take().unwrap();
        let mut stdout = program.stdout.take().unwrap();
        host.data_started(program, stdin);
        let at = middle_of(&host, "DP-1", "Reply");
        click(&mut host, "DP-1", at);
        host.send_commands(|| String::from("a-token"));
        let (mut program, stdin) = host.data.take().unwrap();
        drop(stdin);
        let mut said = String::new();
        std::io::Read::read_to_string(&mut stdout, &mut said).unwrap();
        program.wait().unwrap();
        let sent: serde_json::Value = said
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .find(|command| command["cmd"] == "invoke")
            .unwrap();
        assert_eq!(sent["token"], "a-token");
        assert_eq!(sent["key"], "reply");
    }

    #[test]
    fn the_clipboard_button_opens_the_history_to_copy_from_and_the_shortcut_too() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        host.data_line(&DEVICES.replace('\n', " "));
        host.data_line(
            r#"{"type":"clipboard","available":true,"entries":[
                {"id":3,"kind":"text","text":"second   copy\nwith two lines"},
                {"id":2,"kind":"image","image":"/nowhere/clipboard-2.png"},
                {"id":1,"kind":"text","text":"first copy"}]}"#
                .replace('\n', " ")
                .as_str(),
        );
        settle(&mut host, &mut now);
        let button = middle_of(&host, "DP-1", "edit-paste-symbolic");
        click(&mut host, "DP-1", button);
        settle(&mut host, &mut now);
        // Newest first, each on a line.
        assert!(shows(&host, "DP-1", "second copy with two lines"));
        assert!(shows(&host, "DP-1", "first copy"));

        // Typing finds, and Enter copies what's found and closes it.
        commands(&host);
        type_keys(&mut host, &["f", "i", "r"]);
        settle(&mut host, &mut now);
        assert!(!shows(&host, "DP-1", "second copy with two lines"));
        type_keys(&mut host, &["Return"]);
        assert_eq!(
            commands(&host),
            [serde_json::json!({ "cmd": "clipboard-copy", "id": 1 })]
        );
        settle(&mut host, &mut now);
        assert!(!shows(&host, "DP-1", "first copy"));

        // The shortcut opens it on the focused output; a click on one copies it.
        host.event("toggle-clipboard", Some(String::from("DP-1")));
        settle(&mut host, &mut now);
        let at = middle_of(&host, "DP-1", "second copy with two lines");
        click(&mut host, "DP-1", at);
        assert_eq!(
            commands(&host),
            [serde_json::json!({ "cmd": "clipboard-copy", "id": 3 })]
        );
    }

    #[test]
    fn a_volume_changed_elsewhere_shows_its_level_at_the_bottom_for_a_moment() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        host.data_line(&DEVICES.replace('\n', " "));
        settle(&mut host, &mut now);
        assert!(!shows(&host, "DP-1", "60%"));

        // A key turns it up: the level grows out of nothing at the bottom middle, and goes.
        host.data_line(&DEVICES.replace('\n', " ").replace(
            r#""active":true,"volume":0.4"#,
            r#""active":true,"volume":0.6"#,
        ));
        for _ in 0..50 {
            frame(&mut host, &mut now);
        }
        assert!(shows(&host, "DP-1", "60%"));
        let at = middle_of(&host, "DP-1", "60%");
        assert!(at[1] > 900., "{at:?}");
        settle(&mut host, &mut now);
        assert!(!shows(&host, "DP-1", "60%"));

        // Another device being picked isn't a change of volume.
        let headphones = |volume: &str| {
            DEVICES
                .replace('\n', " ")
                .replace(r#""active":true,"volume":0.4"#, r#""active":false,"volume":0.4"#)
                .replace(
                    r#""active":false,"volume":0.8"#,
                    &format!(r#""active":true,"volume":{volume}"#),
                )
        };
        host.data_line(&headphones("0.8"));
        settle(&mut host, &mut now);
        assert!(!shows(&host, "DP-1", "80%"));

        // Nor is the control center's own slider moving it, which shows it already.
        open_control_center(&mut host, &mut now);
        let slider = middle_of(&host, "DP-1", "audio-volume-high-symbolic");
        click(&mut host, "DP-1", [slider[0] + 150., slider[1]]);
        host.data_line(&headphones("0.5"));
        for _ in 0..50 {
            frame(&mut host, &mut now);
        }
        assert!(!shows(&host, "DP-1", "50%"));
    }

    /// Times what bento does on its main thread for the shell, frame by frame, through first and
    /// later uses of its panels, on the machine's GPU, and prints the frames that took long. It
    /// needs a GPU, so it's run by hand, with `--nocapture`.
    #[test]
    #[ignore]
    fn it_times_the_shells_frames() {
        use smithay::backend::allocator::{Format as DrmFormat, Fourcc, Modifier};

        let started = Instant::now();
        let mut host = SceneHost::default();
        let outputs = [(String::from("DP-1"), (2560., 1440.))];
        host.ensure_loaded(&outputs, None, serde_json::json!({ "gaps": 8 }));
        host.set_workspaces(serde_json::json!({
            "DP-1": [{ "index": 0, "name": null, "active": true, "focused": true, "id": 7 }],
        }));
        host.data_line(&DEVICES.replace('\n', " "));
        println!("load: {:.1} ms", started.elapsed().as_secs_f64() * 1000.);

        let formats = [DrmFormat {
            code: Fourcc::Argb8888,
            modifier: Modifier::Linear,
        }];
        let mut renderer = super::super::vk::Renderer::new(None, &formats).unwrap();
        renderer.set_icon_theme("Adwaita");
        let target = renderer.create_target((2560, 1440)).unwrap();
        let mut now = Instant::now();

        // One frame: the script's timers, layout and springs, then what to draw, then drawing
        // it. Gives each part's time in milliseconds, and the glyphs and images it added.
        let mut frame = |host: &mut SceneHost, step: u64| {
            now += std::time::Duration::from_millis(step);
            let t = Instant::now();
            host.advance(now, Some(1.));
            let advance = t.elapsed().as_secs_f64() * 1000.;
            let t = Instant::now();
            let items = host.shell.as_ref().unwrap().scene.borrow().draw("DP-1");
            let draw = t.elapsed().as_secs_f64() * 1000.;
            let before = renderer.kept();
            let t = Instant::now();
            renderer.render(&target, &items, 1.).unwrap();
            let render = t.elapsed().as_secs_f64() * 1000.;
            let after = renderer.kept();
            (
                advance,
                draw,
                render,
                after.0.saturating_sub(before.0),
                after.1.saturating_sub(before.1),
                items.len(),
            )
        };
        let mut run = |host: &mut SceneHost, what: &str, event: f64, frames: usize| {
            let mut worst = (0f64, String::new());
            let mut total = 0.;
            let mut slow = 0;
            for i in 0..frames {
                let (advance, draw, render, glyphs, images, items) = frame(host, 6);
                let all = advance + draw + render + if i == 0 { event } else { 0. };
                total += all;
                if all > 2. {
                    slow += 1;
                }
                if all > worst.0 {
                    worst = (
                        all,
                        format!(
                            "frame {i}: script+layout {advance:.2}, items {draw:.2}, draw {render:.2} \
                             ({glyphs} glyphs, {images} images, {items} items)"
                        ),
                    );
                }
            }
            println!(
                "{what:<34} event {event:>6.2} ms | worst frame {:>6.2} ms | over 2 ms: {slow:>2}/{frames} | avg {:.2} | {}",
                worst.0,
                total / frames as f64,
                worst.1
            );
        };
        let timed = |f: &mut dyn FnMut()| {
            let t = Instant::now();
            f();
            t.elapsed().as_secs_f64() * 1000.
        };

        run(&mut host, "first frames", 0., 5);
        run(&mut host, "idle", 0., 30);
        for round in ["cold", "warm"] {
            println!("--- {round}");
            let e = timed(&mut || host.toggle_launcher(Some(String::from("DP-1"))));
            run(&mut host, "launcher opens", e, 40);
            for key in ["f", "i", "r"] {
                let e = timed(&mut || type_keys(&mut host, &[key]));
                run(&mut host, &format!("launcher: typed {key}"), e, 20);
            }
            let e = timed(&mut || type_keys(&mut host, &["Escape"]));
            run(&mut host, "launcher closes", e, 40);

            let status = middle_of(&host, "DP-1", "network-offline-symbolic");
            let e = timed(&mut || click(&mut host, "DP-1", status));
            run(&mut host, "control center opens", e, 50);
            let speakers = middle_of(&host, "DP-1", "Speakers");
            let e = timed(&mut || click(&mut host, "DP-1", speakers));
            run(&mut host, "device list opens", e, 40);
            let e = timed(&mut || type_keys(&mut host, &["Escape"]));
            run(&mut host, "control center closes", e, 50);

            let e = timed(&mut || click(&mut host, "DP-1", [1280., 26.]));
            run(&mut host, "calendar opens", e, 50);
            let e = timed(&mut || type_keys(&mut host, &["Escape"]));
            run(&mut host, "calendar closes", e, 50);

            let e = timed(&mut || {
                host.data_line(
                    r#"{"type":"notification","event":"shown","id":1,"app":"Mail","summary":"A notification","body":"With some text in it, as they have.","actions":[],"urgency":1,"timeout":60000,"image":null,"appIcon":null}"#,
                )
            });
            run(&mut host, "notification shows", e, 40);
            let e =
                timed(&mut || host.data_line(r#"{"type":"notification","event":"closed","id":1}"#));
            run(&mut host, "notification goes", e, 40);
            let e = timed(&mut || {});
            run(&mut host, "a second passes (clock)", e, 170);
        }
        renderer.destroy_target(target);
    }

    /// Draws the shell's panels on the machine's GPU into PNG files in the folder
    /// `BENTO_SHELL_SHOTS` names, to look at: the glass, which bento's compositing draws, is
    /// stood in for by a plain tint. It needs a GPU, so it's run by hand.
    #[test]
    #[ignore]
    fn it_draws_the_panels_to_look_at() {
        use smithay::backend::allocator::{Format as DrmFormat, Fourcc, Modifier};

        let Some(dir) = std::env::var_os("BENTO_SHELL_SHOTS").map(std::path::PathBuf::from) else {
            return;
        };
        let place = serde_json::json!({ "name": "Reykjavik, Iceland", "latitude": 64.15, "longitude": -21.94 });
        let style = serde_json::json!({ "gaps": 8, "config": { "location": place } });
        let (mut host, mut now) = loaded(style);
        host.set_minimap(serde_json::json!({
            "DP-1": { "view": [1920, 1080], "tiles": [
                { "id": 1, "x": -960, "y": 0, "w": 960, "h": 1080, "active": false, "floating": false },
                { "id": 2, "x": 0, "y": 0, "w": 960, "h": 1080, "active": true, "floating": false },
                { "id": 3, "x": 960, "y": 0, "w": 960, "h": 540, "active": false, "floating": false },
                { "id": 4, "x": 960, "y": 540, "w": 960, "h": 540, "active": false, "floating": false },
            ] },
        }));
        host.data_line(&DEVICES.replace('\n', " "));
        host.data_line(
            r#"{"type":"brightness","displays":[{"id":"DP-1","label":"Monitor One","value":0.35},{"id":"DP-2","label":"Monitor Two","value":0.8}]}"#,
        );
        host.data_line(r#"{"type":"theme","dark":false}"#);
        host.data_line(
            r#"{"type":"weather","ok":true,"temperature":4,"feels":0,"humidity":66,"wind":12,"windUnit":"km/h","unit":"°C","label":"Mostly clear","icon":"weather-clear-night-symbolic","days":[{"date":"2026-11-09","label":"Overcast","icon":"weather-overcast-symbolic","min":-1,"max":4},{"date":"2026-11-10","label":"Snow","icon":"weather-snow-symbolic","min":-3,"max":1},{"date":"2026-11-11","label":"Clear","icon":"weather-clear-symbolic","min":-5,"max":2},{"date":"2026-11-12","label":"Rain","icon":"weather-showers-symbolic","min":0,"max":6},{"date":"2026-11-13","label":"Fog","icon":"weather-fog-symbolic","min":1,"max":5}]}"#,
        );
        settle(&mut host, &mut now);

        let formats = [DrmFormat {
            code: Fourcc::Argb8888,
            modifier: Modifier::Linear,
        }];
        let mut renderer = super::super::vk::Renderer::new(None, &formats).unwrap();
        renderer.set_icon_theme("Adwaita");
        let target = renderer.create_target((1920, 1080)).unwrap();
        let mut shot = |host: &SceneHost, name: &str, [x0, y0, w, h]: [u32; 4]| {
            let items = host.shell.as_ref().unwrap().scene.borrow().draw("DP-1");
            renderer.render(&target, &items, 1.).unwrap();
            renderer.wait_for_loads();
            renderer.render(&target, &items, 1.).unwrap();
            let pixels = renderer.read_back(&target).unwrap();
            let glass: Vec<([f32; 4], [f32; 4])> = items
                .iter()
                .filter_map(|item| match item {
                    Item::Shape {
                        rect,
                        radii,
                        material: Material::Glass,
                        ..
                    } => Some((*rect, *radii)),
                    _ => None,
                })
                .collect();
            let mut rgb = Vec::with_capacity((w * h * 3) as usize);
            for y in y0..y0 + h {
                for x in x0..x0 + w {
                    // A wallpaper's worth of colour, then the glass as a light tint, then what
                    // the scene drew, whose pixels are BGRA with the alpha multiplied in.
                    let mut under = [70. + x as f32 / 20., 90., 120. + y as f32 / 10.];
                    let point = [x as f32 + 0.5, y as f32 + 0.5];
                    if glass
                        .iter()
                        .any(|(rect, radii)| super::super::inside_rounded(point, *rect, *radii))
                    {
                        under = under.map(|c| c * 0.2 + 244. * 0.8);
                    }
                    let px = &pixels[((y * 1920 + x) * 4) as usize..][..4];
                    let alpha = f32::from(px[3]) / 255.;
                    for c in [2, 1, 0] {
                        let value = f32::from(px[c]) + under[2 - c] * (1. - alpha);
                        rgb.push(value.clamp(0., 255.) as u8);
                    }
                }
            }
            let file = std::fs::File::create(dir.join(format!("{name}.png"))).unwrap();
            let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&rgb)
                .unwrap();
        };

        shot(&host, "bar-left", [0, 0, 640, 60]);
        shot(&host, "bar-right", [1280, 0, 640, 60]);

        click(&mut host, "DP-1", [960., 26.]);
        settle(&mut host, &mut now);
        shot(&host, "calendar", [740, 0, 440, 520]);
        type_keys(&mut host, &["Escape"]);
        settle(&mut host, &mut now);

        open_control_center(&mut host, &mut now);
        shot(&host, "control-center", [1500, 0, 420, 560]);
        let speakers = middle_of(&host, "DP-1", "Speakers");
        click(&mut host, "DP-1", speakers);
        settle(&mut host, &mut now);
        shot(&host, "control-center-list", [1500, 0, 420, 620]);
        type_keys(&mut host, &["Escape"]);
        settle(&mut host, &mut now);

        host.event("edit-workspace", Some(String::from("DP-1")));
        settle(&mut host, &mut now);
        type_keys(&mut host, &["d", "e", "v"]);
        settle(&mut host, &mut now);
        shot(&host, "workspace-editor", [700, 420, 520, 240]);
        type_keys(&mut host, &["Escape"]);
        settle(&mut host, &mut now);

        host.data_line(
            r#"{"type":"notification","event":"shown","id":1,"app":"Mail","summary":"Screenshot taken","body":"It's in your Pictures folder, and on the clipboard.","actions":[],"urgency":1,"timeout":60000,"image":null,"appIcon":null}"#,
        );
        settle(&mut host, &mut now);
        shot(&host, "toast", [1500, 0, 420, 160]);
        host.data_line(
            r#"{"type":"notification","event":"shown","id":2,"app":"Chat","summary":"Sam","body":"Lunch at noon?","actions":[["default",""],["reply","Reply"],["later","Remind Me Later"]],"urgency":1,"timeout":60000,"image":null,"appIcon":null}"#,
        );
        settle(&mut host, &mut now);
        shot(&host, "toast-actions", [1500, 0, 420, 320]);
        host.data_line(r#"{"type":"notification","event":"closed","id":1}"#);
        host.data_line(r#"{"type":"notification","event":"closed","id":2}"#);
        settle(&mut host, &mut now);

        // A picture to copy, which `BENTO_SHELL_PICTURE` can name.
        let picture = std::env::var("BENTO_SHELL_PICTURE").unwrap_or_default();
        host.data_line(
            &serde_json::json!({ "type": "clipboard", "available": true, "entries": [
                { "id": 4, "kind": "text", "text": "cargo test --lib scene:: -- --nocapture" },
                { "id": 3, "kind": "image", "image": picture },
                { "id": 2, "kind": "text", "text": "A long paragraph that was copied from a web page, which goes on for much longer than the panel is wide." },
                { "id": 1, "kind": "text", "text": "https://example.org/a/link" },
            ] })
            .to_string(),
        );
        host.event("toggle-clipboard", Some(String::from("DP-1")));
        settle(&mut host, &mut now);
        shot(&host, "clipboard", [1460, 0, 460, 420]);
        type_keys(&mut host, &["Escape"]);
        settle(&mut host, &mut now);

        host.data_line(&DEVICES.replace('\n', " ").replace(
            r#""active":true,"volume":0.4"#,
            r#""active":true,"volume":0.6"#,
        ));
        for _ in 0..50 {
            frame(&mut host, &mut now);
        }
        shot(&host, "volume-level", [660, 960, 600, 120]);
        settle(&mut host, &mut now);

        host.toggle_launcher(Some(String::from("DP-1")));
        settle(&mut host, &mut now);
        type_keys(&mut host, &["f", "i"]);
        settle(&mut host, &mut now);
        shot(&host, "launcher", [560, 240, 800, 420]);
        renderer.destroy_target(target);
    }

    #[test]
    fn only_where_the_scene_changed_is_drawn_again() {
        let shape = |x: f32, opacity: f32| Item::Shape {
            rect: [x, 10., 100., 36.],
            radii: [18.; 4],
            material: Material::None,
            fill: [1.; 4],
            border: (0., [0.; 4]),
            opacity,
            blur: 0.,
            clip: None,
        };
        let icon = Item::Image {
            rect: [900., 10., 16., 16.],
            source: super::super::ImageSource::Icon(String::from("folder-symbolic")),
            color: [1.; 4],
            opacity: 1.,
            scale: 1.,
            blur: 0.,
            clip: None,
        };
        let before = [shape(0., 1.), shape(400., 1.), icon.clone()];
        // Nothing changed: nothing's drawn again.
        assert!(changed(&before, &before, 1., false).is_empty());

        // One island fades: only around it, with the room its smoothed edges take.
        let after = [shape(0., 1.), shape(400., 0.5), icon.clone()];
        let rects = changed(&before, &after, 1., false);
        let around = rects.iter().copied().reduce(|a, b| a.merge(b)).unwrap();
        assert!(
            around.loc.x >= 390 && around.loc.x + around.size.w <= 510,
            "{around:?}"
        );
        assert!(around.size.h < 60, "{around:?}");

        // It moves: where it was and where it is. At twice the scale, twice the pixels.
        let moved = [shape(0., 1.), shape(600., 1.), icon.clone()];
        let rects = changed(&before, &moved, 2., false);
        let around = rects.iter().copied().reduce(|a, b| a.merge(b)).unwrap();
        assert!(
            around.loc.x >= 780 && around.loc.x + around.size.w <= 1420,
            "{around:?}"
        );

        // An icon that was loading came in: around the icons, not the shapes.
        let rects = changed(&before, &before, 1., true);
        assert_eq!(rects.len(), 1);
        assert!(rects[0].loc.x >= 890 && rects[0].size.w <= 30, "{rects:?}");

        // Something added at the end: around it.
        let more = [shape(0., 1.), shape(400., 1.), icon, shape(1500., 1.)];
        let rects = changed(&before, &more, 1., false);
        assert_eq!(rects.len(), 1);
        assert!(rects[0].loc.x >= 1490, "{rects:?}");
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
