# The bento shell

bento's shell is everything around the workspaces. What's inside a workspace, the windows and
what engines draw with them, is bento's and the engines', see `engine-design.md`.

## The shell is a full desktop shell

The shell is the whole desktop around the workspaces: the panel, the launcher,
notifications, on-screen indicators like volume, quick settings, the bind cheatsheet and
the lock screen. It replaces other shells like Noctalia entirely, rather than living next to
them.

## bento draws the shell

The shell is drawn by bento, in the same frame as the windows, as part of one scene: its surfaces,
their glass, text and icons, and their motion. There's one clock, so a morph, its glass and its
content never get out of step, and nothing the shell does between frames can make it skip one.
bento runs the shell's animations itself; the shell says where things go, bento moves them there.

The shell's surfaces go in as many layers below and above the windows as it wants. Other programs'
surfaces keep the layers Wayland's layer-shell gives them, background, bottom, top and overlay,
with the windows between bottom and top and fullscreen windows over the top layer, as their
toolkits expect, and the shell's layers go anywhere around those. The wallpaper can be a layer of
the shell's too. What's higher gets the keyboard and the pointer first.

A shell places its surfaces the way layer-shell places other programs': held to edges at margins
from them, in the middle of the output when held to none, or beside a node of its own, as a flyout
over its button, or a point, as a menu at the pointer, on whichever side has room. A surface held
to an edge can keep room along it that the windows stay out of, and bento gives the engines what's
left as each monitor's working area.

The shell's data, sound, network, battery, notifications and the tray, comes from a program of its
own next to bento, which hands it to bento. A stuck call there never freezes the compositor.

Apps, bento's settings among them, stay GTK and libadwaita, as windows.

## bento is a Vulkan scene

bento renders with Vulkan, as one 3D scene: everything in it, windows and the shell, has a
position in depth and a 3D transform from the start. While anything moves, bento draws the whole
scene each frame, like a game; when nothing changes, it draws nothing. Fullscreen games go straight
to the display.

## The shell is a game's UI layer

The shell is built the way a game engine's UI is: bento gives it primitives, and everything else,
the shell's parts, how they look, fit together and move, is the scripts'. bento assumes no bar,
panel or other part of a shell of its own.

bento gives the shell primitives, in Rust:

- shapes, with any radius on each corner, continuous corners, borders, and a material;
- lines and paths, filled or stroked;
- text, icons, the icon theme's and the shell's own, and images;
- live copies of windows, as engines have them, for previews and switchers;
- masks: clipping to a shape, gradient fades, and masking by another node's shape or text;
- shadows, opacity, blend modes and 3D transforms;
- layout boxes, input events, text editing, scrolling, and springs.

They're drawn from distance fields where they can be, so they stay sharp at any size and every
number in them can be animated.

## The shell is written in TSX and CSS

The shell is written like a web page, in TSX with Solid: components in JSX, which run once, and
values that update exactly what reads them when they change. Styles are CSS written as typed
objects in the scripts: its flexbox, its margins, its start and end and its states, with bento's
own properties for what CSS has no word for, like materials and springs, which an editor completes
and checks like the rest. libadwaita's colours are variables: the window colours, the accent, light
and dark.

A file can hold a whole shell with its interactions, or a single module, and imports others. A
module is one file that others import by its name: the components it exports, and the state and
code they share. Some are building blocks, like buttons, flyouts and menus; others are what the
shell shows of one thing, like sound, with its volume, devices and the views of them: a status
icon, sliders, a page, a switcher and an on-screen indicator. Those views and what they share live
in one place, and every shell that has them shares them. A component's modes are its props, and
what it holds are its children. bento ships its modules and two shells built from them, so that a
module of one's own, or a change to one of bento's, is one file.

Context menus are one of bento's modules, which any module opens, so every menu looks and works
alike. One thing is open over the desktop at a time, menus among them, but for a menu opened
from something that's open, like the rest of the tray: that stays under it until a press outside
both. A submenu opens in the menu's place, with a way back, as libadwaita's do. Menus go over the
shell, apart from its glass, in the overlay layer under other programs' overlays, like a
screenshot tool's, and the lock screen.

What scrolls says how far it's scrolled and how far it goes, and scripts draw its scrollbar, as
libadwaita's overlay scrollbars: thin, showing while the pointer is near or it scrolls, wider
under the pointer, and dragged.

Which shell runs is chosen the way the engine is: in bento's config, and in bento's settings,
among bento's shells and a file of one's own. bento's own shells aren't edited in place; one is
duplicated into a file of one's own to start from, saved wherever the user picks, and that file
is the shell that runs. A shell of one's own lives wherever its author keeps it, and the config
names its path. bento's modules and shells are always bento's own: nothing takes their place. A
module of one's own is a file of one's own, imported by its path, and its author keeps it up.
bento's own are worked on from a checkout of bento, which `debug { shell-scripts }` reads in
place of the built-in ones. bento reloads them when they change, without building anything or logging out. Scripts react to events
and say where things go; they never run in the per-frame loop. A script that fails leaves the last
working shell on screen and reports the error.

What a shell can use is found in an editor, from the types alone, with nothing to install: bento
keeps a copy of its modules, its shells, its API's types and Solid's in
`~/.local/share/bento/shell/`, written again when bento changes, with a `tsconfig.json` there
that a shell's own extends. bento writes nothing next to the user's files; Settings points to
how, when it makes a copy of a shell. Importing completes bento's modules, and
each module's props and every primitive are documented where they're used. The types are the one
place that says what there is; nothing else lists it.

How anything moves is the scripts' too: bento moves what scripts set with springs, at the speeds
they ask for, and says when a motion settles; how things come and go, grow out of a point, turn
into each other and fade is built from that in the scripts. bento holds no choreography of its
own.

Only a new kind of primitive, or a bug in bento itself, needs bento built again. Shaders for
materials of one's own are files bento loads, like the glass shader.

## Shells declare settings and actions, as engines do

Modules and shells say what they have in their `meta`, the same way engines do: `settings` and
`actions`, each by name, with the same types, written and checked by the same code. A module that
declares neither has none.

A setting is a typed value that a module or a shell declares: a module for what it shows, like a
clock's font or the folders pinned in a start menu, which every shell with that module shares; a
shell for itself, like the height of its taskbar. Every setting declared is the user's: what's
only the shell's to choose is a prop, not a setting.

An action is something a bind can run, like switching to the next pinned sound output, with
its description and arguments, as an engine's are. A bind runs it with
`shell-action "name" args…`. The module that declares an action handles it, like the launcher's
`toggle-launcher` and the start menu's `toggle-start-menu`, so a shell without a launcher has no
action for one. bento's own list of actions has only what bento does itself. A shell program
outside bento gets the `shell-action` binds too, by name, and handles the ones it has; nothing
in bento declares those, so they're the user's own, in the main config's `binds {}`.

An action can come with binds, like the start menu's Mod alone, the launcher's Mod+D and the
clipboard's Mod+V, as engines' actions can: they're bound while the module runs, and the user's
changes to them go in the block of whoever declares them, next to its settings, the way an
engine's go in its file. How defaults, changes and chords claimed twice work is the
same for every declarer; see "Binds: defaults stay with whoever declares them" in
`engine-design.md`.

What the user chooses is a setting, and what a shell notes for itself as it's used, like Do Not
Disturb or what's been seen, is state. Something the shell writes as it's used and the user chose,
like pinned apps, is a setting.

bento's own shells keep their settings, and the user's binds for their actions, in
`~/.config/bento/shell.kdl`: a block for each module and each shell by its name, with its
settings and its binds. A module's block applies to whichever of bento's shells runs. The
file is the user's, and holds only what the user changed: the defaults stay in the shells and
their modules.

A shell of one's own is a step out of bento's defaults, and has a file of its own, chosen as it's
picked, the way a custom engine's is. Switching to it in bento Settings, by picking its file or
making a copy of one of bento's, asks where its settings come from: "Start Fresh" makes an empty
file in `~/.config/bento/shells/`, named after the shell's file, with a number if that name is
taken, and "Use a Backup…" picks a `.kdl` the user has. The config names it next to the shell,
`shell { script "~/my-shell.tsx" settings="~/.config/bento/shells/my-shell.kdl" }`. Its modules'
settings and binds under it stay in that file: what's set under it doesn't reach bento's shells,
and theirs doesn't reach it, unless the user picks `shell.kdl` itself. If its file is missing,
the shell runs on its defaults and the Shell page says so.

What a shell prefers for a module it uses, like a clock on two
lines in a taskbar, isn't a setting: it's what it places the module with, its props. A module of one's own in place of bento's has the settings of its name. State is kept
apart, in bento's state folder.

A module reads its settings as they are, and gets them again as the file changes, without a
reload. How its props and its settings go together is the module's to say, one over the other or
one changing the other. A module can change its own settings, as when an app is pinned.

bento's settings show the running shell's settings and actions, as bento hands it their
declarations, the way they show the running engine's: the actions with bento's own on the
Binds page, in its Shell tab, and a setting only on the page it names, in the group it names, like
`page: "shell", group: "Taskbar"`. Nothing shows there unless it says so: what's changed where
it shows, like pinned apps and their order, is kept as a setting all the same, and doesn't
crowd bento Settings. The pages are bento Settings' own; a declaration picks one of them and
never makes one. What pages there are is in the types an editor gets, so naming one that isn't
there is an error there. Settings that nothing declares
stay in the file, and bento says so rather than failing, so that switching shells loses nothing.

## Materials

Each shape has a material, as on Windows:

- solid;
- mica: the wallpaper alone, blurred and tinted, never the windows behind, so it's nearly free;
- acrylic: what's behind, blurred and tinted;
- glass: what's behind, blurred and refracted through a bezel.

With bento's `plain` material, the shell's glass is blurred evenly too, without an edge to bend
or light it.

A shape's material fades with it, and a piece of glass melts no further than it's big, so one
growing out of other glass, or shrinking back into it, swells it only as much as it's there.

## bento's shells

bento ships two shells: its unix shell, islands of glass along the top, and its windows shell, a
taskbar along the bottom laid out like Windows 11's.

## The unix shell's islands float along the top

Along the top of each output float three islands of glass instead of one bar. They're inset from
the screen's edges by bento's gaps and rounded like the windows, and the wallpaper shows between
them.

They're above the windows, fullscreen ones too. While a window is fullscreen on their output, the
shell sends them up off the screen, and the pointer coming near the top edge brings them back.

- The left island shows the output's workspaces, as their numbers, or names when they have
  one, with the current one highlighted, and the minimap: a track of the active workspace with
  the part the view shows highlighted, from where bento says each tile sits.
- The centre island shows the clock.
- The right island shows three parts, with a faint line between each: the tray of apps' status
  icons; quick things, like the sound output switcher and the clipboard's button; and the status
  icons (network, sound, battery and the like), which open the control center.

They don't show the focused window or a taskbar. Where one grows near another they melt together, as one piece
of glass.

## The islands' panels

The clock opens into the calendar: the time and the date, the month, and the weather where the
user is. Where they are is in bento's config, and bento's settings set it; without a place, the
weather's room has a button that opens that page of the settings.

The right island opens into the control center, which is laid out like a modern phone's quick
settings: an easy way into bento's settings, quick actions, toggles for what matters most, what's
playing, and the notification center. Not everything is laid out at once: what has more to it
than a switch, like the networks or the sound devices, opens behind it, in the panel.

The control center's screenshot and screen recording buttons each run any command the settings
give them, alone or with a notification of the user's wording once it's done, for tools that show
none of their own.

A sound output can be pinned. While any are, the right island has a button that switches to the
next pinned one, as a bind can too. Its clipboard button opens the clipboard's history.

A bind opens an editor for the focused workspace: its name changes as it's typed, and the
editor takes the name away and puts it back.

## The unix shell is made of glass

The unix shell, the islands, their panels, the launcher and notifications, is glass: it blurs and
refracts what's behind it, and has a light edge highlight.

The glass, like every material, is bento's, the same behind the shell as behind apps, and bento's
settings set it. bento shades it towards libadwaita's window colour, light or dark with the system style, so
text and icons read on any wallpaper or window; the shell adds no tint of its own. The shell's
highlights use libadwaita's accent colour, so the shell matches the settings app and GTK apps.

## Islands morph into their panels

An island becomes its panel: clicking the clock morphs the centre island itself into the
calendar, and closing it morphs it back, one piece of glass changing shape rather than a panel
growing out of the clicked spot. Quick settings is the right island's panel. The motion is
restrained, with no overshoot.

## Notifications drip out of the right island

In the unix shell, notifications and toasts show at the top right of the screen, out of the right
island: each one
melts down out of it like a drop of liquid, pulls away and settles into its toast, and goes back
up into it the same way when it's dismissed. It has to feel like one liquid, never like a second
panel sliding out of the first.

## On-screen indicators show at the bottom

On-screen indicators, like volume, brightness and caps lock, show at the bottom middle of the
screen, as on Windows 11. Each grows out of nothing and shrinks back into nothing.

## The launcher is a centred palette

The unix shell's launcher is a glass search field with its results, in the middle of the screen,
apart from the islands. It grows out of nothing and shrinks back into nothing, like the on-screen
indicators. It's keyboard first: typing finds apps.

## The windows shell is a taskbar

Along the bottom of each output runs a taskbar, in the top layer, so fullscreen windows cover it.
In its middle, or on its left as its settings say, are the start button, a button for the
overview, and a button for each app pinned to it and each app with open windows, lit while one
of them has the focus. Resting on an app's button shows live previews of its windows over it,
and a click goes to them. A right click opens the app's menu, which pins it and closes its
windows, and a button dragged among the others stays where it's left; the pinned keep their
order. At its end, in three parts with a faint line
between each, are the tray; quick things, like the sound output switcher and the clipboard's
button; and quick settings, with the clock that opens the notification center and the month.
Then a sliver shows the desktop.

The start menu comes out of the start button: around it on a taskbar in the middle, and from its
left edge on one to the left. It finds apps the way the launcher does, shows all of them
otherwise, and has the power menu. Quick settings and the notification center are the control center's parts, each
coming out of its button. Every panel over the taskbar comes out the same distance from its edge,
close enough to melt into it as one piece, never so far that it half melts. Each slides out of
the taskbar's edge as if from behind it, and back in, its glass flowing out of the taskbar's. Notifications come up over the taskbar's end.

## The tray

The tray holds apps' status icons. The ones pinned to it show; the rest are behind a chevron, in
a flyout that opens on the tray's side. A click activates an icon, and a right click opens its
app's menu, in bento's own menu, with a way to pin the icon or take it off. Which icons are
pinned is kept, as pinned apps are, and isn't listed in bento's settings.

## bento starts the shell's data program

bento starts the shell's data program, and hands it a private connection: only it feeds the
shell, and gets the shell's commands and events. The shell bento draws can be turned off in the
config for another one, Noctalia or any other, which bento starts the same way, as the program
with that private connection. Other programs keep bento's public socket.

## The bind cheatsheet

Holding Mod alone for a moment shows a cheatsheet of pinned binds. bento notices the
gesture, since only it sees the keys, and shows the cheatsheet, then hides it when Mod is let
go or another key is pressed.

Which binds are pinned is bento's, in its config, next to the binds themselves, and
bento's settings pin and unpin them. The shell gets the pinned binds with their chords and
descriptions from bento, so they stay the same whichever shell runs.
