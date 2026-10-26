## Building

bento has no packages, so you need to build it from source.

First, install the dependencies for your distribution.

- Ubuntu 24.04:

    ```sh
    sudo apt-get install -y gcc clang libudev-dev libgbm-dev libxkbcommon-dev libegl1-mesa-dev libwayland-dev libinput-dev libdbus-1-dev libsystemd-dev libseat-dev libpipewire-0.3-dev libpango1.0-dev libdisplay-info-dev
    ```

- Fedora:

    ```sh
    sudo dnf install gcc libudev-devel libgbm-devel libxkbcommon-devel wayland-devel libinput-devel dbus-devel systemd-devel libseat-devel pipewire-devel pango-devel cairo-gobject-devel clang libdisplay-info-devel
    ```

Next, get latest stable Rust: https://rustup.rs/

Then, build bento with `cargo build --release`.

Check Cargo.toml for a list of build features.
For example, you can replace systemd integration with dinit integration using `cargo build --release --no-default-features --features dinit,dbus,xdp-gnome-screencast`.

> [!WARNING]
> Do NOT build with `--all-features`!
>
> Some features are meant only for development use.
> For example, one of the features enables collection of profiling data into a memory buffer that will grow indefinitely until you run out of memory.

### NixOS/Nix

The repository has a flake, inherited from niri, with a devshell that provides the required dependencies.
Run `nix develop`, then build bento with `cargo build --release`.

If you're not on NixOS, you may need [NixGL](https://github.com/nix-community/nixGL) to run the resulting binary:

```sh
nix run --impure github:guibou/nixGL -- ./target/release/bento
```

### Installing

Put the files in the directories indicated in the table below.
These may vary depending on your distribution.

| File | Destination |
| ---- | ----------- |
| `target/release/bento` | `/usr/local/bin/` |
| `resources/bento-session` | `/usr/local/bin/` |
| `resources/bento.desktop`  | `/usr/local/share/wayland-sessions/` |
| `resources/bento-portals.conf` | `/usr/local/share/xdg-desktop-portal/` |
| `resources/bento.service` (systemd) | `/etc/systemd/user/` |
| `resources/bento-shutdown.target` (systemd) | `/etc/systemd/user/` |
| `resources/dinit/bento` (dinit) | `/etc/dinit.d/user/` |
| `resources/dinit/bento.target` (dinit) | `/etc/dinit.d/user/` |

Doing this will make bento appear in GDM and other display managers.

`bento.service` and the dinit service start `bento` by name rather than by full path, so put it in `/usr/local/bin/` or `/usr/bin/` where they'll find it.

## Starting bento

After installing, start bento from your display manager like GDM.
Press <kbd>Super</kbd><kbd>T</kbd> to run a terminal ([Alacritty]) and <kbd>Super</kbd><kbd>D</kbd> to run an application launcher ([fuzzel]).
To exit bento, press <kbd>Super</kbd><kbd>Shift</kbd><kbd>E</kbd>.

If you're not using a display manager, you should run `bento-session` (systemd/dinit) or `bento --session` (others) from a TTY.
The `--session` flag will make bento import its environment variables globally into the system manager and D-Bus, and start its D-Bus services.
The `bento-session` script will additionally start bento as a systemd/dinit service, which starts up a graphical session target required by some services like portals.

> [!WARNING]
> Running bento directly from a TTY means that exiting or crashing bento will put the user straight into an unlocked shell prompt.
> Set up a display manager to avoid this.

You can also run `bento` inside an existing desktop session.
Then it will open as a window, where you can give it a try.
Note that this windowed mode is mainly meant for development, so it is a bit buggy (in particular, there are issues with hotkeys).

Next, see the [list of important software](./Important-Software.md) required for normal desktop use, like a notification daemon and portals.
Also, check the [configuration introduction](./Configuration:-Introduction.md) page to get started configuring bento.
There you can find links to other pages containing thorough documentation and examples for all options.
Finally, the [Xwayland](./Xwayland.md) page explains how to run X11 applications on bento.

### Tiling

In bento, where tiled windows go is up to a layout engine, chosen in the [`tiling {}`](./Configuration:-Tiling-Layouts.md) section of the config.
The default `bento` engine puts windows on a [grid of cells](./Grid-Layout.md) that scrolls sideways.
The `niri` engine gives niri's scrollable columns:

```kdl
tiling {
    layout "niri"
}
```

Each engine gives actions like `focus-column-left` its own meaning, so the same binds do different things depending on the engine.
See [Tiling Layouts](./Configuration:-Tiling-Layouts.md) for more, including how to write your own engine.

If you're coming from niri, bento reads niri's config format, so you can copy your niri config to `~/.config/bento/config.kdl`.
Without a `tiling {}` section you get the `bento` engine, so add the one above to keep niri's columns.

### Desktop environments

Some desktop environments and shells support niri, and can give a more out-of-the-box experience.
bento keeps niri's IPC and sets `XDG_CURRENT_DESKTOP=bento:niri`, so tools made for niri can work with bento, but their instructions are written for niri:

- There are complete desktop shells that support niri and work with bento through its IPC, for example [DankMaterialShell](https://github.com/AvengeMedia/DankMaterialShell) and [Noctalia](https://github.com/noctalia-dev/noctalia-shell).
- [LXQt](https://lxqt-project.org/) officially supports niri, see [their wiki](https://github.com/lxqt/lxqt/wiki/ConfigWaylandSettings#general) for details on setting it up.
- Many [XFCE](https://www.xfce.org/) components work on Wayland, including niri. See [their wiki](https://wiki.xfce.org/releng/wayland_roadmap#component_specific_status) for details.
- You can run a [COSMIC](https://system76.com/cosmic/) session with niri using [cosmic-ext-extra-sessions](https://github.com/Drakulix/cosmic-ext-extra-sessions).

The default bento config runs Waybar, so with a desktop shell you might get two bars on screen.
To fix this, stop Waybar with `pkill waybar` command, then open `~/.config/bento/config.kdl` and delete the `spawn-at-startup "waybar"` line.

### Default applications

If you install bento alongside GNOME or KDE and want to keep their default app associations (e.g. which image viewer or video player to use), symlink their `mimeapps.list` like so:

```
ln -s /usr/share/applications/gnome-mimeapps.list ~/.local/share/applications/bento-mimeapps.list
```

### NVIDIA

The NVIDIA drivers currently have an issue with high VRAM usage due to a heap reuse quirk.
You're recommended to apply a manual fix documented [here](./Nvidia.md) if you run bento on an NVIDIA GPU.

NVIDIA GPUs can have problems running bento (for example, the screen remains black upon starting from a TTY).
Sometimes, the problems can be fixed.
You can try the following:

1. Update NVIDIA drivers. You need a GPU and drivers recent enough to support GBM.
2. Make sure kernel modesetting is enabled. This usually involves adding `nvidia-drm.modeset=1` to the kernel command line. Find and follow a guide for your distribution. Guides from other Wayland compositors can help.

### Asahi, ARM, and other kmsro devices

On some of these systems, bento fails to correctly detect the primary render device.
If you're getting a black screen when starting bento on a TTY, you can try to set the device manually.

First, find which devices you have:

```
$ ls -l /dev/dri/
drwxr-xr-x@       - root 14 мая 07:07 by-path
crw-rw----@   226,0 root 14 мая 07:07 card0
crw-rw----@   226,1 root 14 мая 07:07 card1
crw-rw-rw-@ 226,128 root 14 мая 07:07 renderD128
crw-rw-rw-@ 226,129 root 14 мая 07:07 renderD129
```

You will likely have one `render` device and two `card` devices.

Open the bento config file at `~/.config/bento/config.kdl` and put your `render` device path like this:

```kdl
debug {
    render-drm-device "/dev/dri/renderD128"
}
```

Save, then try to start bento again.
If you still get a black screen, try using each of the `card` devices.

### Nix/NixOS

There's a common problem of mesa drivers going out of sync with bento built through Nix, so make sure your system mesa version matches the mesa version bento was built against.
When this happens, you usually see a black screen when trying to start bento from a TTY.

Also, on Intel graphics, you may need a workaround described [here](https://wiki.nixos.org/wiki/Intel_Graphics).

### Virtual Machines

To run bento in a VM, make sure to enable 3D acceleration.

## Main Default Hotkeys

When running on a TTY, the Mod key is <kbd>Super</kbd>.
When running in a window, the Mod key is <kbd>Alt</kbd>.

The general system is: if a hotkey switches somewhere, then adding <kbd>Ctrl</kbd> will move the focused window there.

The descriptions below are for the default `bento` engine.
With the `niri` engine, the same hotkeys work on columns, like in niri: for example, <kbd>Mod</kbd><kbd>Ctrl</kbd><kbd>H</kbd> moves the focused column to the left.

| Hotkey | Description |
| ------ | ----------- |
| <kbd>Mod</kbd><kbd>Shift</kbd><kbd>/</kbd> | Show a list of important hotkeys |
| <kbd>Mod</kbd><kbd>T</kbd> | Spawn `alacritty` (terminal) |
| <kbd>Mod</kbd><kbd>D</kbd> | Spawn `fuzzel` (application launcher) |
| <kbd>Super</kbd><kbd>Alt</kbd><kbd>L</kbd> | Spawn `swaylock` (screen locker) |
| <kbd>Mod</kbd><kbd>Q</kbd> | Close the focused window |
| <kbd>Mod</kbd><kbd>H</kbd> or <kbd>Mod</kbd><kbd>←</kbd> | Focus the window to the left |
| <kbd>Mod</kbd><kbd>L</kbd> or <kbd>Mod</kbd><kbd>→</kbd> | Focus the window to the right |
| <kbd>Mod</kbd><kbd>J</kbd> or <kbd>Mod</kbd><kbd>↓</kbd> | Focus the window below |
| <kbd>Mod</kbd><kbd>K</kbd> or <kbd>Mod</kbd><kbd>↑</kbd> | Focus the window above |
| <kbd>Mod</kbd><kbd>Ctrl</kbd><kbd>H</kbd> or <kbd>Mod</kbd><kbd>Ctrl</kbd><kbd>←</kbd> | Move the focused window one cell to the left |
| <kbd>Mod</kbd><kbd>Ctrl</kbd><kbd>L</kbd> or <kbd>Mod</kbd><kbd>Ctrl</kbd><kbd>→</kbd> | Move the focused window one cell to the right |
| <kbd>Mod</kbd><kbd>Ctrl</kbd><kbd>J</kbd> or <kbd>Mod</kbd><kbd>Ctrl</kbd><kbd>↓</kbd> | Move the focused window one cell down |
| <kbd>Mod</kbd><kbd>Ctrl</kbd><kbd>K</kbd> or <kbd>Mod</kbd><kbd>Ctrl</kbd><kbd>↑</kbd> | Move the focused window one cell up |
| <kbd>Mod</kbd><kbd>Shift</kbd><kbd>H</kbd><kbd>J</kbd><kbd>K</kbd><kbd>L</kbd> or <kbd>Mod</kbd><kbd>Shift</kbd><kbd>←</kbd><kbd>↓</kbd><kbd>↑</kbd><kbd>→</kbd> | Focus the monitor to the side |
| <kbd>Mod</kbd><kbd>Ctrl</kbd><kbd>Shift</kbd><kbd>H</kbd><kbd>J</kbd><kbd>K</kbd><kbd>L</kbd> or <kbd>Mod</kbd><kbd>Ctrl</kbd><kbd>Shift</kbd><kbd>←</kbd><kbd>↓</kbd><kbd>↑</kbd><kbd>→</kbd> | Move the focused window to the monitor to the side |
| <kbd>Mod</kbd><kbd>U</kbd> or <kbd>Mod</kbd><kbd>PageDown</kbd> | Switch to the workspace below |
| <kbd>Mod</kbd><kbd>I</kbd> or <kbd>Mod</kbd><kbd>PageUp</kbd> | Switch to the workspace above |
| <kbd>Mod</kbd><kbd>Ctrl</kbd><kbd>U</kbd> or <kbd>Mod</kbd><kbd>Ctrl</kbd><kbd>PageDown</kbd> | Move the focused window to the workspace below |
| <kbd>Mod</kbd><kbd>Ctrl</kbd><kbd>I</kbd> or <kbd>Mod</kbd><kbd>Ctrl</kbd><kbd>PageUp</kbd> | Move the focused window to the workspace above |
| <kbd>Mod</kbd><kbd>Shift</kbd><kbd>U</kbd> or <kbd>Mod</kbd><kbd>Shift</kbd><kbd>PageDown</kbd> | Move the focused workspace down |
| <kbd>Mod</kbd><kbd>Shift</kbd><kbd>I</kbd> or <kbd>Mod</kbd><kbd>Shift</kbd><kbd>PageUp</kbd> | Move the focused workspace up |
| <kbd>Mod</kbd><kbd>[</kbd> | Stack the focused window with the window to the left, or take it out of its stack to the left |
| <kbd>Mod</kbd><kbd>]</kbd> | Stack the focused window with the window to the right, or take it out of its stack to the right |
| <kbd>Mod</kbd><kbd>R</kbd> and <kbd>Mod</kbd><kbd>Shift</kbd><kbd>R</kbd> | Toggle between preset widths forward and back |
| <kbd>Mod</kbd><kbd>M</kbd> | Maximize window |
| <kbd>Mod</kbd><kbd>C</kbd> | Center the focused window within view |
| <kbd>Mod</kbd><kbd>-</kbd> | Decrease window width by 10% |
| <kbd>Mod</kbd><kbd>=</kbd> | Increase window width by 10% |
| <kbd>Mod</kbd><kbd>Shift</kbd><kbd>-</kbd> | Decrease window height by 10% |
| <kbd>Mod</kbd><kbd>Shift</kbd><kbd>=</kbd> | Increase window height by 10% |
| <kbd>Mod</kbd><kbd>Ctrl</kbd><kbd>R</kbd> | Reset the focused window to full height |
| <kbd>Mod</kbd><kbd>Shift</kbd><kbd>F</kbd> | Toggle full-screen on the focused window |
| <kbd>Mod</kbd><kbd>V</kbd> | Move the focused window between the floating and the tiling layout |
| <kbd>Mod</kbd><kbd>Shift</kbd><kbd>V</kbd> | Switch focus between the floating and the tiling layout |
| <kbd>PrtSc</kbd> | Take an area screenshot. Select the area to screenshot with mouse, then press Space to save the screenshot, or Escape to cancel |
| <kbd>Alt</kbd><kbd>PrtSc</kbd> | Take a screenshot of the focused window to clipboard and to `~/Pictures/Screenshots/` |
| <kbd>Ctrl</kbd><kbd>PrtSc</kbd> | Take a screenshot of the focused monitor to clipboard and to `~/Pictures/Screenshots/` |
| <kbd>Mod</kbd><kbd>Shift</kbd><kbd>E</kbd> or <kbd>Ctrl</kbd><kbd>Alt</kbd><kbd>Delete</kbd> | Exit bento |

[Alacritty]: https://github.com/alacritty/alacritty
[fuzzel]: https://codeberg.org/dnkl/fuzzel
