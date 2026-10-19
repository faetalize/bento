> [!NOTE]
> **bento** is a fork of [niri](https://github.com/niri-wm/niri).
> Please don't report bento issues to the niri project.
> bento keeps niri's IPC protocol and `NIRI_SOCKET`, so tools that talk to niri should keep working.
> Config lives in `$XDG_CONFIG_HOME/bento/config.kdl`.

<h1 align="center">bento</h1>
<p align="center">A scrollable-tiling Wayland compositor, based on niri</p>
<p align="center">
    <a href="https://github.com/faetalize/bento/blob/main/LICENSE"><img alt="GitHub License" src="https://img.shields.io/github/license/faetalize/bento"></a>
</p>

## About

bento is a fork of niri but with changes that don't necessarily follow the original author's vision. Please do not open issues on the original repository if you face any issues using bento.

This repo follows a more permissive development stride, and thus may be less stable than niri. Caaaareful. Kay?

## Features

- A grid of cells instead of niri's single row of columns. The grid still scrolls horizontally without end, but it has 12 rows, and every window covers a rectangle of whole cells, so windows can sit in different rows, stack, and span several cells
- Window sizes snap to cells, and windows remember their size on the grid
- Left gravity (on by default): windows slide left within their rows to close gaps, like columns do in niri. Turn it off with `left-gravity false` to keep windows where you put them
- Opening, moving or resizing a window pushes the windows in its way aside
- niri's actions, tabs, fullscreen, gestures and IPC, adapted to the grid. See the [Grid Layout](docs/wiki/Grid-Layout.md) wiki page
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
