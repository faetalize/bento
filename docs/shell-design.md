# The bento shell

bento's shell is everything around the workspaces. What's inside a workspace, the windows and
what engines draw with them, is bento's and the engines', see `engine-design.md`.

## The shell is a full desktop shell

The shell provides the whole desktop around the workspaces: the panel, the launcher,
notifications, on-screen indicators like volume, quick settings, the shortcut cheatsheet and
the lock screen. It replaces other shells like Noctalia entirely, rather than living next to
them.

## The shell is its own program

The shell runs as a program of its own next to bento, not inside bento's process. A crash or
freeze in the shell leaves the windows and the session alive, and bento stays a compositor. The
shell talks to bento over Wayland protocols and bento's socket.

## The shell is built with GTK4 and libadwaita

The shell is a GTK4 and libadwaita program, like bento's settings, with gtk4-layer-shell for its
panels and overlays. Each part of the shell is a surface of its own, not one transparent surface
over the whole screen. GTK draws any shape, clips, masks, gradients, blur and shadows of the
shell's own content, and animates them, springs and morphing included. A part that needs a
shader of its own draws it in a GtkGLArea.

Effects that use what's behind the shell, like blur, frosted or liquid glass and refraction, are
bento's: no program can see what's under its own surfaces. The shell asks bento for them on its
surfaces, in the shapes it gives, the way windows ask for background blur.

## The bar floats along the top

The shell's bar runs along the top of each output, always on screen. It floats: it's inset from
the screen's edges by bento's gaps and rounded like the windows, so it sits in the layout as one
more tile rather than a strip glued to the edge.

The bar shows the output's workspaces, the clock, the status icons (network, sound, battery and
the like) and the tray of apps' status icons. It doesn't show the focused window or a taskbar.
Workspaces show as their numbers, or names when they have one, with the current one highlighted.

## The shell is made of glass

The shell's surfaces, the bar, its popovers, the launcher and notifications, are glass: bento
blurs and refracts what's behind them, and they have a light edge highlight.

The glass is bento's material, the same behind the shell as behind apps, and bento's settings set
it. bento shades it towards libadwaita's window colour, light or dark with the system style, so
text and icons read on any wallpaper or window; the shell draws no tint of its own. The shell's
highlights use libadwaita's accent colour, so the shell matches the settings app and GTK apps.

## The bar morphs into its panels

Opening something from the bar, like quick settings or the calendar, grows the part of the bar
that was clicked down into its panel, as one piece of glass, and closing it shrinks it back. The
bar and its open panel are one surface, whose glass shape the shell gives bento as it changes.
The motion is restrained, with no overshoot.

Notifications and on-screen indicators, like volume and brightness, morph out of the bar the same
way, so everything the shell shows comes from the bar and moves alike.

## The launcher is a centred palette

The launcher is a glass search field with its results, in the middle of the screen, apart from
the bar. It's keyboard first: typing finds apps.

## bento starts the shell the config names

The config names the shell's command, and bento starts it. bento hands that process a private
connection, and only it gets the shell's commands and events, like being told to show the
shortcut cheatsheet. Any shell can be the one, bento's own, Noctalia or another, by being the one
the config names. Other programs keep bento's public socket, without the shell's commands.

## The shortcut cheatsheet

Holding Mod alone for a moment shows a cheatsheet of pinned shortcuts. bento notices the
gesture, since only it sees the keys, and tells the shell to show the cheatsheet, then to hide
it when Mod is let go or another key is pressed. The shell draws it.

Which shortcuts are pinned is bento's, in its config, next to the shortcuts themselves, and
bento's settings pin and unpin them. The shell gets the pinned shortcuts with their keys and
descriptions from bento, so they stay the same whichever shell runs.
