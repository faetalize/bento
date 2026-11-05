# niri-ipc

Types that bento's config and its socket share with the compositor: actions' arguments, output
settings and layout engine descriptions. They come from niri's IPC crate; bento's socket itself
is in `bento-ipc`.

## Backwards compatibility

This crate follows the bento version.
It is **not** API-stable in terms of the Rust semver.
In particular, expect new struct fields and enum variants to be added in patch version bumps.
