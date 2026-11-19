# Open questions

What's still to be decided, so it isn't lost. Each one is settled by writing the decision into the
design doc it belongs to (`docs/shell-design.md`, `docs/engine-design.md`) and taking it off here.

## Shells' settings

What a setting is, who declares it, where it's kept, how it's applied and where it's set are in
`docs/shell-design.md`, "Shells keep settings and state". Still open:

- **Whether a shell can set a module's settings for itself.** A taskbar's clock in a smaller font
  than the clock elsewhere: either the shell's block in `settings.kdl` can hold values for the
  modules it uses, a layer between the module's setting and what it's placed with, or that's only
  ever done with what the module is placed with, its props, which the user can't change without
  editing the shell.
- **Whether which shell runs is a setting.** Today it's `~/.config/bento/shell/main.tsx`, a file to
  write. bento's settings could pick among bento's shells and the user's, which would make it a
  setting of bento's config, with `main.tsx` for a shell of one's own.
