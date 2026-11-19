# Open questions

What's still to be decided, so it isn't lost. Each one is settled by writing the decision into the
design doc it belongs to (`docs/shell-design.md`, `docs/engine-design.md`) and taking it off here.

## Shells' settings and what they keep

Engines keep their options in a `.kdl` file next to their `.js`. Shells should have a way to keep
theirs too: a sibling `.kdl` or `.json` file, or something else. Today a shell only has `stored()`,
one JSON object in `~/.local/state/bento/shell.json` shared by every module, with no shape and no
declaration.

Not decided yet:

- **What a setting is, and whose.** A module's own, like the font of a clock; a shell's, like the
  height of its taskbar; or a feature's, shared by every shell that has it, like the folders pinned
  in a start menu or the size of icons. Whether these are one kind of thing at different scopes, or
  different kinds.
- **How a setting is declared.** Its name, type, default, choices and description, the way engines
  declare `meta.options`, and where: in the module, in the shell, or both.
- **Where they're kept, and as what.** A file next to the shell, one per module, or one for all;
  `.kdl` like the config and the engines, or JSON; and how settings differ from state a module
  keeps for itself, like which outputs are pinned.
- **How they're loaded, saved and applied.** Read once or followed as the file changes, written by
  the shell, by the module, or by bento, and handed to components as props, as signals, or
  through context.
- **Where they're set.** bento Settings, which would need to read the declarations of a shell it
  doesn't run; a page the shell itself owns; or one each module owns. The settings app isn't to be
  wired up until the shape above is decided.
- **What happens to one a shell no longer has**, and to settings when a module is replaced by one
  of the user's own.
