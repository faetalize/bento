//! Types that bento's config and its socket share with the compositor: actions' arguments, output
//! settings, layout engine descriptions.
//!
//! The socket itself is in `bento-ipc`.
#![warn(missing_docs)]

use std::str::FromStr;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Color picked from the screen.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct PickedColor {
    /// Color values as red, green, blue, each ranging from 0.0 to 1.0.
    pub rgb: [f64; 3],
}

/// Actions that niri can perform.
// Variants in this enum should match the spelling of the ones in niri-config. Most, but not all,
// variants from niri-config should be present here.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[cfg_attr(feature = "clap", derive(clap::Parser))]
#[cfg_attr(feature = "clap", command(subcommand_value_name = "ACTION"))]
#[cfg_attr(feature = "clap", command(subcommand_help_heading = "Actions"))]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub enum Action {
    /// Exit niri.
    Quit {
        /// Skip the "Press Enter to confirm" prompt.
        #[serde(default)]
        #[cfg_attr(feature = "clap", arg(short, long))]
        skip_confirmation: bool,
    },
    /// Change the virtual terminal.
    ChangeVt {
        /// Virtual terminal to change to.
        #[cfg_attr(feature = "clap", arg(required = true))]
        vt: u8,
    },
    /// Power off all monitors via DPMS.
    PowerOffMonitors {},
    /// Power on all monitors via DPMS.
    PowerOnMonitors {},
    /// Spawn a command.
    Spawn {
        /// Command to spawn.
        #[cfg_attr(feature = "clap", arg(last = true, required = true))]
        command: Vec<String>,
    },
    /// Spawn a command through the shell.
    SpawnSh {
        /// Command to run.
        #[cfg_attr(feature = "clap", arg(last = true, required = true))]
        command: String,
    },
    /// Do a screen transition.
    DoScreenTransition {
        /// Delay in milliseconds for the screen to freeze before starting the transition.
        #[cfg_attr(feature = "clap", arg(short, long))]
        delay_ms: Option<u16>,
    },
    /// Open the screenshot UI.
    Screenshot {
        ///  Whether to show the mouse pointer by default in the screenshot UI.
        #[serde(default = "default_true")]
        #[cfg_attr(feature = "clap", arg(short = 'p', long, action = clap::ArgAction::Set, default_value_t = true))]
        show_pointer: bool,

        /// Path to save the screenshot to.
        ///
        /// The path must be absolute, otherwise an error is returned.
        ///
        /// If `None`, the screenshot is saved according to the `screenshot-path` config setting.
        #[cfg_attr(feature = "clap", arg(long, action = clap::ArgAction::Set))]
        path: Option<String>,
    },
    /// Screenshot the focused screen.
    ScreenshotScreen {
        /// Write the screenshot to disk in addition to putting it in your clipboard.
        ///
        /// The screenshot is saved according to the `screenshot-path` config setting.
        #[serde(default = "default_true")]
        #[cfg_attr(feature = "clap", arg(short = 'd', long, action = clap::ArgAction::Set, default_value_t = true))]
        write_to_disk: bool,

        /// Whether to include the mouse pointer in the screenshot.
        #[serde(default = "default_true")]
        #[cfg_attr(feature = "clap", arg(short = 'p', long, action = clap::ArgAction::Set, default_value_t = true))]
        show_pointer: bool,

        /// Path to save the screenshot to.
        ///
        /// The path must be absolute, otherwise an error is returned.
        ///
        /// If `None`, the screenshot is saved according to the `screenshot-path` config setting.
        #[cfg_attr(feature = "clap", arg(long, action = clap::ArgAction::Set))]
        path: Option<String>,
    },
    /// Screenshot a window.
    #[cfg_attr(feature = "clap", clap(about = "Screenshot the focused window"))]
    ScreenshotWindow {
        /// Id of the window to screenshot.
        ///
        /// If `None`, uses the focused window.
        #[cfg_attr(feature = "clap", arg(long))]
        id: Option<u64>,
        /// Write the screenshot to disk in addition to putting it in your clipboard.
        ///
        /// The screenshot is saved according to the `screenshot-path` config setting.
        #[serde(default = "default_true")]
        #[cfg_attr(feature = "clap", arg(short = 'd', long, action = clap::ArgAction::Set, default_value_t = true))]
        write_to_disk: bool,

        /// Whether to include the mouse pointer in the screenshot.
        ///
        /// The pointer will be included only if the window is currently receiving pointer input
        /// (usually this means the pointer is on top of the window).
        #[serde(default)]
        #[cfg_attr(feature = "clap", arg(short = 'p', long, action = clap::ArgAction::Set, default_value_t = false))]
        show_pointer: bool,

        /// Path to save the screenshot to.
        ///
        /// The path must be absolute, otherwise an error is returned.
        ///
        /// If `None`, the screenshot is saved according to the `screenshot-path` config setting.
        #[cfg_attr(feature = "clap", arg(long, action = clap::ArgAction::Set))]
        path: Option<String>,
    },
    /// Enable or disable the keyboard shortcuts inhibitor (if any) for the focused surface.
    ToggleKeyboardShortcutsInhibit {},
    /// Close a window.
    #[cfg_attr(feature = "clap", clap(about = "Close the focused window"))]
    CloseWindow {
        /// Id of the window to close.
        ///
        /// If `None`, uses the focused window.
        #[cfg_attr(feature = "clap", arg(long))]
        id: Option<u64>,
    },
    /// Toggle fullscreen on a window.
    #[cfg_attr(
        feature = "clap",
        clap(about = "Toggle fullscreen on the focused window")
    )]
    FullscreenWindow {
        /// Id of the window to toggle fullscreen of.
        ///
        /// If `None`, uses the focused window.
        #[cfg_attr(feature = "clap", arg(long))]
        id: Option<u64>,
    },
    /// Toggle windowed (fake) fullscreen on a window.
    #[cfg_attr(
        feature = "clap",
        clap(about = "Toggle windowed (fake) fullscreen on the focused window")
    )]
    ToggleWindowedFullscreen {
        /// Id of the window to toggle windowed fullscreen of.
        ///
        /// If `None`, uses the focused window.
        #[cfg_attr(feature = "clap", arg(long))]
        id: Option<u64>,
    },
    /// Focus a window by id.
    FocusWindow {
        /// Id of the window to focus.
        #[cfg_attr(feature = "clap", arg(long))]
        id: u64,
    },
    /// Focus the previously focused window.
    FocusWindowPrevious {},
    /// Focus the workspace below.
    FocusWorkspaceDown {},
    /// Focus the workspace above.
    FocusWorkspaceUp {},
    /// Focus a workspace by reference (index or name).
    FocusWorkspace {
        /// Reference (index or name) of the workspace to focus.
        #[cfg_attr(feature = "clap", arg())]
        reference: WorkspaceReferenceArg,
    },
    /// Focus the previous workspace.
    FocusWorkspacePrevious {},
    /// Move the focused window to the workspace below.
    MoveWindowToWorkspaceDown {
        /// Whether the focus should follow the target workspace.
        ///
        /// If `true` (the default), the focus will follow the window to the new workspace. If
        /// `false`, the focus will remain on the original workspace.
        #[serde(default = "default_true")]
        #[cfg_attr(feature = "clap", arg(long, action = clap::ArgAction::Set, default_value_t = true))]
        focus: bool,
    },
    /// Move the focused window to the workspace above.
    MoveWindowToWorkspaceUp {
        /// Whether the focus should follow the target workspace.
        ///
        /// If `true` (the default), the focus will follow the window to the new workspace. If
        /// `false`, the focus will remain on the original workspace.
        #[serde(default = "default_true")]
        #[cfg_attr(feature = "clap", arg(long, action = clap::ArgAction::Set, default_value_t = true))]
        focus: bool,
    },
    /// Move a window to a workspace.
    #[cfg_attr(
        feature = "clap",
        clap(about = "Move the focused window to a workspace by reference (index or name)")
    )]
    MoveWindowToWorkspace {
        /// Id of the window to move.
        ///
        /// If `None`, uses the focused window.
        #[cfg_attr(feature = "clap", arg(long))]
        window_id: Option<u64>,

        /// Reference (index or name) of the workspace to move the window to.
        #[cfg_attr(feature = "clap", arg())]
        reference: WorkspaceReferenceArg,

        /// Whether the focus should follow the moved window.
        ///
        /// If `true` (the default) and the window to move is focused, the focus will follow the
        /// window to the new workspace. If `false`, the focus will remain on the original
        /// workspace.
        #[serde(default = "default_true")]
        #[cfg_attr(feature = "clap", arg(long, action = clap::ArgAction::Set, default_value_t = true))]
        focus: bool,
    },
    /// Move the focused workspace down.
    MoveWorkspaceDown {},
    /// Move the focused workspace up.
    MoveWorkspaceUp {},
    /// Move a workspace to a specific index on its monitor.
    #[cfg_attr(
        feature = "clap",
        clap(about = "Move the focused workspace to a specific index on its monitor")
    )]
    MoveWorkspaceToIndex {
        /// New index for the workspace.
        #[cfg_attr(feature = "clap", arg())]
        index: usize,

        /// Reference (index or name) of the workspace to move.
        ///
        /// If `None`, uses the focused workspace.
        #[cfg_attr(feature = "clap", arg(long))]
        reference: Option<WorkspaceReferenceArg>,
    },
    /// Set the name of a workspace.
    #[cfg_attr(
        feature = "clap",
        clap(about = "Set the name of the focused workspace")
    )]
    SetWorkspaceName {
        /// New name for the workspace.
        #[cfg_attr(feature = "clap", arg())]
        name: String,

        /// Reference (index or name) of the workspace to name.
        ///
        /// If `None`, uses the focused workspace.
        #[cfg_attr(feature = "clap", arg(long))]
        workspace: Option<WorkspaceReferenceArg>,
    },
    /// Unset the name of a workspace.
    #[cfg_attr(
        feature = "clap",
        clap(about = "Unset the name of the focused workspace")
    )]
    UnsetWorkspaceName {
        /// Reference (index or name) of the workspace to unname.
        ///
        /// If `None`, uses the focused workspace.
        #[cfg_attr(feature = "clap", arg())]
        reference: Option<WorkspaceReferenceArg>,
    },
    /// Focus the monitor to the left.
    FocusMonitorLeft {},
    /// Focus the monitor to the right.
    FocusMonitorRight {},
    /// Focus the monitor below.
    FocusMonitorDown {},
    /// Focus the monitor above.
    FocusMonitorUp {},
    /// Focus the previous monitor.
    FocusMonitorPrevious {},
    /// Focus the next monitor.
    FocusMonitorNext {},
    /// Focus a monitor by name.
    FocusMonitor {
        /// Name of the output to focus.
        #[cfg_attr(feature = "clap", arg())]
        output: String,
    },
    /// Move the focused window to the monitor to the left.
    MoveWindowToMonitorLeft {},
    /// Move the focused window to the monitor to the right.
    MoveWindowToMonitorRight {},
    /// Move the focused window to the monitor below.
    MoveWindowToMonitorDown {},
    /// Move the focused window to the monitor above.
    MoveWindowToMonitorUp {},
    /// Move the focused window to the previous monitor.
    MoveWindowToMonitorPrevious {},
    /// Move the focused window to the next monitor.
    MoveWindowToMonitorNext {},
    /// Move a window to a specific monitor.
    #[cfg_attr(
        feature = "clap",
        clap(about = "Move the focused window to a specific monitor")
    )]
    MoveWindowToMonitor {
        /// Id of the window to move.
        ///
        /// If `None`, uses the focused window.
        #[cfg_attr(feature = "clap", arg(long))]
        id: Option<u64>,

        /// The target output name.
        #[cfg_attr(feature = "clap", arg())]
        output: String,
    },
    /// Toggle the maximized-to-edges state of the focused window.
    MaximizeWindowToEdges {
        /// Id of the window to maximize.
        ///
        /// If `None`, uses the focused window.
        #[cfg_attr(feature = "clap", arg(long))]
        id: Option<u64>,
    },
    /// Run an action that the tiling layout provides.
    LayoutAction {
        /// Name of the action.
        name: String,
        /// Arguments for the action.
        ///
        /// Numbers and `true` or `false` go to the layout as numbers and booleans.
        #[cfg_attr(feature = "clap", arg(allow_hyphen_values = true))]
        args: Vec<String>,
    },
    /// Switch between keyboard layouts.
    SwitchLayout {
        /// Layout to switch to.
        #[cfg_attr(feature = "clap", arg())]
        layout: LayoutSwitchTarget,
    },
    /// Show the hotkey overlay.
    ShowHotkeyOverlay {},
    /// Move the focused workspace to the monitor to the left.
    MoveWorkspaceToMonitorLeft {},
    /// Move the focused workspace to the monitor to the right.
    MoveWorkspaceToMonitorRight {},
    /// Move the focused workspace to the monitor below.
    MoveWorkspaceToMonitorDown {},
    /// Move the focused workspace to the monitor above.
    MoveWorkspaceToMonitorUp {},
    /// Move the focused workspace to the previous monitor.
    MoveWorkspaceToMonitorPrevious {},
    /// Move the focused workspace to the next monitor.
    MoveWorkspaceToMonitorNext {},
    /// Move a workspace to a specific monitor.
    #[cfg_attr(
        feature = "clap",
        clap(about = "Move the focused workspace to a specific monitor")
    )]
    MoveWorkspaceToMonitor {
        /// The target output name.
        #[cfg_attr(feature = "clap", arg())]
        output: String,

        // Reference (index or name) of the workspace to move.
        ///
        /// If `None`, uses the focused workspace.
        #[cfg_attr(feature = "clap", arg(long))]
        reference: Option<WorkspaceReferenceArg>,
    },
    /// Toggle a debug tint on windows.
    ToggleDebugTint {},
    /// Toggle visualization of render element opaque regions.
    DebugToggleOpaqueRegions {},
    /// Toggle visualization of output damage.
    DebugToggleDamage {},
    /// Toggle the touchpad on/off at runtime.
    ToggleTouchpad {},
    /// Move the focused window between the floating and the tiling layout.
    ToggleWindowFloating {
        /// Id of the window to move.
        ///
        /// If `None`, uses the focused window.
        #[cfg_attr(feature = "clap", arg(long))]
        id: Option<u64>,
    },
    /// Move the focused window to the floating layout.
    MoveWindowToFloating {
        /// Id of the window to move.
        ///
        /// If `None`, uses the focused window.
        #[cfg_attr(feature = "clap", arg(long))]
        id: Option<u64>,
    },
    /// Move the focused window to the tiling layout.
    MoveWindowToTiling {
        /// Id of the window to move.
        ///
        /// If `None`, uses the focused window.
        #[cfg_attr(feature = "clap", arg(long))]
        id: Option<u64>,
    },
    /// Switches focus to the floating layout.
    FocusFloating {},
    /// Switches focus to the tiling layout.
    FocusTiling {},
    /// Toggles the focus between the floating and the tiling layout.
    SwitchFocusBetweenFloatingAndTiling {},
    /// Move a floating window on screen.
    #[cfg_attr(feature = "clap", clap(about = "Move the floating window on screen"))]
    MoveFloatingWindow {
        /// Id of the window to move.
        ///
        /// If `None`, uses the focused window.
        #[cfg_attr(feature = "clap", arg(long))]
        id: Option<u64>,

        /// How to change the X position.
        #[serde(default = "default_position_change")]
        #[cfg_attr(
            feature = "clap",
            arg(short, long, default_value = "+0", allow_hyphen_values = true)
        )]
        x: PositionChange,

        /// How to change the Y position.
        #[serde(default = "default_position_change")]
        #[cfg_attr(
            feature = "clap",
            arg(short, long, default_value = "+0", allow_hyphen_values = true)
        )]
        y: PositionChange,
    },
    /// Focus the floating window to the left, and the floating layout.
    FocusFloatingLeft {},
    /// Focus the floating window to the right, and the floating layout.
    FocusFloatingRight {},
    /// Focus the floating window above, and the floating layout.
    FocusFloatingUp {},
    /// Focus the floating window below, and the floating layout.
    FocusFloatingDown {},
    /// Move the active floating window to the left.
    MoveFloatingLeft {},
    /// Move the active floating window to the right.
    MoveFloatingRight {},
    /// Move the active floating window up.
    MoveFloatingUp {},
    /// Move the active floating window down.
    MoveFloatingDown {},
    /// Focus the leftmost floating window, and the floating layout.
    FocusFloatingLeftmost {},
    /// Focus the rightmost floating window, and the floating layout.
    FocusFloatingRightmost {},
    /// Focus the topmost floating window, and the floating layout.
    FocusFloatingTopmost {},
    /// Focus the bottommost floating window, and the floating layout.
    FocusFloatingBottommost {},
    /// Center the active floating window on screen.
    CenterFloatingWindow {},
    /// Change the width of the active floating window.
    #[cfg_attr(
        feature = "clap",
        clap(about = "Change the width of the active floating window")
    )]
    SetFloatingWindowWidth {
        /// How to change the width.
        #[cfg_attr(feature = "clap", arg(allow_hyphen_values = true))]
        change: SizeChange,
    },
    /// Change the height of the active floating window.
    #[cfg_attr(
        feature = "clap",
        clap(about = "Change the height of the active floating window")
    )]
    SetFloatingWindowHeight {
        /// How to change the height.
        #[cfg_attr(feature = "clap", arg(allow_hyphen_values = true))]
        change: SizeChange,
    },
    /// Group the focused window with the window to the left.
    GroupWindowLeft {},
    /// Group the focused window with the window to the right.
    GroupWindowRight {},
    /// Group the focused window with the window above.
    GroupWindowUp {},
    /// Group the focused window with the window below.
    GroupWindowDown {},
    /// Take the focused window out of its group.
    UngroupWindow {},
    /// Toggle the opacity of a window.
    #[cfg_attr(
        feature = "clap",
        clap(about = "Toggle the opacity of the focused window")
    )]
    ToggleWindowRuleOpacity {
        /// Id of the window.
        ///
        /// If `None`, uses the focused window.
        #[cfg_attr(feature = "clap", arg(long))]
        id: Option<u64>,
    },
    /// Set the dynamic cast target to a window.
    #[cfg_attr(
        feature = "clap",
        clap(about = "Set the dynamic cast target to the focused window")
    )]
    SetDynamicCastWindow {
        /// Id of the window to target.
        ///
        /// If `None`, uses the focused window.
        #[cfg_attr(feature = "clap", arg(long))]
        id: Option<u64>,
    },
    /// Set the dynamic cast target to a monitor.
    #[cfg_attr(
        feature = "clap",
        clap(about = "Set the dynamic cast target to the focused monitor")
    )]
    SetDynamicCastMonitor {
        /// Name of the output to target.
        ///
        /// If `None`, uses the focused output.
        #[cfg_attr(feature = "clap", arg())]
        output: Option<String>,
    },
    /// Clear the dynamic cast target, making it show nothing.
    ClearDynamicCastTarget {},
    /// Stop a PipeWire screencast.
    ///
    /// wlr-screencopy screencasts cannot currently be stopped via IPC.
    StopCast {
        /// Session ID of the screencast to stop.
        ///
        /// If the session has multiple screencast streams, this will stop all of them.
        #[cfg_attr(feature = "clap", arg(long))]
        session_id: u64,
    },
    /// Toggle (open/close) the Overview.
    ToggleOverview {},
    /// Open the Overview.
    OpenOverview {},
    /// Close the Overview.
    CloseOverview {},
    /// Toggle urgent status of a window.
    ToggleWindowUrgent {
        /// Id of the window to toggle urgent.
        #[cfg_attr(feature = "clap", arg(long))]
        id: u64,
    },
    /// Set urgent status of a window.
    SetWindowUrgent {
        /// Id of the window to set urgent.
        #[cfg_attr(feature = "clap", arg(long))]
        id: u64,
    },
    /// Unset urgent status of a window.
    UnsetWindowUrgent {
        /// Id of the window to unset urgent.
        #[cfg_attr(feature = "clap", arg(long))]
        id: u64,
    },
    /// Reload the config file.
    ///
    /// Can be useful for scripts changing the config file, to avoid waiting the small duration for
    /// the config file watcher to notice the changes.
    LoadConfigFile {
        /// Path of a new config file to load.
        ///
        /// If unset, reloads the current config file.
        #[cfg_attr(feature = "clap", arg(long))]
        path: Option<String>,
    },
}

/// Change in window or column size.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub enum SizeChange {
    /// Set the size in logical pixels.
    SetFixed(i32),
    /// Set the size as a proportion of the working area.
    SetProportion(f64),
    /// Add or subtract to the current size in logical pixels.
    AdjustFixed(i32),
    /// Add or subtract to the current size as a proportion of the working area.
    AdjustProportion(f64),
}

/// Change in floating window position.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub enum PositionChange {
    /// Set the position in logical pixels.
    SetFixed(f64),
    /// Set the position as a proportion of the working area.
    SetProportion(f64),
    /// Add or subtract to the current position in logical pixels.
    AdjustFixed(f64),
    /// Add or subtract to the current position as a proportion of the working area.
    AdjustProportion(f64),
}

/// Workspace reference (id, index or name) to operate on.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub enum WorkspaceReferenceArg {
    /// Id of the workspace.
    Id(u64),
    /// Index of the workspace.
    Index(u8),
    /// Name of the workspace.
    Name(String),
}

/// Layout to switch to.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub enum LayoutSwitchTarget {
    /// The next configured layout.
    Next,
    /// The previous configured layout.
    Prev,
    /// The specific layout by index.
    Index(u8),
}

/// Output actions that niri can perform.
// Variants in this enum should match the spelling of the ones in niri-config. Most thigs from
// niri-config should be present here.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[cfg_attr(feature = "clap", derive(clap::Parser))]
#[cfg_attr(feature = "clap", command(subcommand_value_name = "ACTION"))]
#[cfg_attr(feature = "clap", command(subcommand_help_heading = "Actions"))]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub enum OutputAction {
    /// Turn off the output.
    Off,
    /// Turn on the output.
    On,
    /// Set the output mode.
    Mode {
        /// Mode to set, or "auto" for automatic selection.
        ///
        /// Run `bento msg monitors` to see the available modes.
        #[cfg_attr(feature = "clap", arg())]
        mode: ModeToSet,
    },
    /// Set a custom output mode.
    CustomMode {
        /// Custom mode to set.
        #[cfg_attr(feature = "clap", arg())]
        mode: ConfiguredMode,
    },
    /// Set a custom VESA CVT modeline.
    #[cfg_attr(feature = "clap", arg())]
    Modeline {
        /// The rate at which pixels are drawn in MHz.
        #[cfg_attr(feature = "clap", arg())]
        clock: f64,
        /// Horizontal active pixels.
        #[cfg_attr(feature = "clap", arg())]
        hdisplay: u16,
        /// Horizontal sync pulse start position in pixels.
        #[cfg_attr(feature = "clap", arg())]
        hsync_start: u16,
        /// Horizontal sync pulse end position in pixels.
        #[cfg_attr(feature = "clap", arg())]
        hsync_end: u16,
        /// Total horizontal number of pixels before resetting the horizontal drawing position to
        /// zero.
        #[cfg_attr(feature = "clap", arg())]
        htotal: u16,

        /// Vertical active pixels.
        #[cfg_attr(feature = "clap", arg())]
        vdisplay: u16,
        /// Vertical sync pulse start position in pixels.
        #[cfg_attr(feature = "clap", arg())]
        vsync_start: u16,
        /// Vertical sync pulse end position in pixels.
        #[cfg_attr(feature = "clap", arg())]
        vsync_end: u16,
        /// Total vertical number of pixels before resetting the vertical drawing position to zero.
        #[cfg_attr(feature = "clap", arg())]
        vtotal: u16,
        /// Horizontal sync polarity: "+hsync" or "-hsync".
        #[cfg_attr(feature = "clap", arg(allow_hyphen_values = true))]
        hsync_polarity: HSyncPolarity,
        /// Vertical sync polarity: "+vsync" or "-vsync".
        #[cfg_attr(feature = "clap", arg(allow_hyphen_values = true))]
        vsync_polarity: VSyncPolarity,
    },
    /// Set the output scale.
    Scale {
        /// Scale factor to set, or "auto" for automatic selection.
        #[cfg_attr(feature = "clap", arg())]
        scale: ScaleToSet,
    },
    /// Set the output transform.
    Transform {
        /// Transform to set, counter-clockwise.
        #[cfg_attr(feature = "clap", arg())]
        transform: Transform,
    },
    /// Set the output position.
    Position {
        /// Position to set, or "auto" for automatic selection.
        #[cfg_attr(feature = "clap", command(subcommand))]
        position: PositionToSet,
    },
    /// Set the variable refresh rate mode.
    Vrr {
        /// Variable refresh rate mode to set.
        #[cfg_attr(feature = "clap", command(flatten))]
        vrr: VrrToSet,
    },
    /// Set the maximum bits per channel (bit depth).
    MaxBpc {
        /// Maximum bits per channel to set.
        #[cfg_attr(feature = "clap", arg())]
        max_bpc: MaxBpc,
    },
}

/// Output mode to set.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub enum ModeToSet {
    /// Niri will pick the mode automatically.
    Automatic,
    /// Specific mode.
    Specific(ConfiguredMode),
}

/// Output mode as set in the config file.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct ConfiguredMode {
    /// Width in physical pixels.
    pub width: u16,
    /// Height in physical pixels.
    pub height: u16,
    /// Refresh rate.
    pub refresh: Option<f64>,
}

/// Modeline horizontal syncing polarity.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub enum HSyncPolarity {
    /// Positive polarity.
    PHSync,
    /// Negative polarity.
    NHSync,
}

/// Modeline vertical syncing polarity.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub enum VSyncPolarity {
    /// Positive polarity.
    PVSync,
    /// Negative polarity.
    NVSync,
}

/// Output scale to set.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub enum ScaleToSet {
    /// Niri will pick the scale automatically.
    Automatic,
    /// Specific scale.
    Specific(f64),
}

/// Output position to set.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "clap", derive(clap::Subcommand))]
#[cfg_attr(feature = "clap", command(subcommand_value_name = "POSITION"))]
#[cfg_attr(feature = "clap", command(subcommand_help_heading = "Position Values"))]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub enum PositionToSet {
    /// Position the output automatically.
    #[cfg_attr(feature = "clap", command(name = "auto"))]
    Automatic,
    /// Set a specific position.
    #[cfg_attr(feature = "clap", command(name = "set"))]
    Specific(ConfiguredPosition),
}

/// Output position as set in the config file.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "clap", derive(clap::Args))]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct ConfiguredPosition {
    /// Logical X position.
    pub x: i32,
    /// Logical Y position.
    pub y: i32,
}

/// Output VRR to set.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "clap", derive(clap::Args))]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct VrrToSet {
    /// Whether to enable variable refresh rate.
    #[cfg_attr(
        feature = "clap",
        arg(
            value_name = "ON|OFF",
            action = clap::ArgAction::Set,
            value_parser = clap::builder::BoolishValueParser::new(),
            hide_possible_values = true,
        ),
    )]
    pub vrr: bool,
    /// Only enable when the output shows a window matching the variable-refresh-rate window rule.
    #[serde(default)]
    #[cfg_attr(feature = "clap", arg(long))]
    pub on_demand: bool,
}

/// Connected output.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct Output {
    /// Name of the output.
    pub name: String,
    /// Textual description of the manufacturer.
    pub make: String,
    /// Textual description of the model.
    pub model: String,
    /// Serial of the output, if known.
    pub serial: Option<String>,
    /// Physical width and height of the output in millimeters, if known.
    pub physical_size: Option<(u32, u32)>,
    /// Available modes for the output.
    pub modes: Vec<Mode>,
    /// Index of the current mode in [`Self::modes`].
    ///
    /// `None` if the output is disabled.
    pub current_mode: Option<usize>,
    /// Whether the current_mode is a custom mode.
    pub is_custom_mode: bool,
    /// Whether the output supports variable refresh rate.
    pub vrr_supported: bool,
    /// Whether variable refresh rate is enabled on the output.
    pub vrr_enabled: bool,
    /// Logical output information.
    ///
    /// `None` if the output is not mapped to any logical output (for example, if it is disabled).
    pub logical: Option<LogicalOutput>,
    /// Maximum bits per channel (bit depth), if known.
    pub max_bpc: Option<u8>,
}

/// Output mode.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct Mode {
    /// Width in physical pixels.
    pub width: u16,
    /// Height in physical pixels.
    pub height: u16,
    /// Refresh rate in millihertz.
    pub refresh_rate: u32,
    /// Whether this mode is preferred by the monitor.
    pub is_preferred: bool,
}

/// Logical output in the compositor's coordinate space.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct LogicalOutput {
    /// Logical X position.
    pub x: i32,
    /// Logical Y position.
    pub y: i32,
    /// Width in logical pixels.
    pub width: u32,
    /// Height in logical pixels.
    pub height: u32,
    /// Scale factor.
    pub scale: f64,
    /// Transform.
    pub transform: Transform,
}

/// Output transform, which goes counter-clockwise.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub enum Transform {
    /// Untransformed.
    Normal,
    /// Rotated by 90°.
    #[serde(rename = "90")]
    _90,
    /// Rotated by 180°.
    #[serde(rename = "180")]
    _180,
    /// Rotated by 270°.
    #[serde(rename = "270")]
    _270,
    /// Flipped horizontally.
    Flipped,
    /// Rotated by 90° and flipped horizontally.
    #[cfg_attr(feature = "clap", value(name("flipped-90")))]
    Flipped90,
    /// Flipped vertically.
    #[cfg_attr(feature = "clap", value(name("flipped-180")))]
    Flipped180,
    /// Rotated by 270° and flipped horizontally.
    #[cfg_attr(feature = "clap", value(name("flipped-270")))]
    Flipped270,
}

/// Output maximum bits per channel.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub enum MaxBpc {
    /// 6-bit.
    #[serde(rename = "6")]
    _6 = 6,
    /// 8-bit.
    #[default]
    #[serde(rename = "8")]
    _8 = 8,
    /// 10-bit.
    #[serde(rename = "10")]
    _10 = 10,
    /// 12-bit.
    #[serde(rename = "12")]
    _12 = 12,
    /// 14-bit.
    #[serde(rename = "14")]
    _14 = 14,
    /// 16-bit.
    #[serde(rename = "16")]
    _16 = 16,
}

/// A moment in time.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct Timestamp {
    /// Number of whole seconds.
    pub secs: u64,
    /// Fractional part of the timestamp in nanoseconds (10<sup>-9</sup> seconds).
    pub nanos: u32,
}

/// Position- and size-related properties of a window.
///
/// Optional properties will be unset for some windows, do not rely on them being present. Whether
/// some optional properties are present or absent for certain window types may change across niri
/// releases.
///
/// All sizes and positions are in *logical pixels* unless stated otherwise. Logical sizes may be
/// fractional. For example, at 1.25 monitor scale, a 2-physical-pixel-wide window border is 1.6
/// logical pixels wide.
///
/// This struct contains positions and sizes both for full tiles ([`Self::tile_size`],
/// [`Self::tile_pos_in_workspace_view`]) and the window geometry ([`Self::window_size`],
/// [`Self::window_offset_in_tile`]). For visual displays, use the tile properties, as they
/// correspond to what the user visually considers "window". The window properties on the other
/// hand are mainly useful when you need to know the underlying Wayland window sizes, e.g. for
/// application debugging.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct WindowLayout {
    /// Location of a tiled window within a workspace: (column index, tile index in column).
    ///
    /// The indices are 1-based, i.e. the leftmost column is at index 1 and the topmost tile in a
    /// column is at index 1. This is consistent with [`Action::FocusColumn`] and
    /// [`Action::FocusWindowInColumn`].
    pub pos_in_scrolling_layout: Option<(usize, usize)>,
    /// Size of the tile this window is in, including decorations like borders.
    pub tile_size: (f64, f64),
    /// Size of the window's visual geometry itself.
    ///
    /// Does not include niri decorations like borders.
    ///
    /// Currently, Wayland toplevel windows can only be integer-sized in logical pixels, even
    /// though it doesn't necessarily align to physical pixels.
    pub window_size: (i32, i32),
    /// Tile position within the current view of the workspace.
    ///
    /// This is the same "workspace view" as in gradients' `relative-to` in the niri config.
    pub tile_pos_in_workspace_view: Option<(f64, f64)>,
    /// Location of the window's visual geometry within its tile.
    ///
    /// This includes things like border sizes. For fullscreened fixed-size windows this includes
    /// the distance from the corner of the black backdrop to the corner of the (centered) window
    /// contents.
    pub window_offset_in_tile: (f64, f64),
}

/// A layer-shell layer.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub enum Layer {
    /// The background layer.
    Background,
    /// The bottom layer.
    Bottom,
    /// The top layer.
    Top,
    /// The overlay layer.
    Overlay,
}

/// The layout engine that does the tiling.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct LayoutEngine {
    /// Name of the layout engine, from its `meta`.
    pub name: String,
    /// Description of the layout engine, from its `meta`.
    pub description: Option<String>,
    /// Where the layout engine comes from.
    pub source: LayoutEngineSource,
    /// Whether bento's floating layer exists under this layout engine.
    pub floating: bool,
    /// Options of the layout engine, in the order it declares them.
    pub options: Vec<LayoutEngineOption>,
    /// Properties that the layout engine's window rules can set, in the order it declares them.
    pub window_rules: Vec<LayoutEngineWindowRule>,
    /// Actions that the layout engine has, in the order it declares them. They're bound by name
    /// in the layout engine's own config file, and run with `layout-action`.
    pub actions: Vec<LayoutEngineAction>,
}

/// Where a layout engine comes from.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub enum LayoutEngineSource {
    /// A layout engine that ships with bento.
    BuiltIn {},
    /// A layout engine file.
    File {
        /// Path to the file.
        path: String,
    },
}

/// An option of a layout engine, set in the `options {}` section of its config file.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct LayoutEngineOption {
    /// Name of the option.
    pub name: String,
    /// Type of the option's values.
    pub r#type: LayoutEngineOptionType,
    /// Whether the option is a list of values of its type.
    pub list: bool,
    /// The values a `Choice` option accepts.
    pub choices: Vec<String>,
    /// Value of the option when the config doesn't set it. `None` for a size that is left to the
    /// window.
    pub default: Option<LayoutEngineOptionValue>,
    /// Description of the option.
    pub description: Option<String>,
    /// Current value of the option: the one from the config, or the default. A list option has a
    /// list.
    pub value: Option<LayoutEngineOptionValue>,
}

/// A property that a layout engine's window rules can set, written in the `window-rule` blocks of
/// its config file.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct LayoutEngineWindowRule {
    /// Name of the property.
    pub name: String,
    /// Type of the property's values.
    pub r#type: LayoutEngineOptionType,
    /// Whether the property is a list of values of its type.
    pub list: bool,
    /// The values a `Choice` property accepts.
    pub choices: Vec<String>,
    /// Value of the property for windows that no rule sets it for. `None` for a size that is left
    /// to the window.
    pub default: Option<LayoutEngineOptionValue>,
    /// Description of the property.
    pub description: Option<String>,
}

/// Type of a layout engine option.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub enum LayoutEngineOptionType {
    /// `true` or `false`.
    Boolean,
    /// A number.
    Number,
    /// A string.
    String,
    /// One of the strings in `choices`.
    Choice,
    /// A size, written like `"50%"` for a proportion of the working area or `"800"` for logical
    /// pixels.
    Size,
}

/// Value of a layout engine option.
// Untagged, so that values look the same as in the layout engine's `meta` and the config.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum LayoutEngineOptionValue {
    /// A boolean value.
    Boolean(bool),
    /// A number value.
    Number(f64),
    /// A string value. Choices and sizes are strings too.
    String(String),
    /// The values of a list option.
    List(Vec<LayoutEngineOptionValue>),
}

/// An action that a layout engine handles.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct LayoutEngineAction {
    /// Name of the action.
    pub name: String,
    /// Description of the action.
    pub description: Option<String>,
    /// The action's arguments, in order.
    pub args: Vec<LayoutEngineActionArg>,
}

/// An argument of a layout engine action.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct LayoutEngineActionArg {
    /// Name of the argument, for documentation and settings screens.
    pub name: String,
    /// Type of the argument's values, or `None` if the layout engine doesn't say.
    pub r#type: Option<LayoutEngineActionArgType>,
    /// Value of the argument when it's left out. Arguments with a type and no default must be
    /// given, unless they're `optional`.
    pub default: Option<LayoutEngineOptionValue>,
    /// Whether the argument can be left out even though it has no default.
    #[serde(default)]
    pub optional: bool,
    /// Whether the argument is a named property, like `focus=false`, rather than a positional
    /// argument. Only bento's own actions have these.
    #[serde(default)]
    pub property: bool,
    /// The values a `Choice` argument accepts.
    pub choices: Vec<String>,
    /// Description of the argument.
    pub description: Option<String>,
}

/// Type of a layout engine action argument.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub enum LayoutEngineActionArgType {
    /// `true` or `false`.
    Boolean,
    /// A number.
    Number,
    /// A string.
    String,
    /// One of the strings in `choices`.
    Choice,
    /// A size or a change to one, like `"50%"`, `"+10%"`, `"800"` or `"-100"`. The layout engine
    /// gets it as an object with `op`, `value` and `unit`.
    Size,
    /// A workspace, by its index on the monitor or its name. Only bento's own actions have these.
    Workspace,
    /// A monitor, by its name, like `"DP-2"`. Only bento's own actions have these.
    Monitor,
    /// A command and its arguments, taking the rest of the action's arguments. Only bento's own
    /// actions have these.
    Command,
}

impl From<Duration> for Timestamp {
    fn from(value: Duration) -> Self {
        Timestamp {
            secs: value.as_secs(),
            nanos: value.subsec_nanos(),
        }
    }
}

impl From<Timestamp> for Duration {
    fn from(value: Timestamp) -> Self {
        Duration::new(value.secs, value.nanos)
    }
}

impl FromStr for WorkspaceReferenceArg {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let reference = if let Ok(index) = s.parse::<i32>() {
            if let Ok(idx) = u8::try_from(index) {
                Self::Index(idx)
            } else {
                return Err("workspace index must be between 0 and 255");
            }
        } else {
            Self::Name(s.to_string())
        };

        Ok(reference)
    }
}

impl FromStr for SizeChange {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.split_once('%') {
            Some((value, empty)) => {
                if !empty.is_empty() {
                    return Err("trailing characters after '%' are not allowed");
                }

                match value.bytes().next() {
                    Some(b'-' | b'+') => {
                        let value = value.parse().map_err(|_| "error parsing value")?;
                        Ok(Self::AdjustProportion(value))
                    }
                    Some(_) => {
                        let value = value.parse().map_err(|_| "error parsing value")?;
                        Ok(Self::SetProportion(value))
                    }
                    None => Err("value is missing"),
                }
            }
            None => {
                let value = s;
                match value.bytes().next() {
                    Some(b'-' | b'+') => {
                        let value = value.parse().map_err(|_| "error parsing value")?;
                        Ok(Self::AdjustFixed(value))
                    }
                    Some(_) => {
                        let value = value.parse().map_err(|_| "error parsing value")?;
                        Ok(Self::SetFixed(value))
                    }
                    None => Err("value is missing"),
                }
            }
        }
    }
}

impl FromStr for PositionChange {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.split_once('%') {
            Some((value, empty)) => {
                if !empty.is_empty() {
                    return Err("trailing characters after '%' are not allowed");
                }

                match value.bytes().next() {
                    Some(b'-' | b'+') => {
                        let value = value.parse().map_err(|_| "error parsing value")?;
                        Ok(Self::AdjustProportion(value))
                    }
                    Some(_) => {
                        let value = value.parse().map_err(|_| "error parsing value")?;
                        Ok(Self::SetProportion(value))
                    }
                    None => Err("value is missing"),
                }
            }
            None => {
                let value = s;
                match value.bytes().next() {
                    Some(b'-' | b'+') => {
                        let value = value.parse().map_err(|_| "error parsing value")?;
                        Ok(Self::AdjustFixed(value))
                    }
                    Some(_) => {
                        let value = value.parse().map_err(|_| "error parsing value")?;
                        Ok(Self::SetFixed(value))
                    }
                    None => Err("value is missing"),
                }
            }
        }
    }
}

impl FromStr for LayoutSwitchTarget {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "next" => Ok(Self::Next),
            "prev" => Ok(Self::Prev),
            other => match other.parse() {
                Ok(layout) => Ok(Self::Index(layout)),
                _ => Err(r#"invalid layout action, can be "next", "prev" or a layout index"#),
            },
        }
    }
}

impl FromStr for Transform {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "normal" => Ok(Self::Normal),
            "90" => Ok(Self::_90),
            "180" => Ok(Self::_180),
            "270" => Ok(Self::_270),
            "flipped" => Ok(Self::Flipped),
            "flipped-90" => Ok(Self::Flipped90),
            "flipped-180" => Ok(Self::Flipped180),
            "flipped-270" => Ok(Self::Flipped270),
            _ => Err(concat!(
                r#"invalid transform, can be "90", "180", "270", "#,
                r#""flipped", "flipped-90", "flipped-180" or "flipped-270""#
            )),
        }
    }
}

impl TryFrom<u8> for MaxBpc {
    type Error = &'static str;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            6 => Ok(MaxBpc::_6),
            8 => Ok(MaxBpc::_8),
            10 => Ok(MaxBpc::_10),
            12 => Ok(MaxBpc::_12),
            14 => Ok(MaxBpc::_14),
            16 => Ok(MaxBpc::_16),
            _ => Err("invalid max-bpc, can be 6, 8, 10, 12, 14, 16"),
        }
    }
}

impl FromStr for MaxBpc {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s.parse::<u8>().unwrap_or_default())
    }
}

impl FromStr for Layer {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "background" => Ok(Self::Background),
            "bottom" => Ok(Self::Bottom),
            "top" => Ok(Self::Top),
            "overlay" => Ok(Self::Overlay),
            _ => Err("invalid layer, can be \"background\", \"bottom\", \"top\" or \"overlay\""),
        }
    }
}

impl FromStr for ModeToSet {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("auto") {
            return Ok(Self::Automatic);
        }

        let mode = s.parse()?;
        Ok(Self::Specific(mode))
    }
}

impl FromStr for ConfiguredMode {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let Some((width, rest)) = s.split_once('x') else {
            return Err("no 'x' separator found");
        };

        let (height, refresh) = match rest.split_once('@') {
            Some((height, refresh)) => (height, Some(refresh)),
            None => (rest, None),
        };

        let width = width.parse().map_err(|_| "error parsing width")?;
        let height = height.parse().map_err(|_| "error parsing height")?;
        let refresh = refresh
            .map(str::parse)
            .transpose()
            .map_err(|_| "error parsing refresh rate")?;

        Ok(Self {
            width,
            height,
            refresh,
        })
    }
}

impl FromStr for HSyncPolarity {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "+hsync" => Ok(Self::PHSync),
            "-hsync" => Ok(Self::NHSync),
            _ => Err(r#"invalid horizontal sync polarity, can be "+hsync" or "-hsync"#),
        }
    }
}

impl FromStr for VSyncPolarity {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "+vsync" => Ok(Self::PVSync),
            "-vsync" => Ok(Self::NVSync),
            _ => Err(r#"invalid vertical sync polarity, can be "+vsync" or "-vsync"#),
        }
    }
}

impl FromStr for ScaleToSet {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("auto") {
            return Ok(Self::Automatic);
        }

        let scale = s.parse().map_err(|_| "error parsing scale")?;
        Ok(Self::Specific(scale))
    }
}

macro_rules! ensure {
    ($cond:expr, $fmt:literal $($arg:tt)* ) => {
        if !$cond {
            return Err(format!($fmt $($arg)*));
        }
    };
}

impl OutputAction {
    /// Validates some required constraints on the modeline and custom mode.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            OutputAction::Modeline {
                hdisplay,
                hsync_start,
                hsync_end,
                htotal,
                vdisplay,
                vsync_start,
                vsync_end,
                vtotal,
                ..
            } => {
                ensure!(
                    hdisplay < hsync_start,
                    "hdisplay {} must be < hsync_start {}",
                    hdisplay,
                    hsync_start
                );
                ensure!(
                    hsync_start < hsync_end,
                    "hsync_start {} must be < hsync_end {}",
                    hsync_start,
                    hsync_end
                );
                ensure!(
                    hsync_end < htotal,
                    "hsync_end {} must be < htotal {}",
                    hsync_end,
                    htotal
                );
                ensure!(0 < *htotal, "htotal {} must be > 0", htotal);
                ensure!(
                    vdisplay < vsync_start,
                    "vdisplay {} must be < vsync_start {}",
                    vdisplay,
                    vsync_start
                );
                ensure!(
                    vsync_start < vsync_end,
                    "vsync_start {} must be < vsync_end {}",
                    vsync_start,
                    vsync_end
                );
                ensure!(
                    vsync_end < vtotal,
                    "vsync_end {} must be < vtotal {}",
                    vsync_end,
                    vtotal
                );
                ensure!(0 < *vtotal, "vtotal {} must be > 0", vtotal);
                Ok(())
            }
            OutputAction::CustomMode {
                mode: ConfiguredMode { refresh, .. },
            } => {
                if refresh.is_none() {
                    return Err("refresh rate is required for custom modes".to_string());
                }
                if let Some(refresh) = refresh {
                    if *refresh <= 0. {
                        return Err(format!("custom mode refresh rate {refresh} must be > 0"));
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_position_change() -> PositionChange {
    PositionChange::AdjustFixed(0.)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_size_change() {
        assert_eq!(
            "10".parse::<SizeChange>().unwrap(),
            SizeChange::SetFixed(10),
        );
        assert_eq!(
            "+10".parse::<SizeChange>().unwrap(),
            SizeChange::AdjustFixed(10),
        );
        assert_eq!(
            "-10".parse::<SizeChange>().unwrap(),
            SizeChange::AdjustFixed(-10),
        );
        assert_eq!(
            "10%".parse::<SizeChange>().unwrap(),
            SizeChange::SetProportion(10.),
        );
        assert_eq!(
            "+10%".parse::<SizeChange>().unwrap(),
            SizeChange::AdjustProportion(10.),
        );
        assert_eq!(
            "-10%".parse::<SizeChange>().unwrap(),
            SizeChange::AdjustProportion(-10.),
        );

        assert!("-".parse::<SizeChange>().is_err());
        assert!("10% ".parse::<SizeChange>().is_err());
    }

    #[test]
    fn parse_position_change() {
        assert_eq!(
            "10".parse::<PositionChange>().unwrap(),
            PositionChange::SetFixed(10.),
        );
        assert_eq!(
            "+10".parse::<PositionChange>().unwrap(),
            PositionChange::AdjustFixed(10.),
        );
        assert_eq!(
            "-10".parse::<PositionChange>().unwrap(),
            PositionChange::AdjustFixed(-10.),
        );

        assert_eq!(
            "10%".parse::<PositionChange>().unwrap(),
            PositionChange::SetProportion(10.)
        );
        assert_eq!(
            "+10%".parse::<PositionChange>().unwrap(),
            PositionChange::AdjustProportion(10.)
        );
        assert_eq!(
            "-10%".parse::<PositionChange>().unwrap(),
            PositionChange::AdjustProportion(-10.)
        );
        assert!("-".parse::<PositionChange>().is_err());
        assert!("10% ".parse::<PositionChange>().is_err());
    }
}
