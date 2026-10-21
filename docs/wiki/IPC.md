You can communicate with the running bento instance over an IPC socket.
Check `bento msg --help` for available commands.

bento keeps niri's IPC protocol, so tools written for niri's IPC keep working with bento, for example Waybar's niri modules, noctalia and DankMaterialShell.
The socket path is in `$NIRI_SOCKET` (bento kept niri's variable name), and Rust programs can keep using the `niri-ipc` crate.
The socket file itself is `$XDG_RUNTIME_DIR/bento.<wayland-socket>.<pid>.sock`.
What a window's position in the tiling layout means depends on the layout engine, see [Layout positions](#layout-positions) below.

The `--json` flag prints the response in JSON, rather than formatted.
For example, `bento msg --json outputs`.

> [!TIP]
> If you're getting parsing errors from `bento msg` after upgrading bento, make sure that you've restarted bento itself.
> You might be trying to run a newer `bento msg` against an older `bento` compositor.

### Event Stream

<sup>Since: 0.1.9</sup>

While most IPC requests return a single response, the event stream request will make bento continuously stream events into the IPC connection until it is closed.
This is useful for implementing various bars and indicators that update as soon as something happens, without continuous polling.

The event stream IPC is designed to give you the complete current state up-front, then follow up with updates to that state.
This way, your state can never "desync" from bento, and you don't need to make any other IPC information requests.

Where reasonable, event stream state updates are atomic, though this is not always the case.
For example, a window may end up with a workspace id for a workspace that had already been removed.
This can happen if the corresponding workspaces-changed event arrives before the corresponding window-changed event.

To get a taste of the events, run `bento msg event-stream`.
Though, this is more of a debug function than anything.
You can get raw events from `bento msg --json event-stream`, or by connecting to the IPC socket and requesting an event stream manually.

You can find the full list of events along with documentation in the `Event` enum of [the `niri-ipc` sub-crate](https://github.com/faetalize/bento/blob/main/niri-ipc/src/lib.rs).

### Programmatic Access

`bento msg --json` is a thin wrapper over writing and reading to a socket.
When implementing more complex scripts and modules, you're encouraged to access the socket directly.

Connect to the UNIX domain socket located at `$NIRI_SOCKET` in the filesystem.
Write your request encoded in JSON on a single line, followed by a newline character, or by flushing and shutting down the write end of the connection.
Read the reply as JSON, also on a single line.

You can use `socat` to test communicating with bento directly:

```sh
$ socat STDIO "$NIRI_SOCKET"
"FocusedWindow"
{"Ok":{"FocusedWindow":{"id":12,"title":"t socat STDIO /run/u ~","app_id":"Alacritty","workspace_id":6,"is_focused":true}}}
```

The reply is an `Ok` or an `Err` wrapping the same JSON object as you get from `bento msg --json`.

<sup>Since: next release</sup>
For more complex requests, you can pass `--print-request` to `bento msg` to see how the request should be formatted:

```sh
$ bento msg --print-request action focus-workspace 2
{"Action":{"FocusWorkspace":{"reference":{"Index":2}}}}
```

This is the format that you should use for communicating with the IPC socket directly.
You can also use `bento msg raw-request` to send a raw JSON request without `socat`:

```sh
$ echo '{"Action":{"FocusWorkspace":{"reference":{"Id":8}}}}' | bento msg raw-request
"Handled"
```

You can find all available requests and response types in [the `niri-ipc` sub-crate](https://github.com/faetalize/bento/blob/main/niri-ipc/src/lib.rs).

### Layout positions

Where tiled windows go is up to the [layout engine](./Configuration:-Tiling-Layouts.md), so the meaning of a window's `pos_in_scrolling_layout` depends on the engine.
It is the (column, row) pair that the engine reports, both starting at 1, or unset if the engine doesn't report one.
The `niri` engine reports the column and the position in the column, like in niri (see [the niri engine](./Niri-Engine.md)), and the `bento` engine reports a grid column and a position in its stack (see [the grid layout](./Grid-Layout.md#ipc)).

Layout actions sent over IPC, like `bento msg action focus-column-left`, go to the engine just like binds do.
`bento msg action layout-action <name> [args...]` runs an action that the engine adds.

### Backwards Compatibility

The JSON output *should* remain stable, as in:

- existing fields and enum variants should not be renamed
- non-optional existing fields should not be removed

However, new fields and enum variants will be added, so you should handle unknown fields or variants gracefully where reasonable.

The formatted/human-readable output (i.e. without `--json` flag) is **not** considered stable.
Please prefer the JSON output for scripts, since I reserve the right to make any changes to the human-readable output.

The `niri-ipc` sub-crate (like bento's other sub-crates) is *not* API-stable in terms of the Rust semver; rather, it follows the version of bento itself, which continues niri's version numbers.
In particular, new struct fields and enum variants will be added.
