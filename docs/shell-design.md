# The bento shell

bento's shell is everything around the workspaces. What's inside a workspace, the windows and
what engines draw with them, is bento's and the engines', see `engine-design.md`.

## The shell is a full desktop shell

The shell is the whole desktop around the workspaces: the panel, the launcher,
notifications, on-screen indicators like volume, quick settings, the shortcut cheatsheet and
the lock screen. It replaces other shells like Noctalia entirely, rather than living next to
them.

## bento draws the shell

The shell is drawn by bento, in the same frame as the windows, as part of one scene: the islands,
their glass, text and icons, and their motion. There's one clock, so a morph, its glass and its
content never get out of step, and nothing the shell does between frames can make it skip one.
bento runs the shell's animations itself; the shell says where things go, bento moves them there.

The shell's data, sound, network, battery, notifications and the tray, comes from a program of its
own next to bento, which hands it to bento. A stuck call there never freezes the compositor.

Apps, bento's settings among them, stay GTK and libadwaita, as windows.

## bento is a Vulkan scene

bento renders with Vulkan, as one 3D scene: everything in it, windows and the shell, has a
position in depth and a 3D transform from the start. While anything moves, bento draws the whole
scene each frame, like a game; when nothing changes, it draws nothing. Fullscreen games go straight
to the display.

## Widgets are scripts

bento gives the shell primitives, in Rust:

- shapes, with any radius on each corner, continuous corners, borders, and a material;
- lines and paths, filled or stroked;
- text, icons and images;
- masks: clipping to a shape, gradient fades, and masking by another node's shape or text;
- shadows, opacity, blend modes and 3D transforms;
- layout boxes, input events, text editing, and springs.

They're drawn from distance fields where they can be, so they stay sharp at any size and every
number in them can be animated.

Widgets, bento's own and the user's, are JavaScript built from those primitives, and so is the
whole shell. They live in `~/.config/bento/shell/`, next to the widgets bento ships, and bento
reloads them when they change, without building anything or logging out. Scripts react to events
and say where things go; they never run in the per-frame loop. A script that fails leaves the last
working shell on screen and reports the error.

Styles are style objects in the scripts, with libadwaita's colours: the window colours, the accent,
light and dark.

How anything moves is the scripts' too: bento moves what scripts set with springs, at the speeds
they ask for, and says when a motion settles; how things come and go, grow out of a point, turn
into each other and fade is built from that in the scripts. bento holds no choreography of its
own.

Only a new kind of primitive, or a bug in bento itself, needs bento built again. Shaders for materials of one's own are
files bento loads, like the glass shader.

## Materials

Each shape has a material, as on Windows:

- solid;
- mica: the wallpaper alone, blurred and tinted, never the windows behind, so it's nearly free;
- acrylic: what's behind, blurred and tinted;
- glass: what's behind, blurred and refracted through a bezel.

## Islands float along the top

Along the top of each output float three islands of glass instead of one bar. They're inset from
the screen's edges by bento's gaps and rounded like the windows, and the wallpaper shows between
them.

They're above the windows, fullscreen ones too. While a window is fullscreen on their output, the
shell sends them up off the screen, and the pointer coming near the top edge brings them back.

- The left island shows the output's workspaces, as their numbers, or names when they have
  one, with the current one highlighted, and the minimap: a track of the active workspace with
  the part the view shows highlighted, from where bento says each tile sits.
- The centre island shows the clock.
- The right island shows the status icons (network, sound, battery and the like) and the tray
  of apps' status icons.

They don't show the focused window or a taskbar. Where one grows near another they melt together, as one piece
of glass.

## The shell is made of glass

The shell, the islands, their panels, the launcher and notifications, is glass: it blurs and
refracts what's behind it, and has a light edge highlight.

The glass is bento's material, the same behind the shell as behind apps, and bento's settings set
it. bento shades it towards libadwaita's window colour, light or dark with the system style, so
text and icons read on any wallpaper or window; the shell adds no tint of its own. The shell's
highlights use libadwaita's accent colour, so the shell matches the settings app and GTK apps.

## Islands morph into their panels

An island becomes its panel: clicking the clock morphs the centre island itself into the
calendar, and closing it morphs it back, one piece of glass changing shape rather than a panel
growing out of the clicked spot. Quick settings is the right island's panel. The motion is
restrained, with no overshoot.

## Notifications drip out of the right island

Notifications and toasts show at the top right of the screen, out of the right island: each one
melts down out of it like a drop of liquid, pulls away and settles into its toast, and goes back
up into it the same way when it's dismissed. It has to feel like one liquid, never like a second
panel sliding out of the first.

## On-screen indicators show at the bottom

On-screen indicators, like volume, brightness and caps lock, show at the bottom middle of the
screen, as on Windows 11. Each grows out of nothing and shrinks back into nothing.

## The launcher is a centred palette

The launcher is a glass search field with its results, in the middle of the screen, apart from
the islands. It grows out of nothing and shrinks back into nothing, like the on-screen
indicators. It's keyboard first: typing finds apps.

## bento starts the shell's data program

bento starts the shell's data program, and hands it a private connection: only it feeds the
shell, and gets the shell's commands and events. bento's own shell can be turned off in the
config for another one, Noctalia or any other, which bento starts the same way, as the program
with that private connection. Other programs keep bento's public socket.

## The shortcut cheatsheet

Holding Mod alone for a moment shows a cheatsheet of pinned shortcuts. bento notices the
gesture, since only it sees the keys, and shows the cheatsheet, then hides it when Mod is let
go or another key is pressed.

Which shortcuts are pinned is bento's, in its config, next to the shortcuts themselves, and
bento's settings pin and unpin them. The shell gets the pinned shortcuts with their keys and
descriptions from bento, so they stay the same whichever shell runs.
