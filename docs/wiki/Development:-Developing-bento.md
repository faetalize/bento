## Building

[Getting Started](./Getting-Started.md#building) lists the dependencies to install. Then, `cargo build --release` puts the bento binary in `target/release/bento`.

For iterating, there's a `fast` profile in `.cargo/config.toml`: `cargo build --profile fast` uses the same optimizations as a release build, but without LTO, so rebuilding after a small change takes seconds rather than minutes. The binary goes in `target/fast/bento`.

Check `Cargo.toml` for a list of build features. For example, you can replace systemd integration with dinit integration using `cargo build --release --no-default-features --features dinit,dbus,xdp-gnome-screencast`.

> [!WARNING]
> Do NOT build with `--all-features`!
>
> Some features are meant only for development use.
> For example, one of the features enables collection of profiling data into a memory buffer that will grow indefinitely until you run out of memory.

## Running a Local Build

The main way of testing bento during development is running it as a nested window. The second step is usually switching to a different TTY and running bento there.

`cargo nested` builds bento with the `fast` profile and runs it as a nested window. Arguments go to bento, for example `cargo nested -c path/to/config.kdl`. When nested, the Mod key is <kbd>Alt</kbd>.

Once a feature or fix is reasonably complete, you generally want to run a local build as your main compositor for proper testing. The easiest way to do that is to install bento once as described in [Getting Started](./Getting-Started.md#building), then copy `./target/release/bento` over the installed binary after each rebuild. Do make sure that you know how to revert to a working version in case everything breaks though.

If you use an RPM-based distro, you can generate an RPM package for a local build with `cargo generate-rpm`.

## Layout Engines

Layout engines are JavaScript modules in `resources/layouts/`:

- `bento.js` is the `bento` engine, the grid of cells described in [Grid Layout](./Grid-Layout.md).
- `niri.js` is the `niri` engine, a port of niri's scrollable columns from `src/layout/scrolling.rs` in upstream niri.
- `prelude.js` is the host side of the engine API: it turns bento's JSON events into calls on the engine and collects what the engine changed. It runs next to every engine, in bento and in the tests.
- `bento-layout.d.ts` has the types for the API that engines see.

The Rust side, which loads engines and applies what they do, is in `src/layout/tiling/`. See [Tiling Layouts](./Configuration:-Tiling-Layouts.md) for how engines work and how to write one.

The built-in engines are compiled into the bento binary, so changes to `bento.js` or `niri.js` need a rebuild. To iterate without rebuilding, point `tiling { layout }` at the file, for example `layout "~/path/to/bento/resources/layouts/bento.js"`: bento reloads it whenever it changes, and the windows stay where they are.

Calls to `ws.log()` in an engine show up in the bento log as `info!` messages, and errors in an engine as `warn!` messages.

## Logging Levels

bento uses [`tracing`](https://lib.rs/crates/tracing) for logging. This is how logging levels are used:

- `error!`: programming errors and bugs that are recoverable. Things you'd normally use `unwrap()` for. However, when a Wayland compositor crashes, it brings down the entire session, so it's better to recover and log an `error!` whenever reasonable. If you see an `ERROR` in the bento log, that always indicates a *bug*.
- `warn!`: something bad but still *possible* happened. Informing the user that they did something wrong, or that their hardware did something weird, falls into this category. For example, config parsing errors should be indicated with a `warn!`.
- `info!`: the most important messages related to normal operation. Running bento with `RUST_LOG=niri=info` should not make the user want to disable logging altogether. (The library crate is still called `niri`, so log targets start with `niri`.)
- `debug!`: less important messages related to normal operation. Running bento with `debug!` messages hidden should not negatively impact the UX.
- `trace!`: everything that can be useful for debugging but is otherwise too spammy or performance intensive. `trace!` messages are *compiled out* of release builds.

## Tests

We have some unit tests, most prominently for the layout code and for config parsing, and tests for the layout engines that run in node.

When adding new operations to the layout, add them to the `Op` enum in `src/layout/tests.rs` (this will automatically include it in the randomized tests), and if applicable to the `every_op` arrays below.

Tests for the Rust side of layout engines are in `src/layout/tests/tiling.rs`. They mostly use the built-in fallback engine, which puts windows side by side, and a few check that the shipped engines run.

When adding new config options, include them in the config parsing test.

### Running Tests

Make sure to run `cargo test --all` to run tests from sub-crates too. This also builds the `niri-visual-tests` crate, which needs libadwaita; add `--exclude niri-visual-tests` to skip it, like the CI does.

Some tests are a bit too slow to run normally, like the randomized tests of the layout code, so they are normally skipped. Set the `RUN_SLOW_TESTS` variable to run them:

```
env RUN_SLOW_TESTS=1 cargo test --all
```

It also usually helps to run the randomized tests for a longer period, so that they can explore more inputs. You can control this with environment variables, for example:

```
env RUN_SLOW_TESTS=1 PROPTEST_CASES=200000 PROPTEST_MAX_GLOBAL_REJECTS=200000 RUST_BACKTRACE=1 cargo test --release --all
```

Many tests spawn compositor instances and Wayland clients. This doesn't need a graphical session, but due to test parallelism, it can run into file descriptor limits on high core count systems. If that happens, limit the Rayon thread count as well as the test thread count, since some tests use Rayon internally:

```
env RAYON_NUM_THREADS=2 cargo test --all -- --test-threads=2
```

Some tests need surfaceless EGL. If it isn't available, you can skip them like so:

```
cargo test --all -- --skip=::egl
```

### Layout Engine Tests

The layout engines have their own tests, which run with [node](https://nodejs.org), without bento:

```
node --test resources/layouts/
```

They're in `resources/layouts/test/`, for the built-in engines, and in `resources/layouts/examples/`, for the template engine. `resources/layouts/test/host.js` plays bento's role: it opens and closes windows, runs actions and swipes, and shows where the engine puts everything.

The CI (`.github/workflows/ci.yml`) currently runs only the Rust tests, so run the node tests yourself when you change an engine or `prelude.js`.

### Visual Tests

The `niri-visual-tests` sub-crate is a GTK application that runs hard-coded test cases so that you can visually check that they look right. It uses mock windows with the real layout and rendering code. It is especially helpful when working on animations.

## Profiling

We have integration with the [Tracy](https://github.com/wolfpld/tracy) profiler which you can enable by building bento with a feature flag:

```
cargo build --release --features=profile-with-tracy-ondemand
```

Before starting Tracy-integrated bento, run the following commands to give Tracy access to CPU scheduling and context switches (can be very useful for diagnosing slowdowns):

```
sudo mount -o remount,gid=$(id -g),mode=750 /sys/kernel/tracing; echo -1 | sudo tee /proc/sys/kernel/perf_event_paranoid
```

Then you can open Tracy (you will need the latest stable release) and attach to a running bento instance to collect profiling data. Profiling data is collected "on demand"—that is, only when Tracy is connected. You can run a bento build like this as your main compositor if you'd like.

> [!NOTE]
> If you need to profile bento startup or the bento CLI, you can opt for "always on" profiling instead, using this feature flag:
>
> ```
> cargo build --release --features=profile-with-tracy
> ```
>
> When compiled this way, bento will **always** collect profiling data, so you can't run a build like this as your main compositor.

To make a function show up in Tracy, instrument it like this:

```rust
pub fn some_function() {
    let _span = tracy_client::span!("some_function");

    // Code of the function.
}
```

You can also enable Rust memory allocation profiling with `--features=profile-with-tracy-allocations`.

## Version String

bento continues niri's version numbers. The version string includes the version and the commit:

```
$ bento --version
bento 26.04 (v26.04-215-g3545791d)
```

The commit comes from `git describe`, so it shows "unknown commit" when building outside of a git repository. You can set it with the `NIRI_BUILD_COMMIT` environment variable at build time, or replace the whole version string with `NIRI_BUILD_VERSION_STRING`.

## Shell Completions

You can generate shell completions for several shells via `bento completions <SHELL>`, i.e. `bento completions bash`. See `bento completions -h` for a full list.
