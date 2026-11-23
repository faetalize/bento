//! bento's own shell in bento: loads the shell's script, moves its scene on, draws it per output
//! with the Vulkan renderer, and hands the result and its glass to bento's compositing.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
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
use super::settings::Settings;
use super::vk::{Renderer, Target};
use super::{Band, Item, Material, NodeId, Theme, CLICK};
use crate::backend::tty::{TtyFrame, TtyRenderer, TtyRendererError};
use crate::render_helpers::background_effect::BackgroundEffectElement;
use crate::render_helpers::framebuffer_effect::FramebufferEffect;
use crate::render_helpers::glass::{self, GlassFrame};
use crate::render_helpers::primary_gpu_texture::PrimaryGpuTextureRenderElement;
use crate::render_helpers::renderer::AsGlesFrame as _;
use crate::render_helpers::texture::{TextureBuffer, TextureRenderElement};

/// bento's unix shell, which runs when the config names no shell, or the one it names fails to
/// load first.
pub const UNIX: &str = "bento:shells/unix";

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

/// How far a box that scrolls moves for what a wheel's notch or a touchpad says.
const SCROLL_SPEED: f64 = 3.;

/// How close islands come for their glass to melt together.
const MELT: f32 = 24.;
/// Images per output, so the scene never draws into one bento's compositing is still reading.
const TARGETS: usize = 3;

/// What an output's surfaces in one band show, drawn into their own picture, which goes where the
/// band is among what else bento draws.
struct OutputScene {
    targets: Vec<(Target, GlesTexture)>,
    next: usize,
    /// What was last drawn, to show.
    shown: Option<TextureBuffer<GlesTexture>>,
    /// What it was drawn from, to tell what the next frame changes.
    items: Vec<Item>,
    /// What bento's compositing knows the scene's picture by, from frame to frame, and where it
    /// changed in each: only there is the screen drawn again. Bands under the windows are drawn
    /// with each workspace, by its own id each.
    ids: std::cell::RefCell<HashMap<Option<usize>, Id>>,
    damage: DamageBag<i32, Physical>,
    /// The glass under what was last drawn.
    glass: Vec<GlassPiece>,
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

/// When each of the shell's files last changed.
type Mtimes = Vec<(std::path::PathBuf, Option<SystemTime>)>;

#[derive(Default)]
pub struct SceneHost {
    shell: Option<Shell>,
    /// The module the shell starts from and when its files had last changed when they were
    /// loaded, or failed to.
    loaded_mtimes: Option<(String, Mtimes)>,
    /// Why the shell the config names failed to load, if it did.
    load_error: Option<String>,
    /// The files the shell the config names read as it failed to load, watched with the running
    /// shell's, so that fixing any of them loads it again.
    failed_files: Vec<PathBuf>,
    /// The shell's settings, which outlive a shell loading again.
    settings: Option<Rc<RefCell<Settings>>>,
    /// Whether bento's reference copy of the shell is written, for editors.
    wrote_reference: bool,
    renderer: Option<Renderer>,
    renderer_failed: bool,
    /// Each output's scene, by band.
    outputs: HashMap<(String, Band), OutputScene>,
    last_advance: Option<Instant>,
    /// Goes up whenever the scene changed or moved.
    generation: u64,
    moving: bool,
    sent_workspaces: Option<serde_json::Value>,
    sent_fullscreen: Option<serde_json::Value>,
    sent_overview: Option<bool>,
    sent_windows: Option<serde_json::Value>,
    sent_current_output: Option<serde_json::Value>,
    /// The notifications showing, by id, to tell a shell loaded again.
    notifications: Vec<serde_json::Value>,
    known_outputs: Vec<String>,
    /// Where the pointer is over the shell: the output, the point on it, and the band of the
    /// surface it's over.
    pointer: Option<(String, [f32; 2], Band)>,
    /// The nodes the pointer is on that hear it come and go: the topmost and what holds it.
    hovered: Vec<NodeId>,
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
    presses: HashMap<u32, (Option<NodeId>, String, [f32; 2], Band)>,
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
    /// Loads the shell that starts from module `entry`, again when its files changed or it's
    /// another one, onto `outputs`, each with its logical size, with bento's own shell and modules
    /// read from `dir` if it's given, and its settings and binds from `settings_file`. A shell
    /// that fails to load leaves the last one, or bento's unix shell when there's none yet.
    pub fn ensure_loaded(
        &mut self,
        outputs: &[(String, (f32, f32))],
        dir: Option<&Path>,
        entry: &str,
        settings_file: Option<PathBuf>,
        style: serde_json::Value,
    ) {
        if !self.wrote_reference {
            self.wrote_reference = true;
            if !cfg!(test) {
                super::reference::write();
            }
        }
        let mut other_file = false;
        let settings = match &self.settings {
            Some(settings) => {
                other_file = settings.borrow_mut().set_path(settings_file);
                settings.clone()
            }
            None => self
                .settings
                .insert(Rc::new(RefCell::new(Settings::load(settings_file))))
                .clone(),
        };

        // A shell of one's own is watched by the files it read, not the folder it's in, and by
        // the ones it read as it failed to load.
        let mut loaded = self
            .shell
            .as_ref()
            .map(|shell| shell.files())
            .unwrap_or_default();
        loaded.extend(self.failed_files.iter().cloned());
        let mtimes = (String::from(entry), script::mtimes(dir, entry, &loaded));
        if self.loaded_mtimes.as_ref() != Some(&mtimes) {
            self.loaded_mtimes = Some(mtimes);
            let mut loaded = Shell::load(outputs, dir, entry, style.clone(), settings.clone());
            self.load_error = loaded.as_ref().err().map(|err| format!("{err:#}"));
            self.failed_files = match &loaded {
                Ok(_) => Vec::new(),
                Err(err) => err.files.clone(),
            };
            if let Err(err) = &loaded {
                warn!("error loading the shell: {err:?}");
                if self.shell.is_none() && entry != UNIX {
                    loaded = Shell::load(outputs, dir, UNIX, style.clone(), settings.clone());
                }
            }
            match loaded {
                Ok(shell) => {
                    for data in self.data_state.values() {
                        shell.send(data);
                    }
                    for notification in &self.notifications {
                        shell.send(notification);
                    }
                    // What it read is known now: watch those from here on.
                    let mut files = shell.files();
                    files.extend(self.failed_files.iter().cloned());
                    self.loaded_mtimes =
                        Some((String::from(entry), script::mtimes(dir, entry, &files)));
                    self.shell = Some(shell);
                    self.known_outputs = outputs.iter().map(|(n, _)| n.clone()).collect();
                    self.sent_workspaces = None;
                    self.sent_fullscreen = None;
                    self.sent_overview = None;
                    self.sent_windows = None;
                    self.sent_current_output = None;
                    self.generation += 1;
                    settings.borrow().warn_problems();
                }
                Err(err) => {
                    warn!("error loading the shell: {err:?}");
                    // What it read is known now too.
                    let mut files = self.shell.as_ref().map(Shell::files).unwrap_or_default();
                    files.extend(self.failed_files.iter().cloned());
                    self.loaded_mtimes =
                        Some((String::from(entry), script::mtimes(dir, entry, &files)));
                }
            }
        } else if settings.borrow_mut().reload_if_changed() || other_file {
            if let Some(shell) = &self.shell {
                let values = settings.borrow().all_values();
                shell.send(&serde_json::json!({ "type": "settings", "settings": values }));
            }
            settings.borrow().warn_problems();
        }

        let Some(shell) = &self.shell else { return };
        let restyled = shell.scene.borrow().style != style;
        if restyled {
            shell.scene.borrow_mut().style = style.clone();
            shell.send(&serde_json::json!({ "type": "style", "style": style }));
        }
        let mut scene = shell.scene.borrow_mut();
        let mut resized = false;
        for (name, size) in outputs {
            resized |= scene.output_size(name).is_some_and(|old| old != *size);
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
            self.outputs.retain(|(output, _), _| output != name);
        }
        let added: Vec<String> = outputs
            .iter()
            .map(|(n, _)| n.clone())
            .filter(|n| !self.known_outputs.contains(n))
            .collect();
        drop(scene);
        if !added.is_empty() || !gone.is_empty() || resized {
            self.known_outputs = outputs.iter().map(|(n, _)| n.clone()).collect();
            shell.send(&serde_json::json!({ "type": "outputs", "outputs": added }));
        }
    }

    /// The shell that's drawn and its settings, for bento's settings, with `source` saying where
    /// it comes from.
    pub fn ipc_info(&self, source: bento_ipc::ShellSource) -> bento_ipc::Shell {
        let settings = self.settings.as_ref().map(|s| s.borrow());
        let blocks = settings.as_ref().map(|s| s.ipc()).unwrap_or_default();
        let (unused, _) = settings.as_ref().map(|s| s.problems()).unwrap_or_default();
        bento_ipc::Shell {
            source,
            error: self.load_error.clone(),
            settings_error: settings.as_ref().and_then(|s| s.error().map(String::from)),
            settings: blocks
                .into_iter()
                .map(|(name, settings)| bento_ipc::ShellSettings { name, settings })
                .collect(),
            actions: settings
                .as_ref()
                .map(|s| s.owned_actions())
                .unwrap_or_default()
                .into_iter()
                .map(|(module, action)| bento_ipc::ShellAction { module, action })
                .collect(),
            unused,
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
        if kind == "notification" {
            // The ones showing are kept, to show in a shell loaded again.
            let id = data["id"].clone();
            self.notifications.retain(|n| n["id"] != id);
            if data["event"] != "closed" {
                let mut shown = data.clone();
                shown["event"] = serde_json::json!("shown");
                self.notifications.push(shown);
            }
        } else {
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

    /// The commands of the apps the shell asked to start since this was last asked.
    pub fn take_launches(&self) -> Vec<Vec<String>> {
        self.shell
            .as_ref()
            .map(|shell| shell.take_launches())
            .unwrap_or_default()
    }

    /// The bento actions the shell asked to run since this was last asked.
    pub fn take_actions(&self) -> Vec<serde_json::Value> {
        self.shell
            .as_ref()
            .map(|shell| shell.take_actions())
            .unwrap_or_default()
    }

    /// The binds the running shell's actions have by default.
    pub fn default_binds(&self) -> Vec<niri_config::binds::Bind> {
        let settings = self.settings.as_ref().map(|s| s.borrow());
        settings.map(|s| s.binds()).unwrap_or_default()
    }

    /// The user's binds for the running shell's actions, from its file.
    pub fn user_binds(&self) -> Vec<niri_config::binds::Bind> {
        let settings = self.settings.as_ref().map(|s| s.borrow());
        settings.map(|s| s.user_binds()).unwrap_or_default()
    }

    /// The running shell's actions the user turned off in its file.
    pub fn unbound(&self) -> Vec<niri_config::Action> {
        let settings = self.settings.as_ref().map(|s| s.borrow());
        settings.map(|s| s.unbound()).unwrap_or_default()
    }

    /// Whether the pointer is over the shell.
    pub fn pointer_over(&self) -> bool {
        self.pointer.is_some()
    }

    /// The pointer moved to `at`, the output and the point on it, if it's over the shell, or off
    /// it. Tells nodes it comes to or leaves that hear it, and says how the pointer looks there,
    /// if it's over the shell.
    pub fn pointer_moved(&mut self, at: Option<(&str, [f32; 2], Band)>) -> Option<String> {
        let Some(shell) = &self.shell else {
            return None;
        };
        self.pointer = at.map(|(name, point, band)| (String::from(name), point, band));
        let (hovered, cursor) = match at {
            Some((name, point, band)) => {
                let scene = shell.scene.borrow();
                (
                    scene.hovered(name, point, Some(band)),
                    Some(
                        scene
                            .cursor(name, point, Some(band))
                            .unwrap_or_else(|| String::from("default")),
                    ),
                )
            }
            None => (Vec::new(), None),
        };
        if hovered != self.hovered {
            for id in self.hovered.iter().filter(|id| !hovered.contains(id)) {
                shell.send(&serde_json::json!({ "type": "hover", "id": id, "inside": false }));
            }
            for id in hovered.iter().filter(|id| !self.hovered.contains(id)).rev() {
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
        for (pressed, output, _, _) in self.presses.values() {
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
    pub fn press(&mut self, key: u32, button: u8, at: Option<(&str, [f32; 2], Band)>) -> bool {
        let Some(shell) = &self.shell else {
            return false;
        };
        let Some((name, point, band)) = at else {
            shell.send(&serde_json::json!({ "type": "down", "surface": null, "button": button }));
            return false;
        };
        // Which surface it's on, for the others to hear of a press off them.
        let surface = shell.scene.borrow().surface_at(name, point, Some(band));
        shell.send(&serde_json::json!({ "type": "down", "surface": surface, "button": button }));
        // What's dragged needn't take clicks.
        let hit = shell
            .scene
            .borrow()
            .hit(name, point, CLICK | super::DRAG, Some(band));
        if let Some(id) = hit {
            shell.send(&serde_json::json!({
                "type": "press", "id": id, "button": button, "x": point[0], "y": point[1],
                "output": name,
            }));
        }
        self.presses
            .insert(key, (hit, String::from(name), point, band));
        true
    }

    /// The press keyed by `key` ended where it was pressed, as a touch does: a click on what it
    /// pressed. Says whether the press was the shell's.
    pub fn release_where_pressed(&mut self, key: u32, button: u8) -> bool {
        let Some((_, name, point, band)) = self.presses.get(&key).cloned() else {
            return false;
        };
        self.release(key, button, Some((&name, point, band)))
    }

    /// The press keyed by `key` ended, with what it pressed under `at`, the output and the point,
    /// or not over the shell: a click on what it pressed if it's still there. Says whether the
    /// press was the shell's.
    pub fn release(&mut self, key: u32, button: u8, at: Option<(&str, [f32; 2], Band)>) -> bool {
        let Some((pressed, _, _, _)) = self.presses.remove(&key) else {
            return false;
        };
        let Some(shell) = &self.shell else {
            return true;
        };
        // What was pressed hears it let go, wherever that is.
        if let Some(id) = pressed {
            shell.send(&serde_json::json!({ "type": "release", "id": id, "button": button }));
        }
        if let (Some(id), Some((name, point, band))) = (pressed, at) {
            if shell.scene.borrow().hit(name, point, CLICK, Some(band)) == Some(id) {
                shell.send(&serde_json::json!({
                    "type": "click", "id": id, "button": button, "x": point[0], "y": point[1],
                    "output": name,
                }));
            }
        }
        true
    }

    /// Scrolls by `delta` where the pointer is, and says whether the shell took it.
    pub fn scroll(&mut self, delta: [f64; 2]) -> bool {
        let (Some(shell), Some((name, point, band))) = (&self.shell, &self.pointer) else {
            return false;
        };
        let target = shell
            .scene
            .borrow()
            .scroll_target(name, *point, Some(*band));
        match target {
            // What scrolls in its box moves as far as the wheel goes, bento's own way.
            Some((id, true)) => {
                shell
                    .scene
                    .borrow_mut()
                    .scroll(id, (delta[1] * SCROLL_SPEED) as f32);
                self.generation += 1;
            }
            Some((id, false)) => shell.send(&serde_json::json!({
                "type": "scroll", "id": id, "dx": delta[0], "dy": delta[1],
            })),
            None => (),
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

    /// Tells the shell whether the overview is open, when that changes.
    pub fn set_overview(&mut self, open: bool) {
        if self.sent_overview == Some(open) {
            return;
        }
        if let Some(shell) = &self.shell {
            shell.send(&serde_json::json!({ "type": "overview", "open": open }));
        }
        self.sent_overview = Some(open);
    }

    /// Runs one of the shell's own actions, as a bind asked, with the current output.
    pub fn shell_action(&self, name: &str, args: &[String], output: Option<String>) {
        if let Some(shell) = &self.shell {
            shell.send(&serde_json::json!({
                "type": "shell-action", "name": name, "args": args, "output": output,
            }));
        }
    }

    /// Tells the shell which output is current, by its name, when that changes.
    pub fn set_current_output(&mut self, output: Option<String>) {
        let output = serde_json::json!(output);
        if self.sent_current_output.as_ref() == Some(&output) {
            return;
        }
        if let Some(shell) = &self.shell {
            shell.send(&serde_json::json!({ "type": "current-output", "output": output }));
        }
        self.sent_current_output = Some(output);
    }

    /// Tells the shell the windows, when they change.
    pub fn set_windows(&mut self, windows: serde_json::Value) {
        if self.sent_windows.as_ref() == Some(&windows) {
            return;
        }
        if let Some(shell) = &self.shell {
            shell.send(&serde_json::json!({ "type": "windows", "windows": windows }));
        }
        self.sent_windows = Some(windows);
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
        let laid_out = scene.take_laid_out();
        let scrolled = scene.take_scrolled();
        drop(scene);
        for id in settled {
            shell.send(&serde_json::json!({ "type": "settled", "id": id }));
        }
        for (id, [x, y, width, height]) in laid_out {
            shell.send(&serde_json::json!({
                "type": "layout", "id": id, "x": x, "y": y, "width": width, "height": height,
            }));
        }
        for (id, [top, reach, height]) in scrolled {
            shell.send(&serde_json::json!({
                "type": "scrolled", "id": id, "top": top, "reach": reach, "height": height,
            }));
        }
        // What hears the pointer near may have moved under it, and what it does then is drawn.
        let heard = self.update_near();
        moving || heard
    }

    /// Whether the shell's surfaces in `band` cover `point` on output `output`, so they take
    /// the pointer there.
    pub fn covers(&self, output: &str, point: [f32; 2], band: Band) -> bool {
        self.shell
            .as_ref()
            .is_some_and(|shell| shell.scene.borrow().covers(output, point, Some(band)))
    }

    /// The bands output `output` has surfaces in, from the back to the front.
    pub fn bands(&self, output: &str) -> Vec<Band> {
        self.shell
            .as_ref()
            .map(|shell| shell.scene.borrow().bands(output))
            .unwrap_or_default()
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
        // Each band with surfaces is drawn; one whose last surface went is drawn empty once, to
        // let its pictures go.
        let mut bands = scene.bands(output);
        for (name, band) in self.outputs.keys() {
            if name == output && !bands.contains(band) {
                bands.push(*band);
            }
        }
        for band in bands {
            let items = scene.draw(output, band);
            let state = self
                .outputs
                .entry((String::from(output), band))
                .or_insert_with(|| OutputScene {
                    targets: Vec::new(),
                    next: 0,
                    shown: None,
                    items: Vec::new(),
                    ids: Default::default(),
                    damage: DamageBag::default(),
                    glass: Vec::new(),
                    effect: FramebufferEffect::new(),
                    drawn: None,
                });
            draw_band(renderer, vk, state, items, size, scale, self.generation);
        }
        came
    }

    /// Whether icons or images the scene shows are still being loaded: it's drawn again when
    /// they're there.
    pub fn loading(&self) -> bool {
        self.renderer.as_ref().is_some_and(Renderer::loading)
    }

    /// Pushes what output `output`'s surfaces in `band` show, over their glass. Bands under the
    /// windows are pushed once with each workspace, `ns` being which.
    pub fn render_content(
        &self,
        output: &str,
        band: Band,
        ns: Option<usize>,
        push: &mut dyn FnMut(SceneElement),
    ) {
        let Some(state) = self.outputs.get(&(String::from(output), band)) else {
            return;
        };
        let Some(buffer) = &state.shown else {
            return;
        };
        let id = state
            .ids
            .borrow_mut()
            .entry(ns)
            .or_insert_with(Id::new)
            .clone();
        push(SceneElement {
            texture: PrimaryGpuTextureRenderElement(TextureRenderElement::from_texture_buffer(
                buffer.clone(),
                (0., 0.),
                1.,
                None,
                None,
                Kind::Unspecified,
            )),
            id,
            damage: state.damage.snapshot(),
        });
    }

    /// The window copies in output `output`'s surfaces in `band`, which bento draws over its
    /// picture.
    pub fn copies(&self, output: &str, band: Band) -> Vec<super::CopyItem> {
        self.shell
            .as_ref()
            .map(|shell| shell.scene.borrow().draw_with_copies(output, band).1)
            .unwrap_or_default()
    }

    /// Pushes the glass under output `output`'s surfaces in `band`.
    #[allow(clippy::too_many_arguments)]
    pub fn render_glass(
        &self,
        output: &str,
        band: Band,
        ns: Option<usize>,
        scale: f64,
        blur: niri_config::Blur,
        frame: GlassFrame,
        push: &mut dyn FnMut(BackgroundEffectElement),
    ) {
        let Some(state) = self.outputs.get(&(String::from(output), band)) else {
            return;
        };
        if !state.glass.is_empty() {
            glass::render_rects(
                &state.effect,
                ns,
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

/// A piece of glass under the shell: its rectangle, its corners' radii, and how opaque it is.
pub type GlassPiece = ([f32; 4], [f32; 4], f32);

/// The glass under `items`, with each piece as opaque as its shape is, so it fades with it.
/// A shape cut by what clips it is the part of it inside, with its corners, so glass sliding out
/// from behind an edge shows only past the edge.
fn glass_of(items: &[Item]) -> Vec<GlassPiece> {
    items
        .iter()
        .filter_map(|item| match item {
            Item::Shape {
                rect,
                radii,
                material: Material::Glass | Material::Acrylic | Material::Mica,
                opacity,
                clip,
                ..
            } if *opacity > 0.01 => {
                let rect = match clip {
                    Some(clip) => cut(*rect, clip.rect)?,
                    None => *rect,
                };
                let most = rect[2].min(rect[3]) / 2.;
                Some((rect, radii.map(|r| r.min(most)), *opacity))
            }
            _ => None,
        })
        .collect()
}

/// The part of `rect` inside `clip`, if any.
fn cut(rect: [f32; 4], clip: [f32; 4]) -> Option<[f32; 4]> {
    let x0 = rect[0].max(clip[0]);
    let y0 = rect[1].max(clip[1]);
    let x1 = (rect[0] + rect[2]).min(clip[0] + clip[2]);
    let y1 = (rect[1] + rect[3]).min(clip[1] + clip[3]);
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
}

#[cfg(test)]
impl SceneHost {
    /// A point on output `output` where the pointer is over the shell, in the band of the
    /// frontmost surface there, as bento's input finds it.
    fn at(output: &str, point: [f32; 2]) -> Option<(&str, [f32; 2], Band)> {
        Some((output, point, AT_BAND.with(|b| b.get())))
    }

    /// Whether any of the shell's surfaces cover `point` on output `output`.
    pub fn covers_any(&self, output: &str, point: [f32; 2]) -> bool {
        let found = self.shell.as_ref().and_then(|shell| {
            let scene = shell.scene.borrow();
            Band::ALL
                .into_iter()
                .rev()
                .find(|band| scene.covers(output, point, Some(*band)))
        });
        if let Some(band) = found {
            AT_BAND.with(|b| b.set(band));
        }
        found.is_some()
    }
}

#[cfg(test)]
thread_local! {
    /// The band the last point a test looked at is in.
    static AT_BAND: std::cell::Cell<Band> = const { std::cell::Cell::new(Band::OverlayBelow) };
}

/// Draws `items`, what an output's surfaces in a band show, into `state`'s pictures, `size`
/// physical pixels at `scale`, if they changed since generation `generation` was drawn.
fn draw_band(
    renderer: &mut GlesRenderer,
    vk: &mut Renderer,
    state: &mut OutputScene,
    items: Vec<Item>,
    size: (u32, u32),
    scale: f64,
    generation: u64,
) {
    let now = (generation, scale, vk.epoch());
    if state.drawn == Some(now) {
        return;
    }
    let rescaled = state.drawn.is_none_or(|(_, before, _)| before != scale);
    let images = state.drawn.is_some_and(|(_, _, epoch)| epoch != vk.epoch());
    state.drawn = Some(now);
    // The scene changed, but not in what this band shows: its picture stays.
    let same =
        items.len() == state.items.len() && items.iter().zip(&state.items).all(|(a, b)| a.same(b));
    if same && !rescaled && !images && (state.shown.is_some() || items.is_empty()) {
        return;
    }

    state.glass = glass_of(&items);
    // Showing nothing, it takes no images, and nothing is drawn over what's below.
    if items.is_empty() {
        state.shown = None;
        state.items = items;
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
    fn a_row_lit_under_the_pointer_goes_back_when_it_leaves_and_shows_a_hand() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        open_control_center(&mut host, &mut now);
        let power = middle_of(&host, "DP-1", "system-shutdown-symbolic");
        click(&mut host, "DP-1", power);
        settle(&mut host, &mut now);
        // The small shapes filled where the Restart row is: its hover.
        let restart = middle_of(&host, "DP-1", "Restart");
        let lit = |host: &SceneHost| {
            host.shell
                .as_ref()
                .unwrap()
                .scene
                .borrow()
                .draw_all("DP-1")
                .into_iter()
                .filter(|item| match item {
                    Item::Shape {
                        rect,
                        material: Material::None,
                        fill,
                        ..
                    } => {
                        fill[3] > 0.
                            && super::super::inside_rounded(restart, *rect, [0.; 4])
                            && rect[2] < 400.
                    }
                    _ => false,
                })
                .count()
        };
        assert_eq!(lit(&host), 0);
        host.covers_any("DP-1", restart);
        let cursor = host.pointer_moved(SceneHost::at("DP-1", restart));
        assert_eq!(
            cursor.as_deref(),
            Some("pointer"),
            "a row that does something shows a hand"
        );
        settle(&mut host, &mut now);
        assert_eq!(lit(&host), 1, "lit under the pointer");
        host.pointer_moved(None);
        settle(&mut host, &mut now);
        assert_eq!(lit(&host), 0, "not once it's left");
    }

    #[test]
    fn the_launcher_shows_its_field_every_time_it_opens() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        let field = |host: &SceneHost| {
            let scene = host.shell.as_ref().unwrap().scene.borrow();
            scene
                .seen("DP-1", "Search apps")
                .first()
                .map(|(_, opacity, _)| *opacity)
        };
        for time in 0..3 {
            host.shell_action("toggle-launcher", &[], Some(String::from("DP-1")));
            settle(&mut host, &mut now);
            let shown = field(&host);
            assert!(shown.is_some_and(|o| o > 0.99), "opening {time}: {shown:?}");
            type_keys(&mut host, &["Escape"]);
            settle(&mut host, &mut now);
        }
    }

    #[test]
    fn the_launcher_finds_files_under_apps_and_alone_with_its_pill() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        host.data_line(r#"{"type":"file-search","available":true}"#);
        host.shell_action("toggle-launcher", &[], Some(String::from("DP-1")));
        settle(&mut host, &mut now);
        assert!(shows(
            &host,
            "DP-1",
            "Search apps, or / for files and folders"
        ));
        commands(&host);
        // What no app is called, which every key looks for.
        type_keys(&mut host, &["q", "z", "x"]);
        let asked = commands(&host);
        assert_eq!(asked.len(), 3, "{asked:?}");
        assert_eq!(asked[2]["query"], "qzx");
        let found = |host: &mut SceneHost, id: &serde_json::Value, count: usize| {
            let files: Vec<_> = (0..count)
                .map(|i| {
                    serde_json::json!({
                        "path": format!("/h/d/qzx-{i}.txt"), "name": format!("qzx-{i}.txt"),
                        "location": "~/d", "folder": false, "icon": "text-x-generic",
                    })
                })
                .collect();
            let said = serde_json::json!({ "type": "files", "id": id, "files": files });
            host.data_line(&said.to_string());
        };
        found(&mut host, &asked[2]["id"], 10);
        settle(&mut host, &mut now);
        // Alone, files take the room of both, and the rest are a row away. No header says what
        // the only kind is.
        assert!(shows(&host, "DP-1", "qzx-7.txt") && !shows(&host, "DP-1", "qzx-8.txt"));
        assert!(shows(&host, "DP-1", "Show All 10 Files"));
        assert!(shows(&host, "DP-1", "~/d"));

        // `/` lights the pill and isn't typed: all of them show, and the field still searches
        // what it did.
        type_keys(&mut host, &["/"]);
        settle(&mut host, &mut now);
        assert!(shows(&host, "DP-1", "qzx-9.txt") && !shows(&host, "DP-1", "Show All 10 Files"));
        assert!(commands(&host).is_empty());

        // The selected file shows in its folder with Ctrl+Enter, which closes the launcher.
        type_keys(&mut host, &["Down", "Down"]);
        host.key(&EngineKey {
            name: String::from("Return"),
            text: String::new(),
            modifiers: [false, true, false, false],
            pressed: true,
            repeat: false,
        });
        settle(&mut host, &mut now);
        assert_eq!(
            commands(&host),
            [serde_json::json!({ "cmd": "files-show", "path": "/h/d/qzx-2.txt", "token": true })]
        );
        assert!(!host.wants_keyboard());

        // Opened again, it starts empty, with the pill off; `/` anywhere lights it, and again
        // puts it out.
        host.shell_action("toggle-launcher", &[], Some(String::from("DP-1")));
        settle(&mut host, &mut now);
        assert!(shows(
            &host,
            "DP-1",
            "Search apps, or / for files and folders"
        ));
        type_keys(&mut host, &["/"]);
        settle(&mut host, &mut now);
        assert!(shows(&host, "DP-1", "Search files and folders"));
        type_keys(&mut host, &["/"]);
        settle(&mut host, &mut now);
        assert!(shows(
            &host,
            "DP-1",
            "Search apps, or / for files and folders"
        ));
        // Backspace in the empty field puts it out too.
        type_keys(&mut host, &["/", "BackSpace"]);
        settle(&mut host, &mut now);
        assert!(shows(
            &host,
            "DP-1",
            "Search apps, or / for files and folders"
        ));

        // Escape empties the field, then puts the pill out, then closes, and Enter opens the
        // selected file.
        commands(&host);
        type_keys(&mut host, &["/", "q"]);
        let asked = commands(&host);
        found(&mut host, &asked[0]["id"], 3);
        settle(&mut host, &mut now);
        type_keys(&mut host, &["Escape"]);
        settle(&mut host, &mut now);
        assert!(shows(&host, "DP-1", "Search files and folders") && host.wants_keyboard());
        type_keys(&mut host, &["Escape"]);
        settle(&mut host, &mut now);
        assert!(shows(
            &host,
            "DP-1",
            "Search apps, or / for files and folders"
        ));
        assert!(host.wants_keyboard());
        type_keys(&mut host, &["/", "q"]);
        let asked = commands(&host);
        found(&mut host, &asked.last().unwrap()["id"], 3);
        settle(&mut host, &mut now);
        type_keys(&mut host, &["Return"]);
        settle(&mut host, &mut now);
        assert_eq!(
            commands(&host),
            [serde_json::json!({ "cmd": "files-open", "path": "/h/d/qzx-0.txt", "token": true })]
        );
        assert!(!host.wants_keyboard());
    }

    #[test]
    fn the_search_settings_say_where_files_are_found_and_whether_with_apps() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        let settings = host.settings.clone().unwrap();
        settings.borrow_mut().write_file(
            "search {\n    files-with-apps false\n    folders \"~\" \"/data\"\n    \
             excluded-folders \"~/Games\"\n}\n",
        );
        let outputs = [(String::from("DP-1"), (1920., 1080.))];
        host.ensure_loaded(&outputs, None, UNIX, None, serde_json::json!({ "gaps": 8 }));
        settle(&mut host, &mut now);
        // They show on the Shell page.
        let info = host.ipc_info(bento_ipc::ShellSource::BuiltIn {
            name: String::from("unix"),
        });
        let search = info
            .settings
            .iter()
            .find(|block| block.name == "search")
            .unwrap();
        let names: Vec<&str> = search.settings.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["files-with-apps", "folders", "excluded-folders"]);
        assert!(search
            .settings
            .iter()
            .all(|s| s.page.as_deref() == Some("shell")));

        host.data_line(r#"{"type":"file-search","available":true}"#);
        host.shell_action("toggle-launcher", &[], Some(String::from("DP-1")));
        settle(&mut host, &mut now);
        commands(&host);
        // Apps alone, as files aren't found with them; the pill still finds them, where the
        // settings say.
        type_keys(&mut host, &["q"]);
        assert!(commands(&host).is_empty());
        type_keys(&mut host, &["/"]);
        let asked = commands(&host);
        assert_eq!(asked.len(), 1, "{asked:?}");
        assert_eq!(asked[0]["query"], "q");
        assert_eq!(asked[0]["folders"], serde_json::json!(["~", "/data"]));
        assert_eq!(asked[0]["excluded"], serde_json::json!(["~/Games"]));
    }

    #[test]
    fn with_no_folders_to_search_no_files_are_searched() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        let settings = host.settings.clone().unwrap();
        settings
            .borrow_mut()
            .write_file("search {\n    folders\n}\n");
        let outputs = [(String::from("DP-1"), (1920., 1080.))];
        host.ensure_loaded(&outputs, None, UNIX, None, serde_json::json!({ "gaps": 8 }));
        settle(&mut host, &mut now);
        host.data_line(r#"{"type":"file-search","available":true}"#);
        host.shell_action("toggle-launcher", &[], Some(String::from("DP-1")));
        settle(&mut host, &mut now);
        commands(&host);
        // Nothing says files are found, and nothing looks for them.
        assert!(shows(&host, "DP-1", "Search apps"));
        assert!(!shows(&host, "DP-1", "for files and folders"));
        type_keys(&mut host, &["q", "/"]);
        assert!(commands(&host).is_empty());
    }

    #[test]
    fn an_island_open_on_one_output_stays_open_as_whats_in_it_is_pressed() {
        let mut host = SceneHost::default();
        let outputs = [
            (String::from("DP-1"), (1920., 1080.)),
            (String::from("DP-2"), (2560., 1440.)),
        ];
        host.ensure_loaded(&outputs, None, UNIX, None, serde_json::json!({ "gaps": 8 }));
        let mut now = Instant::now();
        settle(&mut host, &mut now);
        host.data_line(&DEVICES.replace('\n', " "));
        open_control_center(&mut host, &mut now);
        // On its sound page, which it'd leave if it closed or opened again.
        open_page(&mut host, &mut now, 0);
        assert!(shows(&host, "DP-1", "Output"));
        // A press in it that nothing takes.
        let words = middle_of(&host, "DP-1", "Output");
        click(&mut host, "DP-1", words);
        settle(&mut host, &mut now);
        assert!(host.wants_keyboard(), "a press in it keeps it open");
        assert!(shows(&host, "DP-1", "Output"), "it stays on its page");
    }

    #[test]
    fn the_launcher_opens_where_the_focus_is_takes_typing_and_closes_on_escape() {
        let mut host = SceneHost::default();
        let outputs = [
            (String::from("DP-1"), (1920., 1080.)),
            (String::from("DP-2"), (2560., 1440.)),
        ];
        host.ensure_loaded(&outputs, None, UNIX, None, serde_json::json!({ "gaps": 8 }));
        let mut now = Instant::now();
        settle(&mut host, &mut now);
        // How much the launcher draws on an output: what's below the islands.
        let overlay = |host: &SceneHost, output: &str| {
            host.shell
                .as_ref()
                .unwrap()
                .scene
                .borrow()
                .draw_all(output)
                .into_iter()
                .filter(|item| match item {
                    Item::Shape { rect, .. } | Item::Image { rect, .. } => rect[1] > 100.,
                    Item::Text { origin, .. } => origin[1] > 100.,
                })
                .count()
        };

        host.shell_action("toggle-launcher", &[], Some(String::from("DP-2")));
        settle(&mut host, &mut now);
        assert!(overlay(&host, "DP-2") > 0, "it opens on the focused output");
        // In the middle of the output at its tallest, 640 high, and only the search bar until
        // something's typed.
        let panel = glass_below_islands(&host, "DP-2").pop().unwrap();
        assert!((panel[1] - (1440. - 640.) / 2.).abs() < 1., "{panel:?}");
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

        // Escape empties the field first, and then closes it.
        host.key(&key("Escape", ""));
        settle(&mut host, &mut now);
        assert!(overlay(&host, "DP-2") > 0, "Escape empties it");
        host.key(&key("Escape", ""));
        settle(&mut host, &mut now);
        assert_eq!(overlay(&host, "DP-2"), 0, "Escape closes it");
        assert!(!host.wants_keyboard());

        // Opened again while it shrinks away, it's the same launcher turning around.
        host.shell_action("toggle-launcher", &[], Some(String::from("DP-2")));
        now += std::time::Duration::from_millis(30);
        host.advance(now, Some(1.));
        host.shell_action("toggle-launcher", &[], Some(String::from("DP-2")));
        host.shell_action("toggle-launcher", &[], Some(String::from("DP-2")));
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
            .draw_all(output)
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
        host.ensure_loaded(&outputs, None, UNIX, None, serde_json::json!({ "gaps": 8 }));
        // Close to real time, which the script's timers run on.
        let mut now = Instant::now();
        let mut step = |host: &mut SceneHost| {
            now += std::time::Duration::from_millis(16);
            host.advance(now, Some(1.));
        };
        host.shell_action("toggle-launcher", &[], Some(String::from("DP-1")));
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
        host.shell_action("toggle-launcher", &[], None);
        for _ in 0..12 {
            step(&mut host);
        }
        let closing = launcher_glass(&host).unwrap();
        host.shell_action("toggle-launcher", &[], Some(String::from("DP-1")));
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
        host.ensure_loaded(&outputs, None, UNIX, None, serde_json::json!({ "gaps": 8 }));
        let mut now = Instant::now();
        settle(&mut host, &mut now);
        let drawn = |host: &SceneHost, output: &str| {
            host.shell
                .as_ref()
                .unwrap()
                .scene
                .borrow()
                .draw_all(output)
                .len()
        };
        assert!(drawn(&host, "DP-1") > 0);

        host.set_fullscreen(serde_json::json!({ "DP-1": true, "DP-2": false }));
        settle(&mut host, &mut now);
        // Nothing is drawn, so nothing is over the window, and it has the pointer everywhere.
        assert_eq!(drawn(&host, "DP-1"), 0, "they're gone");
        assert!(!host.covers_any("DP-1", [960., 26.]));
        assert!(!host.covers_any("DP-1", [960., 0.]));
        assert!(drawn(&host, "DP-2") > 0, "only where the window is");

        // The pointer at the top edge brings them back, and on them it keeps them.
        host.pointer_at(Some(("DP-1", [960., 0.])));
        settle(&mut host, &mut now);
        assert!(drawn(&host, "DP-1") > 0, "they're back");
        assert!(host.covers_any("DP-1", [960., 26.]));
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
        host.press(1, 1, SceneHost::at("DP-1", [960., 26.]));
        host.release(1, 1, SceneHost::at("DP-1", [960., 26.]));
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
        assert!(host.covers_any("DP-1", [960., 26.]));
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
        assert!(
            host.covers_any(output, point),
            "nothing to click at {point:?}"
        );
        host.press(1, 1, SceneHost::at(output, point));
        host.release(1, 1, SceneHost::at(output, point));
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

    #[test]
    fn the_shell_follows_its_settings_file_as_it_changes() {
        super::super::modules::TEST_ENTRY
            .with(|entry| *entry.borrow_mut() = Some(String::from("bento:shells/windows")));
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        assert_eq!(host.reserved("DP-1"), [0., 0., 48., 0.]);

        let settings = host.settings.clone().unwrap();
        settings
            .borrow_mut()
            .write_file("windows {\n    height 60\n}\n");
        let outputs = [(String::from("DP-1"), (1920., 1080.))];
        host.ensure_loaded(&outputs, None, UNIX, None, serde_json::json!({ "gaps": 8 }));
        now += Duration::from_millis(16);
        host.advance(now, None);
        assert_eq!(host.reserved("DP-1"), [0., 0., 60., 0.]);

        // The start button and the apps are in the middle, or on the left.
        let start = middle_of(&host, "DP-1", "ph-squares-four-symbolic");
        assert!(start[0] > 600., "{start:?}");
        settings
            .borrow_mut()
            .write_file("windows {\n    alignment \"left\"\n}\n");
        host.ensure_loaded(&outputs, None, UNIX, None, serde_json::json!({ "gaps": 8 }));
        settle(&mut host, &mut now);
        let start = middle_of(&host, "DP-1", "ph-squares-four-symbolic");
        assert!(start[0] < 60., "{start:?}");

        // What the running shell declares, for bento's settings.
        let info = host.ipc_info(bento_ipc::ShellSource::BuiltIn {
            name: String::from("windows"),
        });
        let names: Vec<&str> = info
            .settings
            .iter()
            .map(|block| block.name.as_str())
            .collect();
        assert!(names.contains(&"windows"), "{names:?}");
        assert!(names.contains(&"taskbar"), "{names:?}");
        assert!(info.unused.is_empty());
        // The start menu's shortcut is the start menu's own action, not bento's, on Mod alone
        // by default.
        let actions: Vec<&str> = info
            .actions
            .iter()
            .map(|a| a.action.name.as_str())
            .collect();
        assert!(actions.contains(&"toggle-start-menu"), "{actions:?}");
        assert!(!actions.contains(&"toggle-launcher"), "{actions:?}");
        let binds = host.default_binds();
        let start = binds.iter().find(|bind| bind.key == "Mod".parse().unwrap());
        let start = start.and_then(|bind| bind.release_action());
        assert_eq!(
            start,
            Some(&niri_config::Action::ShellAction(
                String::from("toggle-start-menu"),
                Vec::new()
            ))
        );
    }

    #[test]
    fn the_start_menu_comes_out_of_the_start_button() {
        super::super::modules::TEST_ENTRY
            .with(|entry| *entry.borrow_mut() = Some(String::from("bento:shells/windows")));
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        let settings = host.settings.clone().unwrap();
        let outputs = [(String::from("DP-1"), (1920., 1080.))];
        // Where the start menu's left edge is, by its heading, and the start button's middle.
        let places = |host: &mut SceneHost, now: &mut Instant| {
            host.shell_action("toggle-start-menu", &[], Some(String::from("DP-1")));
            settle(host, now);
            let heading = host
                .shell
                .as_ref()
                .unwrap()
                .scene
                .borrow()
                .find("DP-1", "All apps");
            let button = middle_of(host, "DP-1", "ph-squares-four-symbolic");
            host.shell_action("toggle-start-menu", &[], Some(String::from("DP-1")));
            settle(host, now);
            (heading[0][0], button[0])
        };

        // On a taskbar in the middle, around the button.
        let (menu, button) = places(&mut host, &mut now);
        let middle = menu - 24. + 336.;
        assert!(
            (middle - button).abs() < 40.,
            "menu {menu}, button {button}"
        );

        // On one to the left, from the button's left edge, not in the middle of the output.
        settings
            .borrow_mut()
            .write_file("windows {\n    alignment \"left\"\n}\n");
        host.ensure_loaded(&outputs, None, UNIX, None, serde_json::json!({ "gaps": 8 }));
        settle(&mut host, &mut now);
        let (menu, button) = places(&mut host, &mut now);
        assert!(menu < 100., "menu {menu}, button {button}");
        super::super::modules::TEST_ENTRY.with(|entry| *entry.borrow_mut() = None);
    }

    #[test]
    fn the_windows_shell_has_a_taskbar_with_its_apps_previews_and_start_menu() {
        super::super::modules::TEST_ENTRY
            .with(|entry| *entry.borrow_mut() = Some(String::from("bento:shells/windows")));
        let shell = super::super::script::Shell::load(
            &[(String::from("DP-1"), (1920., 1080.))],
            None,
            UNIX,
            serde_json::json!({ "gaps": 8 }),
            Rc::default(),
        );
        if let Err(err) = &shell {
            panic!("{err:#}");
        }
        drop(shell);
        let (mut host, mut now, button) = windows_shell_with_an_app();
        // Along the bottom, keeping its height from the windows.
        let reserved = host.reserved("DP-1");
        assert_eq!(reserved, [0., 0., 48., 0.]);
        assert!(shows(&host, "DP-1", "ph-squares-four-symbolic"));

        // Two windows of one app are one button, whose previews are live copies of both.
        host.pointer_at(Some(("DP-1", button)));
        settle(&mut host, &mut now);
        let copies: Vec<u64> = Band::ALL
            .into_iter()
            .flat_map(|band| host.copies("DP-1", band))
            .map(|copy| copy.window)
            .collect();
        assert_eq!(copies, [1, 2], "a preview of each window");
        assert!(shows(&host, "DP-1", "Note 2"));

        // A click goes to the next window, as the first has the focus.
        commands(&host);
        click(&mut host, "DP-1", button);
        let ran = host.take_actions();
        assert_eq!(ran[0]["name"], "focus-window");
        assert_eq!(ran[0]["window"], 2);

        // A right click opens the shell's menu over the button, which takes the keys: Down
        // lights its item, and Return runs it and closes the menu.
        host.pointer_at(None);
        settle(&mut host, &mut now);
        host.press(3, 3, SceneHost::at("DP-1", button));
        host.release(3, 3, SceneHost::at("DP-1", button));
        settle(&mut host, &mut now);
        assert!(shows(&host, "DP-1", "Close All Windows"));
        let item = middle_of(&host, "DP-1", "Close All Windows");
        assert!(item[1] < button[1], "the menu is over the button");
        assert!(host.wants_keyboard());
        type_keys(&mut host, &["Down", "Return"]);
        settle(&mut host, &mut now);
        let ran = host.take_actions();
        let closed: Vec<&serde_json::Value> = ran.iter().map(|a| &a["window"]).collect();
        assert_eq!(closed, [1, 2]);
        assert!(ran.iter().all(|a| a["name"] == "close-window"));
        assert!(!shows(&host, "DP-1", "Close All Windows"));
        assert!(!host.wants_keyboard());

        // What can be clicked shows a hand.
        let start = middle_of(&host, "DP-1", "ph-squares-four-symbolic");
        host.covers_any("DP-1", start);
        let cursor = host.pointer_moved(SceneHost::at("DP-1", start));
        assert_eq!(cursor.as_deref(), Some("pointer"));
        host.covers_any("DP-1", button);
        let cursor = host.pointer_moved(SceneHost::at("DP-1", button));
        assert_eq!(cursor.as_deref(), Some("pointer"));
        host.pointer_moved(None);

        // The start button opens the start menu, which takes the keys.
        click(&mut host, "DP-1", start);
        settle(&mut host, &mut now);
        assert!(host.wants_keyboard());
        assert!(shows(&host, "DP-1", "Search apps"));
        type_keys(&mut host, &["Escape"]);
        settle(&mut host, &mut now);
        assert!(!host.wants_keyboard());
        super::super::modules::TEST_ENTRY.with(|entry| *entry.borrow_mut() = None);
    }

    /// The windows shell, with two windows of one app on its taskbar, and a point on the app's
    /// button.
    fn windows_shell_with_an_app() -> (SceneHost, Instant, [f32; 2]) {
        super::super::modules::TEST_ENTRY
            .with(|entry| *entry.borrow_mut() = Some(String::from("bento:shells/windows")));
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        let window = |id: u32, focused: bool| {
            serde_json::json!({
                "id": id, "appId": "org.example.Editor", "title": format!("Note {id}"),
                "workspace": 7, "output": "DP-1", "focused": focused, "urgent": false,
                "floating": false,
            })
        };
        host.set_windows(serde_json::json!([window(1, true), window(2, false)]));
        settle(&mut host, &mut now);
        let buttons = host
            .shell
            .as_ref()
            .unwrap()
            .scene
            .borrow()
            .find("DP-1", "application-x-executable");
        assert_eq!(buttons.len(), 1, "one button for the app: {buttons:?}");
        let button = [buttons[0][0] + 4., buttons[0][1] + 4.];
        (host, now, button)
    }

    /// The windows the shell shows live copies of.
    fn copied(host: &SceneHost) -> Vec<u64> {
        Band::ALL
            .into_iter()
            .flat_map(|band| host.copies("DP-1", band))
            .map(|copy| copy.window)
            .collect()
    }

    /// The marks under the apps' buttons on the taskbar, from the left: how wide each is.
    fn marks(host: &SceneHost) -> Vec<f32> {
        let scene = host.shell.as_ref().unwrap().scene.borrow();
        let mut marks: Vec<[f32; 4]> = Band::ALL
            .into_iter()
            .flat_map(|band| scene.draw("DP-1", band))
            .filter_map(|item| match item {
                // The overview's button has pips as wide as they're tall.
                Item::Shape { rect, .. } if rect[1] > 1032. && rect[3] == 3. && rect[2] > 3. => {
                    Some(rect)
                }
                _ => None,
            })
            .collect();
        marks.sort_by(|a, b| a[0].total_cmp(&b[0]));
        marks.into_iter().map(|rect| rect[2]).collect()
    }

    #[test]
    fn a_taskbar_button_has_a_mark_for_each_window() {
        let (mut host, mut now, _) = windows_shell_with_an_app();
        // The first has the focus.
        assert_eq!(marks(&host), [16., 6.]);

        let mut show = |ids: std::ops::RangeInclusive<u32>, focused: u32| {
            let windows: Vec<serde_json::Value> = ids
                .map(|id| {
                    serde_json::json!({
                        "id": id, "appId": "org.example.Editor", "title": format!("Note {id}"),
                        "workspace": 7, "output": "DP-1", "focused": id == focused,
                        "urgent": false, "floating": false,
                    })
                })
                .collect();
            host.set_windows(windows.into());
            settle(&mut host, &mut now);
            marks(&host)
        };
        assert_eq!(show(1..=2, 2), [6., 16.], "the long one follows the focus");
        assert_eq!(show(1..=2, 0), [6., 6.]);
        assert_eq!(show(1..=1, 1), [16.]);
        assert_eq!(show(1..=3, 1), [16., 6., 6.]);
        // A button has room for three: the last stands for the rest.
        assert_eq!(show(1..=5, 1), [16., 6., 6.]);
        assert_eq!(show(1..=5, 5), [6., 6., 16.]);
        super::super::modules::TEST_ENTRY.with(|entry| *entry.borrow_mut() = None);
    }

    #[test]
    fn a_taskbar_button_keeps_its_windows_order_as_one_is_raised() {
        let (mut host, mut now, button) = windows_shell_with_an_app();
        // A floating window with the focus comes first, as it's raised over the others.
        let window = |id: u32, focused: bool| {
            serde_json::json!({
                "id": id, "appId": "org.example.Editor", "title": format!("Note {id}"),
                "workspace": 7, "output": "DP-1", "focused": focused, "urgent": false,
                "floating": true,
            })
        };
        host.set_windows(serde_json::json!([
            window(2, true),
            window(1, false),
            window(3, false)
        ]));
        settle(&mut host, &mut now);
        assert_eq!(marks(&host), [6., 16., 6.]);

        // A click goes on to the third, rather than back to the first.
        commands(&host);
        host.take_actions();
        click(&mut host, "DP-1", button);
        let ran = host.take_actions();
        assert_eq!(ran[0]["name"], "focus-window");
        assert_eq!(ran[0]["window"], 3);

        host.pointer_at(Some(("DP-1", button)));
        settle(&mut host, &mut now);
        assert_eq!(copied(&host), [1, 2, 3]);
        super::super::modules::TEST_ENTRY.with(|entry| *entry.borrow_mut() = None);
    }

    /// The windows shell on an output with the workspaces `names`, the one at `active` current.
    fn windows_shell_with_workspaces(
        names: &[Option<&str>],
        active: usize,
    ) -> (SceneHost, Instant) {
        super::super::modules::TEST_ENTRY
            .with(|entry| *entry.borrow_mut() = Some(String::from("bento:shells/windows")));
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        show_workspaces(&mut host, &mut now, names, active);
        commands(&host);
        host.take_actions();
        (host, now)
    }

    fn show_workspaces(
        host: &mut SceneHost,
        now: &mut Instant,
        names: &[Option<&str>],
        active: usize,
    ) {
        let list: Vec<serde_json::Value> = names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                serde_json::json!({
                    "index": index, "name": name, "active": index == active,
                    "focused": index == active, "id": 10 + index,
                })
            })
            .collect();
        host.set_workspaces(serde_json::json!({ "DP-1": list }));
        settle(host, now);
    }

    /// The pips on the overview's button, from the top: how tall each is drawn, and how opaque.
    fn pips(host: &SceneHost) -> Vec<(f32, f32)> {
        let scene = host.shell.as_ref().unwrap().scene.borrow();
        let mut pips: Vec<([f32; 4], f32)> = Band::ALL
            .into_iter()
            .flat_map(|band| scene.draw("DP-1", band))
            .filter_map(|item| match item {
                Item::Shape { rect, opacity, .. }
                    if rect[1] > 1032. && rect[2] <= 3. && rect[3] <= 10. && opacity > 0. =>
                {
                    Some((rect, opacity))
                }
                _ => None,
            })
            .collect();
        pips.sort_by(|a, b| a.0[1].total_cmp(&b.0[1]));
        pips.into_iter().map(|(rect, o)| (rect[3], o)).collect()
    }

    #[test]
    fn the_overviews_button_says_which_workspace_is_current() {
        let names = [None, Some("web"), None];
        let (mut host, mut now) = windows_shell_with_workspaces(&names, 0);
        assert!(shows(&host, "DP-1", "Workspace 1"));
        let heights = |host: &SceneHost| -> Vec<f32> {
            pips(host).into_iter().map(|(height, _)| height).collect()
        };
        assert_eq!(heights(&host), [10., 3., 3.]);

        show_workspaces(&mut host, &mut now, &names, 1);
        assert!(shows(&host, "DP-1", "web"));
        assert!(!shows(&host, "DP-1", "Workspace 1"));
        assert_eq!(heights(&host), [3., 10., 3.]);

        // Of many, five show around the current one, smaller at an end with more past it.
        let many = [None; 12];
        show_workspaces(&mut host, &mut now, &many, 5);
        let shown = pips(&host);
        assert_eq!(shown.len(), 5, "{shown:?}");
        assert_eq!(shown[2].0, 10., "{shown:?}");
        assert!(shown[0].0 < 3. && shown[4].0 < 3., "{shown:?}");
        show_workspaces(&mut host, &mut now, &many, 11);
        let shown = pips(&host);
        assert_eq!(shown.len(), 5, "{shown:?}");
        assert_eq!(shown[4].0, 10., "{shown:?}");
        assert!(shown[0].0 < 3., "{shown:?}");

        // The icon is filled while the overview is open.
        assert!(!shows(&host, "DP-1", "ph-cards-three-fill-symbolic"));
        host.set_overview(true);
        settle(&mut host, &mut now);
        assert!(shows(&host, "DP-1", "ph-cards-three-fill-symbolic"));
        super::super::modules::TEST_ENTRY.with(|entry| *entry.borrow_mut() = None);
    }

    #[test]
    fn the_overviews_button_shows_what_its_settings_say() {
        let names = [None, Some("web"), None];
        let (mut host, mut now) = windows_shell_with_workspaces(&names, 1);
        let settings = host.settings.clone().unwrap();
        let outputs = [(String::from("DP-1"), (1920., 1080.))];
        let mut set = |host: &mut SceneHost, text: &str| {
            settings.borrow_mut().write_file(text);
            host.ensure_loaded(&outputs, None, UNIX, None, serde_json::json!({ "gaps": 8 }));
            settle(host, &mut now);
        };
        assert!(!shows(&host, "DP-1", "2"), "no number in the icon");
        set(&mut host, "overview {\n    workspace-number true\n}\n");
        assert!(shows(&host, "DP-1", "2"));

        // With all of it off, it's an icon that opens the overview and no more.
        set(
            &mut host,
            "overview {\n    workspace-names false\n    workspace-pips false\n    \
             hover-previews false\n    scroll-to-switch false\n}\n",
        );
        assert!(!shows(&host, "DP-1", "web"));
        assert!(pips(&host).is_empty());
        let button = middle_of(&host, "DP-1", "ph-cards-three-symbolic");
        assert!(host.covers_any("DP-1", button));
        host.pointer_moved(SceneHost::at("DP-1", button));
        host.pointer_at(Some(("DP-1", button)));
        host.scroll([0., 15.]);
        set(&mut host, "overview {\n    workspace-names false\n    hover-previews false\n    scroll-to-switch false\n}\n");
        assert!(host.take_actions().is_empty());
        assert!(!shows(&host, "DP-1", "Workspace 3"), "no previews");
        click(&mut host, "DP-1", button);
        assert_eq!(host.take_actions()[0]["name"], "toggle-overview");
        super::super::modules::TEST_ENTRY.with(|entry| *entry.borrow_mut() = None);
    }

    /// Shells that use it keep working.
    #[test]
    fn minimap_says_where_the_active_workspaces_windows_are() {
        let dir = std::env::temp_dir().join(format!("bento-minimap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("main.tsx");
        std::fs::write(
            &main,
            "import { Surface, minimap } from \"bento\";\n\
             export default () => (\n\
             <Surface output=\"DP-1\" anchor={[\"top\", \"left\"]}>\n\
             <text>{JSON.stringify(minimap(\"DP-1\"))}</text>\n\
             </Surface>\n);\n",
        )
        .unwrap();
        let mut host = SceneHost::default();
        let outputs = [(String::from("DP-1"), (1920., 1080.))];
        let entry = main.to_string_lossy().into_owned();
        host.ensure_loaded(
            &outputs,
            None,
            &entry,
            None,
            serde_json::json!({ "gaps": 8 }),
        );
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(host.load_error, None);
        let mut now = Instant::now();
        show_workspaces(&mut host, &mut now, &[None, None], 0);
        let window = |id: u64, workspace: u64, rect: serde_json::Value| {
            serde_json::json!({
                "id": id, "workspace": workspace, "output": "DP-1", "active": id == 1,
                "focused": false, "floating": false, "rect": rect,
            })
        };
        let rect = serde_json::json!({ "x": -8, "y": 4, "width": 960, "height": 540 });
        host.set_windows(serde_json::json!([
            window(1, 10, rect.clone()),
            window(2, 10, serde_json::Value::Null),
            window(3, 11, rect),
        ]));
        settle(&mut host, &mut now);
        let wanted =
            "{\"view\":[1920,1080],\"tiles\":[{\"id\":1,\"x\":-8,\"y\":4,\"w\":960,\"h\":540,\
                      \"active\":true,\"floating\":false}]}";
        assert!(shows(&host, "DP-1", wanted));
    }

    #[test]
    fn a_right_click_on_the_overviews_button_renames_and_unnames_the_workspace() {
        let (mut host, mut now) = windows_shell_with_workspaces(&[None, Some("web")], 1);
        let button = middle_of(&host, "DP-1", "web");
        assert!(host.covers_any("DP-1", button));
        host.press(3, 3, SceneHost::at("DP-1", button));
        host.release(3, 3, SceneHost::at("DP-1", button));
        settle(&mut host, &mut now);
        let item = middle_of(&host, "DP-1", "Rename");
        assert!(item[1] < button[1], "the menu is over the button");
        click(&mut host, "DP-1", item);
        settle(&mut host, &mut now);
        assert!(!shows(&host, "DP-1", "Rename"));
        assert!(host.wants_keyboard());
        assert!(shows(&host, "DP-1", "Workspace 2"));
        type_keys(&mut host, &["s"]);
        let ran = host.take_actions();
        assert_eq!(ran[0]["name"], "set-workspace-name");
        assert_eq!(ran[0]["args"], serde_json::json!(["webs"]));
        assert_eq!(ran[0]["workspace"], 11);
        type_keys(&mut host, &["Escape"]);
        settle(&mut host, &mut now);

        let right_click = |host: &mut SceneHost, now: &mut Instant| {
            host.press(3, 3, SceneHost::at("DP-1", button));
            host.release(3, 3, SceneHost::at("DP-1", button));
            settle(host, now);
        };
        assert!(host.covers_any("DP-1", button));
        right_click(&mut host, &mut now);
        let item = middle_of(&host, "DP-1", "Unname");
        host.take_actions();
        click(&mut host, "DP-1", item);
        let ran = host.take_actions();
        assert_eq!(ran[0]["name"], "unset-workspace-name");
        assert_eq!(ran[0]["workspace"], 11);
        settle(&mut host, &mut now);

        // One without a name has none to take away.
        show_workspaces(&mut host, &mut now, &[None, None], 1);
        assert!(host.covers_any("DP-1", button));
        right_click(&mut host, &mut now);
        assert!(shows(&host, "DP-1", "Rename"));
        assert!(!shows(&host, "DP-1", "Unname"));
        super::super::modules::TEST_ENTRY.with(|entry| *entry.borrow_mut() = None);
    }

    #[test]
    fn workspaces_offer_no_renaming_without_an_editor() {
        let dir = std::env::temp_dir().join(format!("bento-no-editor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("main.tsx");
        std::fs::write(
            &main,
            "import { Surface } from \"bento\";\n\
             import { Workspaces } from \"bento:modules/workspaces\";\n\
             export default () => (\n\
             <Surface output=\"DP-1\" anchor={[\"top\", \"left\"]}><Workspaces /></Surface>\n);\n",
        )
        .unwrap();
        let mut host = SceneHost::default();
        let outputs = [(String::from("DP-1"), (1920., 1080.))];
        let entry = main.to_string_lossy().into_owned();
        host.ensure_loaded(
            &outputs,
            None,
            &entry,
            None,
            serde_json::json!({ "gaps": 8 }),
        );
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(host.load_error, None);
        let mut now = Instant::now();
        show_workspaces(&mut host, &mut now, &[None, Some("web")], 0);
        for (label, unname) in [("1", false), ("web", true)] {
            let at = middle_of(&host, "DP-1", label);
            assert!(host.covers_any("DP-1", at));
            host.press(3, 3, SceneHost::at("DP-1", at));
            host.release(3, 3, SceneHost::at("DP-1", at));
            settle(&mut host, &mut now);
            assert!(!shows(&host, "DP-1", "Rename"));
            assert_eq!(shows(&host, "DP-1", "Unname"), unname);
        }
    }

    #[test]
    fn scrolling_over_the_overviews_button_switches_and_holds_its_previews_back() {
        let names = [None, Some("web"), None];
        let (mut host, mut now) = windows_shell_with_workspaces(&names, 0);
        let window = |id: u64, workspace: u64, rect: serde_json::Value| {
            serde_json::json!({
                "id": id, "workspace": workspace, "output": "DP-1", "active": true,
                "floating": false, "rect": rect,
            })
        };
        let full = serde_json::json!({ "x": 0, "y": 0, "width": 960, "height": 540 });
        // From the front to the back, and one that isn't anywhere.
        host.set_windows(serde_json::json!([
            window(1, 10, full.clone()),
            window(2, 10, full.clone()),
            window(3, 11, full),
            window(4, 11, serde_json::Value::Null),
        ]));
        settle(&mut host, &mut now);
        // The end of its name, which is off a button that fits a shorter one.
        let scene = host.shell.as_ref().unwrap().scene.borrow();
        let name = scene.find("DP-1", "Workspace 1")[0];
        drop(scene);
        let button = [name[0] + name[2] - 2., name[1] + name[3] / 2.];
        assert!(host.covers_any("DP-1", button));
        host.pointer_moved(SceneHost::at("DP-1", button));
        host.pointer_at(Some(("DP-1", button)));
        settle(&mut host, &mut now);
        // Every workspace's windows, the one in front over the others.
        assert_eq!(copied(&host), [2, 1, 3]);
        let scale = 180. / 1920.;
        let copies: Vec<_> = Band::ALL
            .into_iter()
            .flat_map(|band| host.copies("DP-1", band))
            .collect();
        assert!((copies[0].rect[2] - 960. * scale).abs() < 0.5);

        // A notch goes one on.
        host.scroll([0., 15.]);
        let ran = host.take_actions();
        assert_eq!(ran[0]["name"], "focus-workspace");
        assert_eq!(ran[0]["workspace"], 11);
        // It's still under the pointer, though its name is shorter.
        show_workspaces(&mut host, &mut now, &names, 1);
        let held = middle_of(&host, "DP-1", "web");
        host.scroll([0., 15.]);
        let ran = host.take_actions();
        assert!(!ran.is_empty(), "the button went from under the pointer");
        assert_eq!(ran[0]["workspace"], 12);
        // Past the last there's nowhere to go, and it still holds them back.
        show_workspaces(&mut host, &mut now, &names, 2);
        for _ in 0..12 {
            host.scroll([0., 15.]);
            now += Duration::from_millis(100);
            host.advance(now, Some(1.));
        }
        assert!(host.take_actions().is_empty());
        assert_eq!(copied(&host), Vec::<u64>::new(), "still scrolling");
        settle(&mut host, &mut now);
        assert_eq!(copied(&host), [2, 1, 3]);
        host.scroll([0., 15.]);

        // Leaving while they're held back brings none, and the button fits its name again.
        host.pointer_moved(None);
        host.pointer_at(None);
        show_workspaces(&mut host, &mut now, &names, 1);
        assert_eq!(copied(&host), Vec::<u64>::new());
        assert!(middle_of(&host, "DP-1", "web")[0] > held[0] + 5.);

        // A click on a preview goes to its workspace.
        let button = middle_of(&host, "DP-1", "web");
        host.pointer_at(Some(("DP-1", button)));
        settle(&mut host, &mut now);
        let scene = host.shell.as_ref().unwrap().scene.borrow();
        let label = scene.find("DP-1", "web").into_iter().find(|f| f[1] < 1032.);
        let label = label.expect("no preview of the workspace");
        drop(scene);
        let web = [label[0] + 4., label[1] + 4.];
        click(&mut host, "DP-1", web);
        let ran = host.take_actions();
        assert_eq!(ran[0]["name"], "focus-workspace");
        assert_eq!(ran[0]["workspace"], 11);
        super::super::modules::TEST_ENTRY.with(|entry| *entry.borrow_mut() = None);
    }

    #[test]
    fn previews_go_back_into_the_taskbar_and_not_past_it() {
        let (mut host, mut now) = windows_shell_with_workspaces(&[None, None], 0);
        host.set_windows(serde_json::json!([{
            "id": 1, "workspace": 10, "output": "DP-1", "active": true, "floating": false,
            "appId": "org.example.Editor", "title": "Note",
            "rect": { "x": 0, "y": 0, "width": 1920, "height": 1032 },
        }]));
        settle(&mut host, &mut now);
        for icon in ["ph-cards-three-symbolic", "application-x-executable"] {
            let button = middle_of(&host, "DP-1", icon);
            host.pointer_at(Some(("DP-1", button)));
            settle(&mut host, &mut now);
            assert_eq!(copied(&host), [1]);
            host.pointer_at(None);
            let mut seen = 0;
            for _ in 0..120 {
                now += Duration::from_millis(16);
                host.advance(now, Some(1.));
                for copy in Band::ALL.into_iter().flat_map(|b| host.copies("DP-1", b)) {
                    seen += 1;
                    let clip = copy.clip.expect("it's cut to its panel");
                    let drawn = (clip.rect[1] + clip.rect[3]).min(copy.rect[1] + copy.rect[3]);
                    assert!(drawn <= 1030.5, "over the taskbar: {copy:?}");
                }
            }
            assert!(seen > 0);
            assert_eq!(copied(&host), Vec::<u64>::new());
        }
        super::super::modules::TEST_ENTRY.with(|entry| *entry.borrow_mut() = None);
    }

    /// How far above the taskbar the glass of the panel open over it is, once it's settled.
    fn above_taskbar(host: &mut SceneHost, now: &mut Instant) -> f32 {
        settle(host, now);
        let scene = host.shell.as_ref().unwrap().scene.borrow();
        let items: Vec<Item> = Band::ALL
            .into_iter()
            .flat_map(|band| scene.draw("DP-1", band))
            .collect();
        let glass = glass_of(&items);
        let taskbar = glass.iter().find(|piece| piece.0[2] >= 1920.).unwrap().0;
        let panel = glass.iter().find(|piece| piece.0[2] < 1920.).unwrap().0;
        taskbar[1] - (panel[1] + panel[3])
    }

    #[test]
    fn the_taskbars_panels_all_melt_into_it_alike() {
        let (mut host, mut now, app) = windows_shell_with_an_app();
        let clipboard = middle_of(&host, "DP-1", "edit-paste-symbolic");
        click(&mut host, "DP-1", clipboard);
        let from_clipboard = above_taskbar(&mut host, &mut now);
        click(&mut host, "DP-1", clipboard);
        settle(&mut host, &mut now);

        host.shell_action("toggle-start-menu", &[], Some(String::from("DP-1")));
        let from_start = above_taskbar(&mut host, &mut now);
        // Close enough to melt into it whole, and as close whatever button it came from.
        assert!(from_clipboard < 4., "{from_clipboard}");
        assert_eq!(from_start, from_clipboard);
        host.shell_action("toggle-start-menu", &[], Some(String::from("DP-1")));
        settle(&mut host, &mut now);

        // The previews, of an app's windows and of the workspaces, too.
        let workspaces = middle_of(&host, "DP-1", "ph-cards-three-symbolic");
        let from_hover = [workspaces, app].map(|button| {
            host.pointer_at(Some(("DP-1", button)));
            let away = above_taskbar(&mut host, &mut now);
            host.pointer_at(None);
            settle(&mut host, &mut now);
            away
        });
        assert_eq!(from_hover, [from_clipboard; 2]);
        super::super::modules::TEST_ENTRY.with(|entry| *entry.borrow_mut() = None);
    }

    #[test]
    fn a_panel_opened_and_closed_over_and_over_stays_where_it_comes_out() {
        let (mut host, mut now, _) = windows_shell_with_an_app();
        let toggle = |host: &SceneHost| {
            host.shell_action("toggle-clipboard", &[], Some(String::from("DP-1")));
        };
        // Where its glass is, frame by frame, apart from the taskbar's.
        let mut seen = Vec::new();
        let mut look = |host: &mut SceneHost, now: &mut Instant| {
            *now += Duration::from_millis(16);
            host.advance(*now, Some(1.));
            let scene = host.shell.as_ref().unwrap().scene.borrow();
            let items: Vec<Item> = Band::ALL
                .into_iter()
                .flat_map(|band| scene.draw("DP-1", band))
                .collect();
            seen.extend(glass_of(&items).into_iter().filter(|p| p.0[2] < 1920.));
        };
        // Pressed again and again, some presses within a frame, some a frame apart.
        for i in 0..12 {
            toggle(&host);
            if i % 3 == 0 {
                toggle(&host);
            }
            look(&mut host, &mut now);
        }
        for _ in 0..60 {
            look(&mut host, &mut now);
        }
        // It comes out over the clipboard's button, at the right of the taskbar.
        let astray: Vec<_> = seen
            .iter()
            .filter(|([x, _, w, _], _, _)| x + w < 1700.)
            .collect();
        assert!(astray.is_empty(), "glass away from the button: {astray:?}");
        // Closed, every one of them goes in the end; open, one is left.
        settle(&mut host, &mut now);
        let scene = host.shell.as_ref().unwrap().scene.borrow();
        let items: Vec<Item> = Band::ALL
            .into_iter()
            .flat_map(|band| scene.draw("DP-1", band))
            .collect();
        let left: Vec<_> = glass_of(&items)
            .into_iter()
            .filter(|p| p.0[2] < 1920.)
            .collect();
        assert!(left.len() <= 1, "{left:?}");
        drop(scene);
        super::super::modules::TEST_ENTRY.with(|entry| *entry.borrow_mut() = None);
    }

    #[test]
    fn the_start_menu_slides_out_of_the_taskbar_and_back_into_it() {
        let (mut host, mut now, _) = windows_shell_with_an_app();
        // The start menu's glass, frame by frame: its rectangle and how opaque it is.
        let frames = |host: &mut SceneHost, now: &mut Instant| {
            let mut seen = Vec::new();
            for _ in 0..90 {
                *now += Duration::from_millis(16);
                host.advance(*now, Some(1.));
                let scene = host.shell.as_ref().unwrap().scene.borrow();
                let items: Vec<Item> = Band::ALL
                    .into_iter()
                    .flat_map(|band| scene.draw("DP-1", band))
                    .collect();
                let menu = glass_of(&items)
                    .into_iter()
                    .find(|piece| piece.0[2] < 1920.);
                seen.push(menu);
            }
            seen
        };
        let grows = |seen: &[Option<GlassPiece>]| {
            let shown: Vec<GlassPiece> = seen.iter().flatten().copied().collect();
            // At its whole width and full strength, coming out of the taskbar's edge, 2px over
            // it, as if from behind it.
            assert!(
                shown.iter().all(|(_, _, opacity)| *opacity > 0.99),
                "{shown:?}"
            );
            assert!(
                shown.iter().all(|([_, _, w, _], _, _)| *w == 672.),
                "{shown:?}"
            );
            assert!(
                shown
                    .iter()
                    .all(|([_, y, _, h], _, _)| (1032. - (y + h) - 2.).abs() < 0.5),
                "{shown:?}"
            );
            assert!(
                shown
                    .iter()
                    .any(|([_, _, _, h], _, _)| *h > 1. && *h < 300.),
                "it's there at its size at once: {shown:?}"
            );
        };
        host.shell_action("toggle-start-menu", &[], Some(String::from("DP-1")));
        let coming = frames(&mut host, &mut now);
        grows(&coming);
        assert!(coming
            .last()
            .unwrap()
            .is_some_and(|([_, _, _, h], _, _)| h > 500.));

        host.shell_action("toggle-start-menu", &[], Some(String::from("DP-1")));
        let going = frames(&mut host, &mut now);
        grows(&going);
        assert!(going.last().unwrap().is_none(), "{going:?}");

        // What it holds slides up with it, rather than being uncovered where it is.
        // Where the search field's icon is drawn, moved with what holds it.
        let heading = |host: &SceneHost| {
            let scene = host.shell.as_ref().unwrap().scene.borrow();
            let items: Vec<Item> = Band::ALL
                .into_iter()
                .flat_map(|band| scene.draw("DP-1", band))
                .collect();
            items.iter().find_map(|item| match item {
                Item::Image {
                    rect,
                    source: super::super::ImageSource::Icon(name),
                    ..
                } if name == "system-search-symbolic" => Some(rect[1]),
                _ => None,
            })
        };
        host.shell_action("toggle-start-menu", &[], Some(String::from("DP-1")));
        for _ in 0..4 {
            now += Duration::from_millis(16);
            host.advance(now, Some(1.));
        }
        let early = heading(&host).unwrap();
        settle(&mut host, &mut now);
        let settled = heading(&host).unwrap();
        assert!(early > settled + 100., "{early} {settled}");
        super::super::modules::TEST_ENTRY.with(|entry| *entry.borrow_mut() = None);
    }

    #[test]
    fn a_menu_is_over_the_shell_rather_than_in_its_layer() {
        let (mut host, mut now, button) = windows_shell_with_an_app();
        assert!(host.covers_any("DP-1", button));
        host.press(3, 3, SceneHost::at("DP-1", button));
        host.release(3, 3, SceneHost::at("DP-1", button));
        settle(&mut host, &mut now);
        assert!(shows(&host, "DP-1", "Close All Windows"));
        // The taskbar is in the top layer; the menu is in the overlay layer, below other
        // programs' overlays, like a screenshot tool's.
        let scene = host.shell.as_ref().unwrap().scene.borrow();
        let menu = scene.find_in("DP-1", "Close All Windows", Band::OverlayBelow);
        assert!(
            !menu.is_empty(),
            "the menu isn't in the overlay layer: {:?}",
            scene.bands("DP-1")
        );
        drop(scene);
        super::super::modules::TEST_ENTRY.with(|entry| *entry.borrow_mut() = None);
    }

    #[test]
    fn a_taskbar_button_shows_no_previews_while_its_menu_is_open() {
        let (mut host, mut now, button) = windows_shell_with_an_app();
        host.pointer_at(Some(("DP-1", button)));
        settle(&mut host, &mut now);
        assert_eq!(copied(&host), [1, 2]);

        assert!(host.covers_any("DP-1", button));
        host.press(3, 3, SceneHost::at("DP-1", button));
        host.release(3, 3, SceneHost::at("DP-1", button));
        settle(&mut host, &mut now);
        assert!(shows(&host, "DP-1", "Close All Windows"));
        assert_eq!(copied(&host), Vec::<u64>::new());

        // Going to the menu and coming back to the button brings no previews over it.
        host.pointer_at(None);
        settle(&mut host, &mut now);
        host.pointer_at(Some(("DP-1", button)));
        settle(&mut host, &mut now);
        assert!(shows(&host, "DP-1", "Close All Windows"));
        assert_eq!(copied(&host), Vec::<u64>::new());
        super::super::modules::TEST_ENTRY.with(|entry| *entry.borrow_mut() = None);
    }

    #[test]
    fn a_taskbar_button_dragged_past_another_takes_its_place() {
        super::super::modules::TEST_ENTRY
            .with(|entry| *entry.borrow_mut() = Some(String::from("bento:shells/windows")));
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        let window = |id: u32, app: &str| {
            serde_json::json!({
                "id": id, "appId": app, "title": app, "workspace": 7, "output": "DP-1",
                "focused": false, "urgent": false, "floating": false,
            })
        };
        host.set_windows(serde_json::json!([
            window(1, "org.example.A"),
            window(2, "org.example.B")
        ]));
        settle(&mut host, &mut now);
        let buttons = host
            .shell
            .as_ref()
            .unwrap()
            .scene
            .borrow()
            .find("DP-1", "application-x-executable");
        assert_eq!(buttons.len(), 2, "{buttons:?}");
        let first = [buttons[0][0] + 4., buttons[0][1] + 4.];
        let second = [buttons[1][0] + 4., buttons[1][1] + 4.];
        commands(&host);
        host.take_actions();

        // A's button, dragged onto B's, takes its place, and letting go there isn't a click.
        assert!(host.covers_any("DP-1", first));
        host.press(1, 1, SceneHost::at("DP-1", first));
        host.pointer_at(Some(("DP-1", [first[0] + 12., first[1]])));
        host.pointer_at(Some(("DP-1", second)));
        settle(&mut host, &mut now);
        host.release(1, 1, SceneHost::at("DP-1", second));
        settle(&mut host, &mut now);
        assert!(host.take_actions().is_empty(), "the drop was a click");

        // B's is first now.
        click(&mut host, "DP-1", first);
        let ran = host.take_actions();
        assert_eq!(ran[0]["name"], "focus-window");
        assert_eq!(ran[0]["window"], 2);
        super::super::modules::TEST_ENTRY.with(|entry| *entry.borrow_mut() = None);
    }

    /// An app's status icon, as the data program tells the shell of it.
    const TRAY: &str = r#"{"type":"tray","items":[{"id":":1.5/StatusNotifierItem","name":"thing","title":"Thing","icon":"thing-tray-symbolic","menu":false,"attention":false}]}"#;

    #[test]
    fn a_right_click_on_a_tray_icon_opens_the_apps_menu_in_the_shell() {
        super::super::modules::TEST_ENTRY
            .with(|entry| *entry.borrow_mut() = Some(String::from("bento:shells/windows")));
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        // Pinned, so it's on the taskbar.
        let settings = host.settings.clone().unwrap();
        settings
            .borrow_mut()
            .write_file("tray {\n    pinned \"thing\"\n}\n");
        let outputs = [(String::from("DP-1"), (1920., 1080.))];
        host.ensure_loaded(&outputs, None, UNIX, None, serde_json::json!({ "gaps": 8 }));
        host.data_line(TRAY);
        settle(&mut host, &mut now);
        let icon = middle_of(&host, "DP-1", "thing-tray-symbolic");
        commands(&host);

        // bento asks the data program for the app's menu, rather than the app to show its own.
        assert!(host.covers_any("DP-1", icon));
        host.press(3, 3, SceneHost::at("DP-1", icon));
        host.release(3, 3, SceneHost::at("DP-1", icon));
        settle(&mut host, &mut now);
        let asked = commands(&host);
        assert!(
            asked
                .iter()
                .any(|c| c["cmd"] == "tray-menu" && c["id"] == ":1.5/StatusNotifierItem"),
            "{asked:?}"
        );

        // The menu shows in the shell's menu, with the tray's own items, and a pick tells the app.
        host.data_line(
            r#"{"type":"tray-menu","id":":1.5/StatusNotifierItem","menu":[{"id":7,"label":"Open Thing","enabled":true,"toggle":null,"on":false,"separator":false,"children":[]}]}"#,
        );
        settle(&mut host, &mut now);
        assert!(shows(&host, "DP-1", "Open Thing"));
        assert!(shows(&host, "DP-1", "Unpin from Tray"));
        let open = middle_of(&host, "DP-1", "Open Thing");
        click(&mut host, "DP-1", open);
        settle(&mut host, &mut now);
        let picked = commands(&host);
        assert!(
            picked
                .iter()
                .any(|c| c["cmd"] == "tray-menu-click" && c["item"] == 7),
            "{picked:?}"
        );
        super::super::modules::TEST_ENTRY.with(|entry| *entry.borrow_mut() = None);
    }

    #[test]
    fn tray_icons_not_pinned_are_behind_the_chevron_and_pinning_brings_one_out() {
        super::super::modules::TEST_ENTRY
            .with(|entry| *entry.borrow_mut() = Some(String::from("bento:shells/windows")));
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        host.data_line(TRAY);
        settle(&mut host, &mut now);
        assert!(!shows(&host, "DP-1", "thing-tray-symbolic"));

        // The chevron opens the rest of the tray over it, and turns back while it's out.
        let arrow = middle_of(&host, "DP-1", "pan-up-symbolic");
        click(&mut host, "DP-1", arrow);
        settle(&mut host, &mut now);
        let icon = middle_of(&host, "DP-1", "thing-tray-symbolic");
        assert!(
            icon[1] < arrow[1],
            "the rest of the tray is over the chevron"
        );
        assert!(shows(&host, "DP-1", "pan-down-symbolic"));

        // Its menu pins it.
        assert!(host.covers_any("DP-1", icon));
        host.press(3, 3, SceneHost::at("DP-1", icon));
        host.release(3, 3, SceneHost::at("DP-1", icon));
        host.data_line(r#"{"type":"tray-menu","id":":1.5/StatusNotifierItem","menu":[]}"#);
        settle(&mut host, &mut now);
        let pin = middle_of(&host, "DP-1", "Pin to Tray");
        click(&mut host, "DP-1", pin);
        settle(&mut host, &mut now);
        let settings = host.settings.clone().unwrap();
        let pinned = settings.borrow().values("tray")["pinned"].clone();
        assert_eq!(pinned, serde_json::json!(["thing"]));
        // On the taskbar now, and the chevron goes, with nothing behind it.
        assert!(!shows(&host, "DP-1", "pan-up-symbolic"));
        assert!(!shows(&host, "DP-1", "pan-down-symbolic"));
        let on_bar = middle_of(&host, "DP-1", "thing-tray-symbolic");
        assert!((on_bar[1] - arrow[1]).abs() < 1.);
        super::super::modules::TEST_ENTRY.with(|entry| *entry.borrow_mut() = None);
    }

    #[test]
    fn the_rest_of_the_tray_stays_out_while_an_icon_in_it_has_its_menu_open() {
        super::super::modules::TEST_ENTRY
            .with(|entry| *entry.borrow_mut() = Some(String::from("bento:shells/windows")));
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        host.data_line(TRAY);
        settle(&mut host, &mut now);
        let chevron = middle_of(&host, "DP-1", "pan-up-symbolic");
        click(&mut host, "DP-1", chevron);
        settle(&mut host, &mut now);
        let icon = middle_of(&host, "DP-1", "thing-tray-symbolic");

        assert!(host.covers_any("DP-1", icon));
        host.press(3, 3, SceneHost::at("DP-1", icon));
        host.release(3, 3, SceneHost::at("DP-1", icon));
        host.data_line(r#"{"type":"tray-menu","id":":1.5/StatusNotifierItem","menu":[]}"#);
        settle(&mut host, &mut now);
        assert!(shows(&host, "DP-1", "Pin to Tray"));
        assert!(
            shows(&host, "DP-1", "thing-tray-symbolic"),
            "the rest of the tray went"
        );

        // Escape closes the menu alone, and a press elsewhere closes the rest too.
        type_keys(&mut host, &["Escape"]);
        settle(&mut host, &mut now);
        assert!(!shows(&host, "DP-1", "Pin to Tray"));
        assert!(shows(&host, "DP-1", "thing-tray-symbolic"));
        host.press(3, 3, SceneHost::at("DP-1", icon));
        host.release(3, 3, SceneHost::at("DP-1", icon));
        host.data_line(r#"{"type":"tray-menu","id":":1.5/StatusNotifierItem","menu":[]}"#);
        settle(&mut host, &mut now);
        assert!(shows(&host, "DP-1", "Pin to Tray"));
        host.press(1, 1, None);
        host.release(1, 1, None);
        settle(&mut host, &mut now);
        assert!(!shows(&host, "DP-1", "Pin to Tray"));
        assert!(!shows(&host, "DP-1", "thing-tray-symbolic"));
        super::super::modules::TEST_ENTRY.with(|entry| *entry.borrow_mut() = None);
    }

    #[test]
    fn a_scroller_has_a_bar_that_drags_and_pages_what_it_holds() {
        // A shell of a scroller with ten rows of fifty in a hundred.
        let dir = std::env::temp_dir().join(format!("bento-scroller-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("scroller.tsx");
        std::fs::write(
            &file,
            r#"
            import { Surface } from "bento";
            import { For } from "solid-js";
            import { Scroller } from "bento:modules/ui";
            export default () => (
              <Surface output="DP-1" anchor={["top", "left"]}>
                <Scroller style={{ width: 200, height: 100 }}>
                  <For each={[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]}>
                    {(i) => <box style={{ height: 50, flexShrink: 0 }}><text>{`Row ${i}`}</text></box>}
                  </For>
                </Scroller>
              </Surface>
            );
            "#,
        )
        .unwrap();
        let entry = file.to_string_lossy().into_owned();
        super::super::modules::TEST_ENTRY.with(|e| *e.borrow_mut() = Some(entry));
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        let row = |host: &SceneHost, i: u32| middle_of(host, "DP-1", &format!("Row {i}"))[1];
        assert_eq!(row(&host, 0), 25.);

        // The thumb is a fifth of the track, on its right, at the top. Dragged down by forty, it
        // goes forty of the eighty it can, so what it holds goes two hundred of its four.
        host.pointer_at(Some(("DP-1", [100., 50.])));
        settle(&mut host, &mut now);
        let thumb = [196., 10.];
        assert!(host.covers_any("DP-1", thumb));
        host.press(1, 1, SceneHost::at("DP-1", thumb));
        host.pointer_at(Some(("DP-1", [196., 50.])));
        host.release(1, 1, SceneHost::at("DP-1", [196., 50.]));
        settle(&mut host, &mut now);
        assert_eq!(row(&host, 4), 25.);

        // A press in the track under the thumb goes a page down.
        click(&mut host, "DP-1", [196., 95.]);
        settle(&mut host, &mut now);
        assert_eq!(row(&host, 6), 25.);
        std::fs::remove_dir_all(&dir).unwrap();
        super::super::modules::TEST_ENTRY.with(|e| *e.borrow_mut() = None);
    }

    #[test]
    fn a_shell_that_fails_to_load_leaves_the_running_one_its_actions_and_binds() {
        super::super::modules::TEST_ENTRY
            .with(|entry| *entry.borrow_mut() = Some(String::from("bento:shells/windows")));
        let (mut host, _) = loaded(serde_json::json!({ "gaps": 8 }));
        super::super::modules::TEST_ENTRY.with(|entry| *entry.borrow_mut() = None);
        let actions = |host: &SceneHost| -> Vec<String> {
            let info = host.ipc_info(bento_ipc::ShellSource::BuiltIn {
                name: String::from("windows"),
            });
            info.actions.into_iter().map(|a| a.action.name).collect()
        };
        let before = (actions(&host), host.default_binds());
        assert!(before.0.contains(&String::from("toggle-start-menu")));

        // A shell of one's own with a mistake in it.
        let dir = std::env::temp_dir().join(format!("bento-broken-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("mine.tsx");
        std::fs::write(&file, "export default () => <box>;\n").unwrap();
        let outputs = [(String::from("DP-1"), (1920., 1080.))];
        let entry = file.to_string_lossy().into_owned();
        host.ensure_loaded(
            &outputs,
            None,
            &entry,
            None,
            serde_json::json!({ "gaps": 8 }),
        );
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(host.load_error.is_some());
        assert_eq!((actions(&host), host.default_binds()), before);
    }

    #[test]
    fn a_shell_that_failed_to_load_loads_once_a_file_it_imports_is_fixed() {
        let dir = std::env::temp_dir().join(format!("bento-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("main.tsx");
        std::fs::write(
            &main,
            "import Bar from \"./bar\";\nexport default () => <Bar />;\n",
        )
        .unwrap();
        let bar = dir.join("bar.tsx");
        std::fs::write(&bar, "export default () => <box>;\n").unwrap();
        let mut host = SceneHost::default();
        let outputs = [(String::from("DP-1"), (1920., 1080.))];
        let entry = main.to_string_lossy().into_owned();
        let style = serde_json::json!({ "gaps": 8 });
        host.ensure_loaded(&outputs, None, &entry, None, style.clone());
        assert!(host.load_error.is_some());

        std::fs::write(&bar, "export default () => <box />;\n").unwrap();
        let later = SystemTime::now() + std::time::Duration::from_secs(5);
        let opened = std::fs::File::options().write(true).open(&bar).unwrap();
        opened.set_modified(later).unwrap();
        host.ensure_loaded(&outputs, None, &entry, None, style);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(host.load_error, None);
    }

    fn loaded(style: serde_json::Value) -> (SceneHost, Instant) {
        let mut host = SceneHost::default();
        let outputs = [(String::from("DP-1"), (1920., 1080.))];
        host.ensure_loaded(&outputs, None, UNIX, None, style);
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
            let drawn = scene.draw_all("DP-1");
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

        let window = |id: u64, x: f64, active: bool| {
            serde_json::json!({
                "id": id, "workspace": 7, "output": "DP-1", "active": active, "floating": false,
                "rect": { "x": x, "y": 0, "width": 960, "height": 1080 },
            })
        };
        // One on another workspace, and one that isn't anywhere, don't show.
        let mut elsewhere = window(4, 0., true);
        elsewhere["workspace"] = 8.into();
        let mut hidden = window(5, 0., false);
        hidden["rect"] = serde_json::Value::Null;
        host.set_windows(serde_json::json!([
            window(1, 0., true),
            window(2, 960., false),
            window(3, 1920., false),
            elsewhere,
            hidden,
        ]));
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
        host.pointer_moved(SceneHost::at("DP-1", over));
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
        host.shell_action("edit-workspace", &[], Some(String::from("DP-1")));
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
        host.shell_action("edit-workspace", &[], Some(String::from("DP-1")));
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
        host.shell_action("edit-workspace", &[], Some(String::from("DP-1")));
        settle(&mut host, &mut now);
        assert!(!host.wants_keyboard());
    }

    #[test]
    fn the_shell_hears_which_output_is_current() {
        let dir = std::env::temp_dir().join(format!("bento-current-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("main.tsx");
        std::fs::write(
            &main,
            "import { Surface, currentOutput } from \"bento\";\n\
             export default () => (\n\
             <Surface output=\"DP-1\" anchor={[\"top\", \"left\"]}>\n\
             <text>{`on ${currentOutput()}`}</text>\n\
             </Surface>\n);\n",
        )
        .unwrap();
        let mut host = SceneHost::default();
        let outputs = ["DP-1", "DP-2"].map(|name| (String::from(name), (1920., 1080.)));
        let entry = main.to_string_lossy().into_owned();
        host.ensure_loaded(
            &outputs,
            None,
            &entry,
            None,
            serde_json::json!({ "gaps": 8 }),
        );
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(host.load_error, None);
        let mut now = Instant::now();
        for name in ["DP-1", "DP-2"] {
            host.set_current_output(Some(String::from(name)));
            settle(&mut host, &mut now);
            assert!(shows(&host, "DP-1", &format!("on {name}")));
        }
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

        // The devices are behind the volume's arrow, on a page of their own; a click on one
        // makes it the default, and the page's back button goes back.
        assert!(!shows(&host, "DP-1", "Headphones"));
        open_page(&mut host, &mut now, 0);
        let headphones = middle_of(&host, "DP-1", "Headphones");
        click(&mut host, "DP-1", headphones);
        assert_eq!(
            commands(&host)[0],
            serde_json::json!({ "cmd": "device", "kind": "output", "name": "b" })
        );
        let back = middle_of(&host, "DP-1", "go-previous-symbolic");
        click(&mut host, "DP-1", back);
        settle(&mut host, &mut now);
        assert!(!shows(&host, "DP-1", "Headphones"));

        // A press halfway along the brightness slider sets it to a half.
        let sun = middle_of(&host, "DP-1", "display-brightness-symbolic");
        let track = sun[0] - 14.;
        let halfway = [track + 160., sun[1]];
        host.press(1, 1, SceneHost::at("DP-1", halfway));
        // Dragged, it follows the pointer.
        host.pointer_at(Some(("DP-1", [track + 240., halfway[1]])));
        host.release(1, 1, SceneHost::at("DP-1", halfway));
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

        // The dark style's tile turns it off while it's on.
        let dark = middle_of(&host, "DP-1", "Dark Style");
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

    /// Opens the page behind the control center's `index`th arrow: the volume's is the first.
    fn open_page(host: &mut SceneHost, now: &mut Instant, index: usize) {
        let scene = host.shell.as_ref().unwrap().scene.borrow();
        let arrow = scene.find("DP-1", "go-next-symbolic")[index];
        drop(scene);
        click(
            host,
            "DP-1",
            [arrow[0] + arrow[2] / 2., arrow[1] + arrow[3] / 2.],
        );
        settle(host, now);
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
        // The calendar is made as the island opens into it.
        let long = seen(&host, &clock_texts(&host).2.unwrap())[0].0;
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
    fn clock_texts(host: &SceneHost) -> (String, String, Option<String>) {
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
        let long = texts.iter().find(|t| t.contains(", ")).cloned();
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
                .draw_all("DP-1")
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
                .draw_all("DP-1")
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
    fn more_notifications_open_like_a_drawer_and_the_glass_grows_with_them() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        for id in 1..=5 {
            host.data_line(&notification(id, &format!("Message {id}"), false));
        }
        open_control_center(&mut host, &mut now);
        // The newest three, and the rest behind a button.
        assert!(shows(&host, "DP-1", "Message 5") && shows(&host, "DP-1", "Message 3"));
        assert!(!shows(&host, "DP-1", "Message 2"));
        let rest = middle_of(&host, "DP-1", "Show 2 More");
        click(&mut host, "DP-1", rest);
        // The glass grows little by little, rather than jumping to its size.
        let mut heights = Vec::new();
        for _ in 0..20 {
            frame(&mut host, &mut now);
            heights.push(panel_height(&host));
        }
        assert!(
            heights[0] < heights[19] && heights[1] - heights[0] < heights[19] - heights[0],
            "{heights:?}"
        );
        settle(&mut host, &mut now);
        assert!(shows(&host, "DP-1", "Message 1"));
        assert!(!shows(&host, "DP-1", "Show 2 More"));
    }

    #[test]
    fn the_wifi_tile_switches_it_and_its_page_joins_a_network_with_its_password() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        let wifi = |enabled: bool, joining: Option<&str>| {
            serde_json::json!({
                "type": "wifi", "available": true, "enabled": enabled, "joining": joining,
                "networks": [
                    { "ssid": "Home", "strength": 80, "secure": true, "known": true, "active": true },
                    { "ssid": "Cafe", "strength": 40, "secure": true, "known": false, "active": false },
                    { "ssid": "Open", "strength": 30, "secure": false, "known": false, "active": false },
                ],
            })
            .to_string()
        };
        host.data_line(&wifi(true, None));
        open_control_center(&mut host, &mut now);
        // The tile says where it's joined, and a click turns Wi-Fi off.
        assert!(shows(&host, "DP-1", "Home"));
        commands(&host);
        let tile = middle_of(&host, "DP-1", "Wi-Fi");
        click(&mut host, "DP-1", tile);
        assert_eq!(
            commands(&host),
            [serde_json::json!({ "cmd": "wifi-enable", "on": false })]
        );

        // Its arrow opens the networks, and looks for more.
        let scene = host.shell.as_ref().unwrap().scene.borrow();
        let arrows = scene.find("DP-1", "go-next-symbolic");
        let wifi_arrow = arrows
            .iter()
            .find(|arrow| (arrow[1] - tile[1]).abs() < 20.)
            .copied()
            .unwrap();
        drop(scene);
        click(
            &mut host,
            "DP-1",
            [wifi_arrow[0] + 10., wifi_arrow[1] + 10.],
        );
        settle(&mut host, &mut now);
        assert!(commands(&host).contains(&serde_json::json!({ "cmd": "wifi-scan" })));
        assert!(shows(&host, "DP-1", "Cafe") && shows(&host, "DP-1", "Connected"));

        // An open network is joined at once; a new one with a password asks for it first.
        let open = middle_of(&host, "DP-1", "Open");
        click(&mut host, "DP-1", open);
        assert_eq!(
            commands(&host),
            [serde_json::json!({ "cmd": "wifi-join", "ssid": "Open" })]
        );
        let cafe = middle_of(&host, "DP-1", "Cafe");
        click(&mut host, "DP-1", cafe);
        settle(&mut host, &mut now);
        assert!(shows(&host, "DP-1", "Password"));
        // A list that comes while it's typed doesn't take the field away.
        host.data_line(&wifi(true, None));
        type_keys(&mut host, &["p", "w", "Return"]);
        assert_eq!(
            commands(&host),
            [serde_json::json!({ "cmd": "wifi-join", "ssid": "Cafe", "password": "pw" })]
        );

        // A join that fails says so.
        host.data_line(r#"{"type":"wifi-error","ssid":"Cafe","message":"Wrong password."}"#);
        settle(&mut host, &mut now);
        assert!(shows(&host, "DP-1", "Wrong password."));
    }

    #[test]
    fn a_file_search_gives_what_the_last_search_found_and_opens_it() {
        let dir = std::env::temp_dir().join(format!("bento-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("main.tsx");
        std::fs::write(
            &main,
            r#"import { Surface } from "bento";
            import { createEffect, For, Show } from "solid-js";
            import { fileSearch, filesAvailable, openFile } from "bento:modules/files";
            export default () => {
              const search = fileSearch(1);
              let field: any;
              createEffect(() => true, () => field.focus());
              return (
                <Surface output="DP-1" keyboard="exclusive" style={{ flexDirection: "column" }}>
                  <box style={{ width: 200 }}><input ref={field} onInput={search.search} /></box>
                  <Show when={search.searching()}><text>Searching</text></Show>
                  <text>{filesAvailable() ? "Search apps and files" : "Search apps"}</text>
                  <For each={search.files()}>
                    {(file) => <box onClick={() => openFile(file)}><text>{file.name}</text></box>}
                  </For>
                </Surface>
              );
            };"#,
        )
        .unwrap();
        let mut host = SceneHost::default();
        let outputs = [(String::from("DP-1"), (1920., 1080.))];
        let entry = main.to_string_lossy().into_owned();
        let gaps = serde_json::json!({ "gaps": 8 });
        host.ensure_loaded(&outputs, None, &entry, None, gaps);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(host.load_error, None);
        let mut now = Instant::now();
        settle(&mut host, &mut now);
        commands(&host);
        let found = |host: &mut SceneHost, id: &serde_json::Value, names: &[&str]| {
            let files: Vec<_> = names
                .iter()
                .map(|name| {
                    serde_json::json!({
                        "path": format!("/h/{name}"), "name": name, "folder": false,
                        "icon": "text-x-generic",
                    })
                })
                .collect();
            let said = serde_json::json!({
                "type": "files", "id": id, "files": files,
            });
            host.data_line(&said.to_string());
        };

        // It's known whether files can be found before anything's looked for.
        for (line, field) in [
            (None, "Search apps"),
            (Some(true), "Search apps and files"),
            (Some(false), "Search apps"),
        ] {
            if let Some(available) = line {
                let said = serde_json::json!({ "type": "file-search", "available": available });
                host.data_line(&said.to_string());
            }
            settle(&mut host, &mut now);
            assert!(shows(&host, "DP-1", field), "{line:?}");
        }

        // Each search is asked for right away, and one in its place is another.
        type_keys(&mut host, &["a", "b"]);
        settle(&mut host, &mut now);
        assert!(shows(&host, "DP-1", "Searching"));
        let asked = commands(&host);
        let (first, second) = (asked[0]["id"].clone(), asked[1]["id"].clone());
        assert_ne!(first, second);
        assert_eq!(
            asked,
            [
                serde_json::json!({ "cmd": "files-search", "id": first, "query": "a" }),
                serde_json::json!({ "cmd": "files-search", "id": second, "query": "ab" }),
            ]
        );

        // What the first one finds comes too late.
        found(&mut host, &first, &["late"]);
        settle(&mut host, &mut now);
        assert!(!shows(&host, "DP-1", "late") && shows(&host, "DP-1", "Searching"));

        // A data program that started again is asked again, and it finds as many as wanted.
        host.shell
            .as_ref()
            .unwrap()
            .send(&serde_json::json!({ "type": "data-ready" }));
        let asked = commands(&host);
        assert_eq!(asked[0]["query"], "ab");
        found(&mut host, &asked[0]["id"], &["ab.txt", "abc.txt"]);
        settle(&mut host, &mut now);
        assert!(!shows(&host, "DP-1", "Searching"));
        assert!(shows(&host, "DP-1", "ab.txt") && !shows(&host, "DP-1", "abc.txt"));
        let file = middle_of(&host, "DP-1", "ab.txt");
        click(&mut host, "DP-1", file);
        assert_eq!(
            commands(&host),
            [serde_json::json!({ "cmd": "files-open", "path": "/h/ab.txt", "token": true })]
        );

        // Looking for nothing ends the search that's on its way, and what was found goes.
        type_keys(&mut host, &["BackSpace"]);
        let asked = commands(&host);
        assert_eq!(asked[0]["query"], "a");
        type_keys(&mut host, &["BackSpace"]);
        settle(&mut host, &mut now);
        assert!(!shows(&host, "DP-1", "ab.txt") && !shows(&host, "DP-1", "Searching"));
        assert_eq!(
            commands(&host),
            [serde_json::json!({ "cmd": "files-stop", "id": asked[0]["id"] })]
        );
    }

    #[test]
    fn tiles_show_only_with_something_behind_them_and_power_mode_has_a_page() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        open_control_center(&mut host, &mut now);
        // Nothing says there's Wi-Fi, Bluetooth or power modes.
        assert!(!shows(&host, "DP-1", "Wi-Fi"));
        assert!(!shows(&host, "DP-1", "Bluetooth"));
        assert!(!shows(&host, "DP-1", "Power Mode"));
        assert!(shows(&host, "DP-1", "Do Not Disturb"));

        host.data_line(
            r#"{"type":"power-profile","available":true,"active":"balanced","profiles":["power-saver","balanced","performance"]}"#,
        );
        host.data_line(r#"{"type":"bluetooth","available":true,"powered":false,"discovering":false,"devices":[]}"#);
        settle(&mut host, &mut now);
        assert!(shows(&host, "DP-1", "Bluetooth"));
        let mode = middle_of(&host, "DP-1", "Power Mode");
        click(&mut host, "DP-1", mode);
        settle(&mut host, &mut now);
        commands(&host);
        let performance = middle_of(&host, "DP-1", "Performance");
        click(&mut host, "DP-1", performance);
        assert_eq!(
            commands(&host),
            [serde_json::json!({ "cmd": "power-profile", "name": "performance" })]
        );
    }

    #[test]
    fn do_not_disturb_keeps_toasts_away_and_clear_all_dismisses_every_one() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        open_control_center(&mut host, &mut now);
        let quiet = middle_of(&host, "DP-1", "Do Not Disturb");
        click(&mut host, "DP-1", quiet);
        type_keys(&mut host, &["Escape"]);
        settle(&mut host, &mut now);
        // The island says it's on.
        assert!(shows(&host, "DP-1", "notifications-disabled-symbolic"));

        host.data_line(&notification(1, "Quietly", false));
        host.data_line(&notification(2, "Also quietly", false));
        settle(&mut host, &mut now);
        assert!(!shows(&host, "DP-1", "Quietly"), "no toast");

        // They're in the control center, and Clear All dismisses them.
        open_control_center(&mut host, &mut now);
        assert!(shows(&host, "DP-1", "Quietly") && shows(&host, "DP-1", "Also quietly"));
        commands(&host);
        let clear = middle_of(&host, "DP-1", "Clear All");
        click(&mut host, "DP-1", clear);
        let sent = commands(&host);
        assert!(sent.contains(&serde_json::json!({ "cmd": "dismiss", "id": 1 })));
        assert!(sent.contains(&serde_json::json!({ "cmd": "dismiss", "id": 2 })));
    }

    #[test]
    fn the_power_menu_asks_twice_to_restart_and_the_settings_button_opens_them() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        open_control_center(&mut host, &mut now);
        let power = middle_of(&host, "DP-1", "system-shutdown-symbolic");
        click(&mut host, "DP-1", power);
        settle(&mut host, &mut now);
        commands(&host);
        let restart = middle_of(&host, "DP-1", "Restart");
        click(&mut host, "DP-1", restart);
        assert!(commands(&host).is_empty(), "the first click only asks");
        // From the next frame, where it's laid out.
        frame(&mut host, &mut now);
        assert!(shows(&host, "DP-1", "Click again to confirm"));
        let restart = middle_of(&host, "DP-1", "Restart");
        click(&mut host, "DP-1", restart);
        assert_eq!(commands(&host), [serde_json::json!({ "cmd": "reboot" })]);
        assert!(!host.wants_keyboard(), "it closes");

        open_control_center(&mut host, &mut now);
        let settings = middle_of(&host, "DP-1", "emblem-system-symbolic");
        click(&mut host, "DP-1", settings);
        assert_eq!(
            host.take_actions()[0]["args"],
            serde_json::json!(["bento-settings"])
        );
    }

    #[test]
    fn whats_playing_shows_with_its_controls() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        host.data_line(
            r#"{"type":"media","players":[{"id":"org.mpris.MediaPlayer2.music","name":"Music","icon":null,"playing":true,"title":"A Song","artist":"Someone","art":null,"canNext":true,"canPrevious":false,"canPlay":true}]}"#,
        );
        open_control_center(&mut host, &mut now);
        assert!(shows(&host, "DP-1", "A Song") && shows(&host, "DP-1", "Someone"));
        commands(&host);
        let pause = middle_of(&host, "DP-1", "media-playback-pause-symbolic");
        click(&mut host, "DP-1", pause);
        let next = middle_of(&host, "DP-1", "media-skip-forward-symbolic");
        click(&mut host, "DP-1", next);
        let previous = middle_of(&host, "DP-1", "media-skip-backward-symbolic");
        click(&mut host, "DP-1", previous);
        let player = "org.mpris.MediaPlayer2.music";
        assert_eq!(
            commands(&host),
            [
                serde_json::json!({ "cmd": "media", "player": player, "action": "PlayPause" }),
                serde_json::json!({ "cmd": "media", "player": player, "action": "Next" }),
            ],
            "a player that can't go back isn't asked to"
        );
    }

    /// A notification as the data program says it.
    fn notification(id: u32, summary: &str, transient: bool) -> String {
        serde_json::json!({
            "type": "notification", "event": "shown", "id": id, "app": "Chat",
            "summary": summary, "body": "", "actions": [], "urgency": 1, "timeout": -1,
            "image": null, "appIcon": null, "transient": transient,
        })
        .to_string()
    }

    /// How tall the control center's glass is.
    fn panel_height(host: &SceneHost) -> f32 {
        let scene = host.shell.as_ref().unwrap().scene.borrow();
        scene
            .draw_all("DP-1")
            .into_iter()
            .filter_map(|item| match item {
                Item::Shape {
                    rect,
                    material: Material::Glass,
                    ..
                } if rect[0] > 960. => Some(rect[3]),
                _ => None,
            })
            .fold(0., f32::max)
    }

    #[test]
    fn pinned_outputs_get_a_switcher_that_goes_around_them() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        host.data_line(&DEVICES.replace('\n', " "));
        settle(&mut host, &mut now);
        // Without pinned outputs there's no switcher, and its shortcut says what to do.
        assert!(!shows(&host, "DP-1", "audio-speakers-symbolic"));
        commands(&host);
        host.shell_action("switch-audio-output", &[], None);
        let sent = commands(&host);
        assert_eq!(sent[0]["cmd"], "notify");
        assert!(sent[0]["body"]
            .as_str()
            .unwrap()
            .contains("No outputs are pinned"));

        // Both outputs pinned, on the control center's sound page.
        open_control_center(&mut host, &mut now);
        open_page(&mut host, &mut now, 0);
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
        host.shell_action("switch-audio-output", &[], None);
        assert_eq!(commands(&host)[0]["name"], "a");
    }

    #[test]
    fn the_screenshot_button_runs_its_command_and_shows_its_notification_after() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        let settings = host.settings.clone().unwrap();
        settings.borrow_mut().write_file(
            "control-center {\n    screenshot-command \"grim\"\n    screenshot-notification \"Shot\"\n    \
             screenshot-notification-body \"Saved.\"\n}\n",
        );
        let outputs = [(String::from("DP-1"), (1920., 1080.))];
        host.ensure_loaded(&outputs, None, UNIX, None, serde_json::json!({ "gaps": 8 }));
        open_control_center(&mut host, &mut now);
        commands(&host);
        let button = middle_of(&host, "DP-1", "camera-photo-symbolic");
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
        let record = middle_of(&host, "DP-1", "media-record-symbolic");
        click(&mut host, "DP-1", record);
        let actions = host.take_actions();
        assert_eq!(actions[0]["name"], "spawn");
        assert_eq!(
            actions[0]["args"],
            serde_json::json!(["bento-settings", "shell"])
        );
    }

    #[test]
    fn without_a_command_the_screenshot_button_takes_bentos_own() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        open_control_center(&mut host, &mut now);
        let button = middle_of(&host, "DP-1", "camera-photo-symbolic");
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
        host.ensure_loaded(&outputs, None, UNIX, None, style);
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

        // A click on one that isn't for anything sends it away.
        let at = middle_of(&host, "DP-1", "New message");
        commands(&host);
        click(&mut host, "DP-1", at);
        let sent = commands(&host);
        assert_eq!(sent, [serde_json::json!({ "cmd": "dismiss", "id": 5 })]);
        host.data_line(r#"{"type":"notification","event":"closed","id":5}"#);
        settle(&mut host, &mut now);
        assert!(!shows(&host, "DP-1", "New message"));

        // Once its time is up, the toast goes and the notification stays, in the control
        // center.
        host.data_line(&toast.replace("60000", "1000").replace(":5,", ":6,"));
        settle(&mut host, &mut now);
        assert!(!shows(&host, "DP-1", "New message"));
        assert!(commands(&host).iter().all(|c| c["cmd"] != "expire"));
        open_control_center(&mut host, &mut now);
        assert!(shows(&host, "DP-1", "New message"));
        type_keys(&mut host, &["Escape"]);
        settle(&mut host, &mut now);

        // A transient one ends with its toast, and isn't kept.
        host.data_line(&notification(7, "Sound Output Switched", true));
        settle(&mut host, &mut now);
        settle(&mut host, &mut now);
        assert!(commands(&host).contains(&serde_json::json!({ "cmd": "expire", "id": 7 })));
    }

    #[test]
    fn a_search_fields_placeholder_is_on_one_line() {
        let (mut host, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        let one_line = |host: &SceneHost, placeholder: &str, size: f32| {
            let scene = host.shell.as_ref().unwrap().scene.borrow();
            let frame = scene.seen("DP-1", placeholder)[0].0;
            assert!(frame[3] < size * 2., "{placeholder}: {frame:?}");
        };
        host.shell_action("toggle-launcher", &[], Some(String::from("DP-1")));
        settle(&mut host, &mut now);
        one_line(&host, "Search apps", 22.);
        type_keys(&mut host, &["Escape"]);
        settle(&mut host, &mut now);

        host.shell_action("toggle-clipboard", &[], Some(String::from("DP-1")));
        settle(&mut host, &mut now);
        one_line(&host, "Search the clipboard", 15.);
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
        let invoke =
            |key: &str| serde_json::json!({ "cmd": "invoke", "id": 7, "key": key, "token": true });
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
        host.shell_action("toggle-clipboard", &[], Some(String::from("DP-1")));
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
                .replace(
                    r#""active":true,"volume":0.4"#,
                    r#""active":false,"volume":0.4"#,
                )
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
        host.ensure_loaded(&outputs, None, UNIX, None, serde_json::json!({ "gaps": 8 }));
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
            let items = host.shell.as_ref().unwrap().scene.borrow().draw_all("DP-1");
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
            let e = timed(&mut || {
                host.shell_action("toggle-launcher", &[], Some(String::from("DP-1")))
            });
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
        let window = |id: u64, rect: [u32; 4], active: bool| {
            serde_json::json!({
                "id": id, "workspace": 7, "output": "DP-1", "active": active, "floating": false,
                "rect": { "x": f64::from(rect[0]) - 960., "y": rect[1], "width": rect[2], "height": rect[3] },
            })
        };
        host.set_windows(serde_json::json!([
            window(1, [0, 0, 960, 1080], false),
            window(2, [960, 0, 960, 1080], true),
            window(3, [1920, 0, 960, 540], false),
            window(4, [1920, 540, 960, 540], false),
        ]));
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
            let items = host.shell.as_ref().unwrap().scene.borrow().draw_all("DP-1");
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

        host.data_line(
            r#"{"type":"wifi","available":true,"enabled":true,"joining":null,"networks":[{"ssid":"Home","strength":80,"secure":true,"known":true,"active":true},{"ssid":"Cafe Corner","strength":55,"secure":true,"known":false,"active":false},{"ssid":"Library Guest","strength":20,"secure":false,"known":false,"active":false}]}"#,
        );
        host.data_line(
            r#"{"type":"bluetooth","available":true,"powered":true,"discovering":false,"devices":[{"path":"/a","name":"Headphones","icon":"audio-headphones-symbolic","paired":true,"connected":true,"connecting":false}]}"#,
        );
        host.data_line(
            r#"{"type":"power-profile","available":true,"active":"performance","profiles":["power-saver","balanced","performance"]}"#,
        );
        host.data_line(
            r#"{"type":"media","players":[{"id":"org.mpris.MediaPlayer2.music","name":"Music","icon":null,"playing":true,"title":"A Song With a Long Name","artist":"Someone","art":null,"canNext":true,"canPrevious":true,"canPlay":true}]}"#,
        );
        host.data_line(&notification(41, "Build finished", false));
        host.data_line(
            r#"{"type":"notification","event":"shown","id":42,"app":"Chat","summary":"Sam","body":"Lunch at noon? I found a place near the station.","actions":[["default",""],["reply","Reply"]],"urgency":1,"timeout":-1,"image":null,"appIcon":null,"transient":false}"#,
        );
        settle(&mut host, &mut now);
        open_control_center(&mut host, &mut now);
        shot(&host, "control-center", [1480, 0, 440, 1000]);
        open_page(&mut host, &mut now, 0);
        shot(&host, "control-center-sound", [1480, 0, 440, 500]);
        let back = middle_of(&host, "DP-1", "go-previous-symbolic");
        click(&mut host, "DP-1", back);
        settle(&mut host, &mut now);
        let wifi = middle_of(&host, "DP-1", "Wi-Fi");
        let scene = host.shell.as_ref().unwrap().scene.borrow();
        let arrow = scene
            .find("DP-1", "go-next-symbolic")
            .into_iter()
            .find(|arrow| (arrow[1] - wifi[1]).abs() < 20.)
            .unwrap();
        drop(scene);
        click(&mut host, "DP-1", [arrow[0] + 10., arrow[1] + 10.]);
        settle(&mut host, &mut now);
        let cafe = middle_of(&host, "DP-1", "Cafe Corner");
        click(&mut host, "DP-1", cafe);
        settle(&mut host, &mut now);
        shot(&host, "control-center-wifi", [1480, 0, 440, 400]);
        type_keys(&mut host, &["Escape"]);
        settle(&mut host, &mut now);
        open_control_center(&mut host, &mut now);
        let power = middle_of(&host, "DP-1", "system-shutdown-symbolic");
        click(&mut host, "DP-1", power);
        settle(&mut host, &mut now);
        shot(&host, "control-center-power", [1480, 0, 440, 320]);
        type_keys(&mut host, &["Escape"]);
        settle(&mut host, &mut now);
        for id in [41, 42] {
            host.data_line(&format!(
                r#"{{"type":"notification","event":"closed","id":{id}}}"#
            ));
        }
        settle(&mut host, &mut now);

        host.shell_action("edit-workspace", &[], Some(String::from("DP-1")));
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
        host.shell_action("toggle-clipboard", &[], Some(String::from("DP-1")));
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

        host.data_line(r#"{"type":"file-search","available":true}"#);
        host.shell_action("toggle-launcher", &[], Some(String::from("DP-1")));
        settle(&mut host, &mut now);
        shot(&host, "launcher-empty", [560, 180, 800, 140]);
        commands(&host);
        type_keys(&mut host, &["f", "i"]);
        let asked = commands(&host);
        let files: Vec<_> = [
            ("fireplace.jpg", "~/Pictures", "image-x-generic", false),
            ("fireworks", "~/Pictures/2026", "folder", true),
            ("firewall.nft", "~/Source/router", "text-x-generic", false),
            (
                "Fire safety plan.pdf",
                "~/Documents/Home",
                "x-office-document",
                false,
            ),
            (
                "firework-01.jpg",
                "~/Pictures/2026/fireworks",
                "image-x-generic",
                false,
            ),
            (
                "firework-02.jpg",
                "~/Pictures/2026/fireworks",
                "image-x-generic",
                false,
            ),
        ]
        .iter()
        .map(|(name, location, icon, folder)| {
            serde_json::json!({
                "path": format!("/h/{name}"), "name": name, "location": location,
                "folder": folder, "icon": icon,
            })
        })
        .collect();
        let found = |host: &mut SceneHost, asked: &[serde_json::Value]| {
            let id = &asked.last().unwrap()["id"];
            let said = serde_json::json!({ "type": "files", "id": id, "files": files });
            host.data_line(&said.to_string());
        };
        found(&mut host, &asked);
        settle(&mut host, &mut now);
        shot(&host, "launcher", [560, 180, 800, 700]);
        type_keys(&mut host, &["/", "Down"]);
        settle(&mut host, &mut now);
        shot(&host, "launcher-files", [560, 180, 800, 700]);
        type_keys(&mut host, &["Escape", "Escape", "Escape"]);

        // The start menu, which searches the same way.
        super::super::modules::TEST_ENTRY
            .with(|entry| *entry.borrow_mut() = Some(String::from("bento:shells/windows")));
        let (mut windows, mut now) = loaded(serde_json::json!({ "gaps": 8 }));
        super::super::modules::TEST_ENTRY.with(|entry| *entry.borrow_mut() = None);
        windows.data_line(r#"{"type":"theme","dark":false}"#);
        windows.data_line(r#"{"type":"file-search","available":true}"#);
        windows.shell_action("toggle-start-menu", &[], Some(String::from("DP-1")));
        settle(&mut windows, &mut now);
        commands(&windows);
        type_keys(&mut windows, &["f", "i"]);
        let asked = commands(&windows);
        found(&mut windows, &asked);
        settle(&mut windows, &mut now);
        shot(&windows, "start-menu", [560, 280, 800, 800]);
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
        host.ensure_loaded(&outputs, None, UNIX, None, style);
        host.shell_action("toggle-launcher", &[], Some(String::from("DP-1")));
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
        host.ensure_loaded(&outputs, None, UNIX, None, serde_json::json!({ "gaps": 8 }));
        // The script's timers, like the caret's blink, run on real time, so this one stays close
        // to it and settles the springs without them.
        let mut now = Instant::now();
        host.shell_action("toggle-launcher", &[], Some(String::from("DP-1")));
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
                .draw_all("DP-1")
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
        host.ensure_loaded(&outputs, None, UNIX, None, serde_json::json!({ "gaps": 8 }));
        host.data_line(r#"{"type":"network","connected":true,"icon":"network-wired-symbolic"}"#);
        let mut now = Instant::now();
        settle(&mut host, &mut now);
        let shows_wired =
            |host: &SceneHost| {
                host.shell.as_ref().unwrap().scene.borrow().draw_all("DP-1").iter().any(|item| {
                matches!(item, Item::Image { source: super::super::ImageSource::Icon(name), .. }
                    if name == "network-wired-symbolic")
            })
            };
        assert!(shows_wired(&host));

        // Loaded again, the shell hears the last of it straight away.
        host.loaded_mtimes = None;
        host.ensure_loaded(&outputs, None, UNIX, None, serde_json::json!({ "gaps": 8 }));
        settle(&mut host, &mut now);
        assert!(shows_wired(&host));
    }

    #[test]
    fn a_click_opens_the_calendar_and_escape_or_a_press_elsewhere_closes_it() {
        let mut host = SceneHost::default();
        let outputs = [(String::from("DP-1"), (1920., 1080.))];
        host.ensure_loaded(&outputs, None, UNIX, None, serde_json::json!({ "gaps": 8 }));
        let mut now = Instant::now();
        settle(&mut host, &mut now);

        // The clock's island is a pill: its rounded corner isn't the shell's.
        assert!(host.covers_any("DP-1", [960., 26.]));
        let island = host
            .shell
            .as_ref()
            .unwrap()
            .scene
            .borrow()
            .draw_all("DP-1")
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
        assert!(!host.covers_any("DP-1", [island[0] + 1., island[1] + 1.]));

        let click = |host: &mut SceneHost| {
            let at = SceneHost::at("DP-1", [960., 26.]);
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
