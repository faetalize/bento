use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use clap_complete::Shell;
use niri_ipc::OutputAction;

use crate::utils::version;

#[derive(Parser)]
#[command(author, version = version(), about, long_about = None)]
#[command(args_conflicts_with_subcommands = true)]
#[command(subcommand_value_name = "SUBCOMMAND")]
#[command(subcommand_help_heading = "Subcommands")]
pub struct Cli {
    /// Path to config file (default: `$XDG_CONFIG_HOME/bento/config.kdl`).
    ///
    /// This can also be set with the `NIRI_CONFIG` environment variable. If both are set, the
    /// command line argument takes precedence.
    #[arg(short, long)]
    pub config: Option<PathBuf>,
    /// Import environment globally to systemd and D-Bus, run D-Bus services.
    ///
    /// Set this flag in a systemd service started by your display manager, or when running
    /// manually as your main compositor instance. Do not set when running as a nested window, or
    /// on a TTY as your non-main compositor instance, to avoid messing up the global environment.
    #[arg(long)]
    pub session: bool,
    /// Command to run upon compositor startup.
    #[arg(last = true)]
    pub command: Vec<OsString>,

    #[command(subcommand)]
    pub subcommand: Option<Sub>,
}

#[derive(Subcommand)]
pub enum Sub {
    /// Communicate with the running bento instance.
    Msg {
        #[command(subcommand)]
        msg: Msg,
        /// Format output as JSON.
        #[arg(short, long)]
        json: bool,
        /// Print the IPC request as JSON instead of sending it.
        #[arg(long)]
        print_request: bool,
    },
    /// Validate the config file.
    Validate {
        /// Path to config file (default: `$XDG_CONFIG_HOME/bento/config.kdl`).
        ///
        /// This can also be set with the `NIRI_CONFIG` environment variable. If both are set, the
        /// command line argument takes precedence.
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    /// Cause a panic to check if the backtraces are good.
    Panic,
    /// Generate shell completions.
    Completions { shell: CompletionShell },
}

#[derive(Subcommand)]
pub enum Msg {
    /// Print the version of the running bento instance.
    Version,
    /// List connected monitors.
    Monitors,
    /// List workspaces.
    Workspaces,
    /// List open windows.
    Windows,
    /// List groups of windows.
    Groups,
    /// List open layer-shell surfaces.
    Layers,
    /// Print the focused monitor and the focused window.
    Focused,
    /// Get the configured keyboard layouts.
    KeyboardLayouts,
    /// Print the overview state.
    Overview,
    /// List screencasts.
    Casts,
    /// Print the running engine's options, actions and window rules.
    Engine,
    /// Print the state the running engine exposes, as JSON.
    EngineState,
    /// List bento's own actions.
    Actions,
    /// Pick a window with the mouse and print information about it.
    PickWindow,
    /// Pick a color from the screen with the mouse.
    PickColor,
    /// Run an action, one of bento's or the running engine's.
    ///
    /// The action is written as in a bind: `bento msg action focus-workspace 3`. Properties go
    /// before the action's name: `bento msg action -p focus=false move-window-to-workspace 3`.
    /// Everything after the name is an argument. `layout-action NAME ARGS...` runs an engine's
    /// action by its name.
    ///
    /// Run `bento msg actions` and `bento msg engine` to see the actions.
    Action {
        /// Run it on this window instead of the focused one.
        #[arg(long, value_name = "ID")]
        window: Option<u64>,
        /// Run it on this workspace instead of the focused one, by its id.
        #[arg(long, value_name = "ID")]
        workspace: Option<u64>,
        /// A property of the action, like `focus=false`. Can be given several times.
        ///
        /// Screenshot actions also take `path=/absolute/path.png`.
        #[arg(short, long = "property", value_name = "NAME=VALUE")]
        properties: Vec<String>,
        /// The action's name.
        name: String,
        /// The action's arguments.
        #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// Change monitor configuration temporarily.
    ///
    /// The configuration is changed temporarily and not saved into the config file. If the monitor
    /// configuration subsequently changes in the config file, these temporary changes will be
    /// forgotten.
    Output {
        /// Monitor name.
        ///
        /// Run `bento msg monitors` to see the monitor names.
        #[arg()]
        output: String,
        /// Configuration to apply.
        #[command(subcommand)]
        action: OutputAction,
    },
    /// Focus a window.
    FocusWindow {
        /// Id of the window to focus.
        id: u64,
    },
    /// Set whether a window asks for attention, or toggle it.
    WindowUrgent {
        /// Id of the window.
        id: u64,
        /// The new state: `on` or `off`. Toggles it if left out.
        #[arg(value_parser = clap::builder::BoolishValueParser::new())]
        urgent: Option<bool>,
    },
    /// Stop a PipeWire screencast.
    ///
    /// wlr-screencopy screencasts can't be stopped.
    StopCast {
        /// Session ID of the screencast to stop.
        ///
        /// If the session has multiple screencast streams, this will stop all of them.
        session_id: u64,
    },
    /// Reload the config file.
    ///
    /// Can be useful for scripts changing the config file, to avoid waiting the small duration for
    /// the config file watcher to notice the changes.
    LoadConfigFile {
        /// Path of a new config file to load.
        ///
        /// If unset, reloads the current config file.
        path: Option<String>,
    },
    /// Start continuously receiving events from the compositor.
    ///
    /// The first events describe everything that is there right now.
    #[command(visible_alias = "event-stream")]
    Subscribe {
        /// The kinds of events to receive, like `windowAdded`. All of them if left out.
        events: Vec<String>,
    },
    /// Send a raw JSON request to the compositor, reading from stdin.
    RawRequest,
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum CompletionShell {
    Bash,
    Elvish,
    Fish,
    PowerShell,
    Zsh,
    Nushell,
}

impl TryFrom<CompletionShell> for Shell {
    type Error = &'static str;

    fn try_from(shell: CompletionShell) -> Result<Self, Self::Error> {
        match shell {
            CompletionShell::Bash => Ok(Shell::Bash),
            CompletionShell::Elvish => Ok(Shell::Elvish),
            CompletionShell::Fish => Ok(Shell::Fish),
            CompletionShell::PowerShell => Ok(Shell::PowerShell),
            CompletionShell::Zsh => Ok(Shell::Zsh),
            CompletionShell::Nushell => Err("Nushell should be handled separately"),
        }
    }
}
