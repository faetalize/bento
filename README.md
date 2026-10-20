> [!NOTE]
> **bento** is a fork of [niri](https://github.com/niri-wm/niri).
> Please don't report bento issues to the niri project.
> bento keeps niri's IPC protocol and `NIRI_SOCKET`, so tools that talk to niri should keep working.
> Config lives in `$XDG_CONFIG_HOME/bento/config.kdl`.

<h1 align="center">bento</h1>
<p align="center">A Wayland compositor with scriptable tiling, based on niri</p>
<p align="center">
    <a href="https://github.com/faetalize/bento/blob/main/LICENSE"><img alt="GitHub License" src="https://img.shields.io/github/license/faetalize/bento"></a>
</p>

## About

bento is a fork of niri but with changes that don't necessarily follow the original author's vision. Please do not open issues on the original repository if you face any issues using bento.

This repo follows a more permissive development stride, and thus may be less stable than niri. Caaaareful. Kay?

## Features

- Everything niri has, except that the tiling layout is a script
- Layouts are JavaScript files that decide where windows go, which window has focus, where the view points and what actions do. Point the config at one, and bento reloads it as you edit it. See [Tiling Layouts](docs/wiki/Configuration:-Tiling-Layouts.md)
- Layouts run in a sandbox, with no access to files or the network, so you can try ones other people share
- Two layouts ship with bento:
  - `bento`, the default: a grid of cells, 12 rows tall, that scrolls sideways. Windows cover rectangles of cells, push each other aside, and slide left with gravity. See [Grid Layout](docs/wiki/Grid-Layout.md)
  - `niri`: niri's scrollable columns
- Support for more experimental Wayland protocols

## Status

bento does not guarantee stability, but does most things expected of a Wayland compositor.

## Contributing

Not accepting contributions until 2027

## License

bento is licensed under GPL-3.0-or-later, the same as niri. See [LICENSE](LICENSE).

## Contact

@faetalize on Discord.

Join the discord server: https://discord.gg/EZFTEvWWxC
