use crate::appearance::{Color, WorkspaceShadow, WorkspaceShadowPart, DEFAULT_BACKDROP_COLOR};
use crate::utils::{Flag, MergeWith};
use crate::FloatOrInt;

#[derive(knuffel::Decode, Debug, Clone, PartialEq, Eq)]
pub struct SpawnAtStartup {
    #[knuffel(arguments)]
    pub command: Vec<String>,
}

#[derive(knuffel::Decode, Debug, Clone, PartialEq, Eq)]
pub struct SpawnShAtStartup {
    #[knuffel(argument)]
    pub command: String,
}

/// The shell: the program bento starts with a private connection of its own, and starts again
/// when it exits, and what bento's own shell is set to. No arguments means no program.
#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq, Eq)]
pub struct Shell {
    #[knuffel(arguments)]
    pub command: Vec<String>,
    /// Whether bento draws its own shell, the scene: its bar, control center, notifications and
    /// launcher. It's on top of the program, if the config names one too. A flag that can be
    /// written `built-in false`, the way bento Settings turns it off.
    #[knuffel(child)]
    pub built_in: Option<Flag>,
    /// Which shell bento draws: one of its own by name, `"unix"` or `"windows"`, or a file of
    /// one's own, by its path, with the file of its settings and binds next to it:
    /// `script "~/my-shell.tsx" settings="~/.config/bento/shells/my-shell.kdl"`. bento's own
    /// `"unix"` when it's left out.
    #[knuffel(child)]
    pub script: Option<ShellScriptNode>,
}

/// The `script` node of [`Shell`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellScriptNode {
    /// The shell, as written.
    pub name: String,
    /// The file of a shell file's settings and binds, as written. bento's own shells share
    /// `shell.kdl` next to the config instead.
    pub settings: Option<String>,
}

impl<S: knuffel::traits::ErrorSpan> knuffel::Decode<S> for ShellScriptNode {
    fn decode_node(
        node: &knuffel::ast::SpannedNode<S>,
        ctx: &mut knuffel::decode::Context<S>,
    ) -> Result<Self, knuffel::errors::DecodeError<S>> {
        use knuffel::errors::DecodeError;
        let mut arguments = node.arguments.iter();
        let name: String = match arguments.next() {
            Some(value) => knuffel::traits::DecodeScalar::decode(value, ctx)?,
            None => return Err(DecodeError::missing(node, "the shell's name or path")),
        };
        for value in arguments {
            ctx.emit_error(DecodeError::unexpected(
                &value.literal,
                "argument",
                "a shell is one name or path",
            ));
        }
        if let Some(child) = node.children().next() {
            ctx.emit_error(DecodeError::unexpected(
                child,
                "node",
                "a shell has no children",
            ));
        }
        let mut settings = None;
        for (property, value) in &node.properties {
            if &***property != "settings" {
                ctx.emit_error(DecodeError::unexpected(
                    property,
                    "property",
                    format!("unexpected property `{}`", property.escape_default()),
                ));
                continue;
            }
            if matches!(
                ShellScript::from_config(Some(&name)),
                ShellScript::BuiltIn(_)
            ) {
                ctx.emit_error(DecodeError::unexpected(
                    property,
                    "property",
                    "bento's own shells' settings are always in `shell.kdl` next to this config",
                ));
                continue;
            }
            settings = Some(knuffel::traits::DecodeScalar::decode(value, ctx)?);
        }
        Ok(Self { name, settings })
    }
}

impl Shell {
    /// Whether bento draws its own shell.
    pub fn built_in(&self) -> bool {
        self.built_in.is_some_and(|flag| flag.0)
    }

    /// Which shell bento draws.
    pub fn script(&self) -> ShellScript {
        ShellScript::from_config(self.script.as_ref().map(|s| s.name.as_str()))
    }

    /// The file of the shell's settings and binds, with `dir` the folder of the user's config:
    /// bento's own shells share `shell.kdl` there, and a shell file's is the one the config names
    /// next to it, if any, with `~` for the home folder and relative to `dir`.
    pub fn settings_file(&self, dir: &std::path::Path) -> Option<std::path::PathBuf> {
        match self.script() {
            ShellScript::BuiltIn(_) => Some(dir.join("shell.kdl")),
            ShellScript::File(_) => {
                let written = self.script.as_ref()?.settings.as_deref()?;
                let path = std::path::Path::new(written);
                Some(match path.strip_prefix("~") {
                    Ok(rest) => {
                        std::env::home_dir().map_or_else(|| path.to_owned(), |h| h.join(rest))
                    }
                    Err(_) => dir.join(path),
                })
            }
        }
    }
}

/// Which shell bento draws, the way the config names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellScript {
    /// One of bento's own shells, by name.
    BuiltIn(String),
    /// A shell file, by its path as written, which can start with `~`.
    File(std::path::PathBuf),
}

impl ShellScript {
    /// The shell bento draws when the config names none.
    pub const DEFAULT: &str = "unix";

    /// The shell that `script` names: a path when it has a `/` or a script's extension, one of
    /// bento's own otherwise.
    pub fn from_config(script: Option<&str>) -> Self {
        let script = script.map(str::trim).filter(|s| !s.is_empty());
        match script {
            None => Self::BuiltIn(String::from(Self::DEFAULT)),
            Some(script)
                if script.contains('/')
                    || [".tsx", ".ts", ".jsx", ".js"]
                        .iter()
                        .any(|e| script.ends_with(e)) =>
            {
                Self::File(std::path::PathBuf::from(script))
            }
            Some(name) => Self::BuiltIn(String::from(name)),
        }
    }
}

/// Where the user is, for what depends on it, like the weather.
#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct Location {
    /// What the place is called, to show.
    #[knuffel(child, unwrap(argument))]
    pub name: Option<String>,
    #[knuffel(child, unwrap(argument))]
    pub latitude: Option<FloatOrInt<-90, 90>>,
    #[knuffel(child, unwrap(argument))]
    pub longitude: Option<FloatOrInt<-180, 180>>,
}

#[derive(Debug, PartialEq)]
pub struct Cursor {
    pub xcursor_theme: String,
    pub xcursor_size: u8,
    pub hide_when_typing: bool,
    pub hide_after_inactive_ms: Option<u32>,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            xcursor_theme: String::from("default"),
            xcursor_size: 24,
            hide_when_typing: false,
            hide_after_inactive_ms: None,
        }
    }
}

#[derive(knuffel::Decode, Debug, PartialEq)]
pub struct CursorPart {
    #[knuffel(child, unwrap(argument))]
    pub xcursor_theme: Option<String>,
    #[knuffel(child, unwrap(argument))]
    pub xcursor_size: Option<u8>,
    #[knuffel(child)]
    pub hide_when_typing: Option<Flag>,
    #[knuffel(child, unwrap(argument))]
    pub hide_after_inactive_ms: Option<u32>,
}

impl MergeWith<CursorPart> for Cursor {
    fn merge_with(&mut self, part: &CursorPart) {
        merge_clone!((self, part), xcursor_theme, xcursor_size);
        merge!((self, part), hide_when_typing);
        merge_clone_opt!((self, part), hide_after_inactive_ms);
    }
}

#[derive(knuffel::Decode, Debug, Clone, PartialEq)]
pub struct ScreenshotPath(#[knuffel(argument)] pub Option<String>);

impl Default for ScreenshotPath {
    fn default() -> Self {
        Self(Some(String::from(
            "~/Pictures/Screenshots/Screenshot from %Y-%m-%d %H-%M-%S.png",
        )))
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HotkeyOverlay {
    pub skip_at_startup: bool,
    pub hide_not_bound: bool,
}

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HotkeyOverlayPart {
    #[knuffel(child)]
    pub skip_at_startup: Option<Flag>,
    #[knuffel(child)]
    pub hide_not_bound: Option<Flag>,
}

impl MergeWith<HotkeyOverlayPart> for HotkeyOverlay {
    fn merge_with(&mut self, part: &HotkeyOverlayPart) {
        merge!((self, part), skip_at_startup, hide_not_bound);
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConfigNotification {
    pub disable_failed: bool,
}

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConfigNotificationPart {
    #[knuffel(child)]
    pub disable_failed: Option<Flag>,
}

impl MergeWith<ConfigNotificationPart> for ConfigNotification {
    fn merge_with(&mut self, part: &ConfigNotificationPart) {
        merge!((self, part), disable_failed);
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Clipboard {
    pub disable_primary: bool,
}

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ClipboardPart {
    #[knuffel(child)]
    pub disable_primary: Option<Flag>,
}

impl MergeWith<ClipboardPart> for Clipboard {
    fn merge_with(&mut self, part: &ClipboardPart) {
        merge!((self, part), disable_primary);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Overview {
    pub zoom: f64,
    pub backdrop_color: Color,
    pub workspace_shadow: WorkspaceShadow,
}

impl Default for Overview {
    fn default() -> Self {
        Self {
            zoom: 0.5,
            backdrop_color: DEFAULT_BACKDROP_COLOR,
            workspace_shadow: WorkspaceShadow::default(),
        }
    }
}

#[derive(knuffel::Decode, Debug, Clone, Copy, PartialEq)]
pub struct OverviewPart {
    #[knuffel(child, unwrap(argument))]
    pub zoom: Option<FloatOrInt<0, 1>>,
    #[knuffel(child)]
    pub backdrop_color: Option<Color>,
    #[knuffel(child)]
    pub workspace_shadow: Option<WorkspaceShadowPart>,
}

impl MergeWith<OverviewPart> for Overview {
    fn merge_with(&mut self, part: &OverviewPart) {
        merge!((self, part), zoom, workspace_shadow);
        merge_clone!((self, part), backdrop_color);
    }
}

#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq, Eq)]
pub struct Environment(#[knuffel(children)] pub Vec<EnvironmentVariable>);

#[derive(knuffel::Decode, Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentVariable {
    #[knuffel(node_name)]
    pub name: String,
    #[knuffel(argument)]
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XwaylandSatellite {
    pub off: bool,
    pub path: String,
}

impl Default for XwaylandSatellite {
    fn default() -> Self {
        Self {
            off: false,
            path: String::from("xwayland-satellite"),
        }
    }
}

#[derive(knuffel::Decode, Debug, Clone, PartialEq, Eq)]
pub struct XwaylandSatellitePart {
    #[knuffel(child)]
    pub off: bool,
    #[knuffel(child)]
    pub on: bool,
    #[knuffel(child, unwrap(argument))]
    pub path: Option<String>,
}

impl MergeWith<XwaylandSatellitePart> for XwaylandSatellite {
    fn merge_with(&mut self, part: &XwaylandSatellitePart) {
        self.off |= part.off;
        if part.on {
            self.off = false;
        }

        merge_clone!((self, part), path);
    }
}
