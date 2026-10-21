This page contains various bits of information on how bento fits together with the rest of the system.
First, to build and install bento, see [Getting Started](./Getting-Started.md).

### Compatibility with niri

bento keeps some of niri's names so that tools made for niri keep working:

- bento keeps niri's IPC protocol and the `NIRI_SOCKET` environment variable, so tools that talk to niri over IPC, like Waybar's `niri/workspaces` module, [DankMaterialShell](https://github.com/AvengeMedia/DankMaterialShell) or [Noctalia](https://github.com/noctalia-dev/noctalia-shell), work with bento.
- When running as a session, bento sets `XDG_CURRENT_DESKTOP=bento:niri`, and `bento.desktop` has `DesktopNames=bento;niri`, so tools and configs that look for niri find it too.
- xdg-desktop-portal looks for `bento-portals.conf` first, then `niri-portals.conf`.

### Configuration

bento will load configuration from `$XDG_CONFIG_HOME/bento/config.kdl` or `~/.config/bento/config.kdl`, falling back to `/etc/bento/config.kdl`.
If both of these files are missing, bento will create `$XDG_CONFIG_HOME/bento/config.kdl` with the contents of [the default configuration file](https://github.com/faetalize/bento/blob/main/resources/default-config.kdl), which are embedded into the bento binary at build time.

This means that you can set system-wide defaults by creating `/etc/bento/config.kdl`.
When this file is present, bento *will not* automatically create a config at `~/.config/bento/`, so users will need to create one themselves.

Keep in mind that the default config changes as bento changes, so if you have a custom `/etc/bento/config.kdl`, you likely want to inspect and apply the relevant changes too.

The default configuration locations can be overridden with the `NIRI_CONFIG` environment variable.

<sup>Since: 26.04</sup> You can also change the configuration path at runtime via the bento IPC or using the command `bento msg action load-config-file --path <path-to-config.kdl>`.

<sup>Since: 25.11</sup> You can split the bento config file into multiple files using [`include`](./Configuration:-Include.md).

### Xwayland

Xwayland is required for running X11 apps and games, and also the Orca screen reader.

<sup>Since: 25.08</sup> bento integrates with [xwayland-satellite](https://github.com/Supreeeme/xwayland-satellite) out of the box.
The integration requires xwayland-satellite >= 0.7 available in `$PATH`.
If you had a custom config which manually started `xwayland-satellite` and set `$DISPLAY`, you should remove those customizations for the automatic integration to work.

You can change the path where bento looks for xwayland-satellite using the [`xwayland-satellite` top-level option](./Configuration:-Miscellaneous.md#xwayland-satellite).

### Keyboard layout

<sup>Since: 25.08</sup> By default (unless [manually configured](./Configuration:-Input.md#layout) otherwise), bento reads keyboard layout settings from systemd-localed at `org.freedesktop.locale1` over D-Bus.
If your system installer sets the keyboard layout via systemd-localed, bento should pick it up.

### Autostart

bento works with the normal systemd autostart.
The default [bento.service](https://github.com/faetalize/bento/blob/main/resources/bento.service) brings up `graphical-session.target` as well as `xdg-desktop-autostart.target`.

To make a program run at bento startup without editing the bento config, you can either link its .desktop to `~/.config/autostart/`, or use a .service file with `WantedBy=graphical-session.target`.
See the [example systemd setup](./Example-systemd-Setup.md) page for some examples.

If this is inconvenient, you can also add [`spawn-at-startup`](./Configuration:-Miscellaneous.md#spawn-at-startup) lines in the bento config.

### Screen readers

<sup>Since: 25.08</sup> bento works with the [Orca](https://orca.gnome.org) screen reader.
Please see the [Accessibility](./Accessibility.md) page for details.

### Desktop components

You very likely want to run at least a notification daemon, portals, and an authentication agent.
This is detailed on the [Important Software](./Important-Software.md) page.

On top of that, you may want to preconfigure some desktop shell components to make the experience less barebones.
bento's default config spawns [Waybar](https://github.com/Alexays/Waybar), which is a good starting point, but you may want to consider changing its default configuration to be less of a kitchen sink, and adding the `niri/workspaces` module, which works with bento.
You will probably also want a desktop background tool ([swaybg](https://github.com/swaywm/swaybg) or [awww (which used to be swww)](https://codeberg.org/LGFae/awww/)), and a nicer screen locker (compared to the default `swaylock`), like [hyprlock](https://github.com/hyprwm/hyprlock/).

Alternatively, some desktop environments and shells support niri, and can give a more cohesive experience in one package.
Thanks to the compatibility described above, tools made for niri can work with bento, but their instructions are written for niri:

- There are complete desktop shells that support niri and work with bento through its IPC, for example [DankMaterialShell](https://github.com/AvengeMedia/DankMaterialShell) and [Noctalia](https://github.com/noctalia-dev/noctalia-shell).
- [LXQt](https://lxqt-project.org/) officially supports niri, see [their wiki](https://lxqt-project.org/wiki/Wayland-Session) for details on setting it up.
- Many [XFCE](https://www.xfce.org/) components work on Wayland, including niri. See [their wiki](https://wiki.xfce.org/releng/wayland_roadmap#component_specific_status) for details.
- You can run a [COSMIC](https://system76.com/cosmic/) session with niri using [cosmic-ext-extra-sessions](https://github.com/Drakulix/cosmic-ext-extra-sessions).

### Security model

See the [Security Model](./Security-Model.md) page for an overview of bento's security model.
