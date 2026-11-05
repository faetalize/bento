use std::collections::HashSet;
use std::ffi::OsStr;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::{env, io, process};

use anyhow::Context;
use async_channel::{Receiver, Sender, TrySendError};
use bento_ipc::{
    Event, EventKind, Focused, Get, LayoutEngine, OutputConfigChanged, Overview, Request, Response,
};
use calloop::futures::Scheduler;
use calloop::io::Async;
use directories::BaseDirs;
use futures_util::io::{AsyncReadExt, BufReader};
use futures_util::{select_biased, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, FutureExt as _};
use niri_config::actions::native_actions;
use niri_config::{Action, OutputName};
use smithay::input::pointer::{
    CursorIcon, CursorImageStatus, Focus, GrabStartData as PointerGrabStartData,
};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction};
use smithay::reexports::rustix::fs::unlink;
use smithay::utils::SERIAL_COUNTER;

use super::action::{self, ActionRequest};
use super::read;
use super::snapshot::{Focus as FocusedIds, Snapshot};
use crate::input::pick_window_grab::PickWindowGrab;
use crate::layout::tiling::engine;
use crate::niri::State;
use crate::utils::version;

// If an event stream client fails to read events fast enough that we accumulate more than this
// number in our buffer, we drop that event stream client.
const EVENT_STREAM_BUFFER_SIZE: usize = 64;

pub struct IpcServer {
    /// Path to the IPC socket.
    ///
    /// This is `None` when creating `IpcServer` without a socket.
    pub socket_path: Option<PathBuf>,
    event_streams: Vec<EventStreamSender>,
    /// What the event streams know. It's only kept up to date while there are streams.
    snapshot: Snapshot,
    /// Whether the last attempt to load the config failed, once there was one.
    config_failed: Option<bool>,
    /// The engine the event streams know, with the generation it was read at.
    engine: Option<LayoutEngine>,
    engine_generation: u64,
    /// Whether the engine's options may have changed since it was read.
    engine_dirty: bool,
}

struct ClientCtx {
    event_loop: LoopHandle<'static, State>,
    scheduler: Scheduler<()>,
}

struct EventStreamClient {
    /// What the subscriber has to be told first.
    replay: Vec<Event>,
    events: Receiver<Event>,
    disconnect: Receiver<()>,
    write: Box<dyn AsyncWrite + Unpin>,
}

struct EventStreamSender {
    events: Sender<Event>,
    disconnect: Sender<()>,
    /// The kinds of events the subscriber wants, or all of them.
    kinds: Option<HashSet<EventKind>>,
}

impl EventStreamSender {
    fn wants(&self, kind: EventKind) -> bool {
        self.kinds
            .as_ref()
            .is_none_or(|kinds| kinds.contains(&kind))
    }
}

impl IpcServer {
    pub fn start(
        event_loop: &LoopHandle<'static, State>,
        wayland_socket_name: Option<&OsStr>,
    ) -> anyhow::Result<Self> {
        let _span = tracy_client::span!("Ipc::start");

        let socket_path = if let Some(wayland_socket_name) = wayland_socket_name {
            let wayland_socket_name = wayland_socket_name.to_string_lossy();
            let socket_name = format!("bento.{wayland_socket_name}.{}.sock", process::id());
            let mut socket_path = socket_dir();
            socket_path.push(socket_name);

            let listener = UnixListener::bind(&socket_path).context("error binding socket")?;
            listener
                .set_nonblocking(true)
                .context("error setting socket to non-blocking")?;

            let source = Generic::new(listener, Interest::READ, Mode::Level);
            event_loop
                .insert_source(source, |_, socket, state| {
                    match socket.accept() {
                        Ok((stream, _)) => on_new_ipc_client(state, stream),
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => (),
                        Err(e) => return Err(e),
                    }

                    Ok(PostAction::Continue)
                })
                .unwrap();

            Some(socket_path)
        } else {
            None
        };

        Ok(Self {
            socket_path,
            event_streams: Vec::new(),
            snapshot: Snapshot::default(),
            config_failed: None,
            engine: None,
            engine_generation: 0,
            engine_dirty: false,
        })
    }

    fn send_event(&mut self, event: Event) {
        let kind = event.kind();
        let mut to_remove = Vec::new();
        for (idx, stream) in self.event_streams.iter().enumerate() {
            if !stream.wants(kind) {
                continue;
            }

            match stream.events.try_send(event.clone()) {
                Ok(()) => (),
                Err(TrySendError::Closed(_)) => to_remove.push(idx),
                Err(TrySendError::Full(_)) => {
                    warn!(
                        "disconnecting IPC event stream client \
                         because it is reading events too slowly"
                    );
                    to_remove.push(idx);
                }
            }
        }

        for idx in to_remove.into_iter().rev() {
            let stream = self.event_streams.swap_remove(idx);
            let _ = stream.disconnect.send_blocking(());
        }
    }

    fn wants(&self, kind: EventKind) -> bool {
        self.event_streams.iter().any(|stream| stream.wants(kind))
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        if let Some(socket_path) = &self.socket_path {
            let _ = unlink(socket_path);
        }
    }
}

fn socket_dir() -> PathBuf {
    BaseDirs::new()
        .as_ref()
        .and_then(|x| x.runtime_dir())
        .map(|x| x.to_owned())
        .unwrap_or_else(env::temp_dir)
}

fn on_new_ipc_client(state: &mut State, stream: UnixStream) {
    let _span = tracy_client::span!("on_new_ipc_client");
    trace!("new IPC client connected");

    let stream = match state.niri.event_loop.adapt_io(stream) {
        Ok(stream) => stream,
        Err(err) => {
            warn!("error making IPC stream async: {err:?}");
            return;
        }
    };

    let ctx = ClientCtx {
        event_loop: state.niri.event_loop.clone(),
        scheduler: state.niri.scheduler.clone(),
    };

    let future = async move {
        if let Err(err) = handle_client(ctx, stream).await {
            warn!("error handling IPC client: {err:?}");
        }
    };
    if let Err(err) = state.niri.scheduler.schedule(future) {
        warn!("error scheduling IPC stream future: {err:?}");
    }
}

async fn handle_client(ctx: ClientCtx, stream: Async<'static, UnixStream>) -> anyhow::Result<()> {
    let (read, mut write) = stream.split();
    let mut read = BufReader::new(read);

    loop {
        // Don't keep buf around to avoid clients wasting RAM by filling it with bogus data.
        let mut buf = Vec::new();
        let res = read.read_until(b'\n', &mut buf).await;
        match res {
            Ok(0) => return Ok(()),
            Ok(_) => (),
            // Normal client disconnection.
            Err(err) if err.kind() == io::ErrorKind::BrokenPipe => return Ok(()),
            Err(err) => {
                return Err(err).context("error reading request");
            }
        }

        let request = serde_json::from_slice::<Request>(&buf)
            .map_err(|err| format!("error parsing request: {err}"));
        let subscription = match &request {
            Ok(Request::Subscribe { events }) => Some(events.clone()),
            _ => None,
        };

        let reply = match request {
            Ok(request) => process(&ctx, request).await,
            Err(err) => Err(err),
        };

        if let Err(err) = &reply {
            warn!("error processing IPC request: {err}");
        }
        let response = reply.unwrap_or_else(Response::Error);

        buf.clear();
        serde_json::to_writer(&mut buf, &response).context("error formatting reply")?;
        buf.push(b'\n');
        write.write_all(&buf).await.context("error writing reply")?;

        if let Some(events) = subscription {
            let (events_tx, events_rx) = async_channel::bounded(EVENT_STREAM_BUFFER_SIZE);
            let (disconnect_tx, disconnect_rx) = async_channel::bounded(1);
            let sender = EventStreamSender {
                events: events_tx,
                disconnect: disconnect_tx,
                kinds: events.map(|kinds| kinds.into_iter().collect()),
            };

            // The state the subscriber is told and the events after it have to follow each other
            // without a gap, so both are settled in one go on the main thread.
            let replay = on_main(&ctx, move |state| state.ipc_subscribe(sender))
                .await
                .map_err(anyhow::Error::msg)?;

            // Spawn a task for the client.
            let client = EventStreamClient {
                replay,
                events: events_rx,
                disconnect: disconnect_rx,
                write: Box::new(write) as _,
            };
            let future = async move {
                if let Err(err) = handle_event_stream_client(client).await {
                    warn!("error handling IPC event stream client: {err:?}");
                }
            };
            if let Err(err) = ctx.scheduler.schedule(future) {
                warn!("error scheduling IPC event stream future: {err:?}");
            }

            return Ok(());
        }
    }
}

/// Runs `f` on the main thread, where the compositor's state is, and waits for its result.
async fn on_main<T: 'static>(
    ctx: &ClientCtx,
    f: impl FnOnce(&mut State) -> T + 'static,
) -> Result<T, String> {
    let (tx, rx) = async_channel::bounded(1);
    ctx.event_loop.insert_idle(move |state| {
        let _ = tx.send_blocking(f(state));
    });
    rx.recv()
        .await
        .map_err(|_| String::from("bento is shutting down"))
}

async fn process(ctx: &ClientCtx, request: Request) -> Result<Response, String> {
    let response = match request {
        Request::Get { what } => get(ctx, what).await?,
        Request::Action {
            name,
            args,
            properties,
            window,
            workspace,
        } => {
            let request = ActionRequest {
                name,
                args,
                properties,
                window,
                workspace,
            };
            // The engine lives on this thread, where the futures run too.
            let action = action::parse(request, engine::action_args)?;
            run_action(ctx, action).await;
            Response::Handled
        }
        Request::Output { output, action } => {
            action.validate()?;

            let found = on_main(ctx, move |state| {
                let found = state
                    .backend
                    .ipc_outputs()
                    .lock()
                    .unwrap()
                    .values()
                    .any(|o| OutputName::from_ipc_output(o).matches(&output));
                state.apply_transient_output_config(&output, action);
                found
            })
            .await?;

            Response::OutputConfigChanged(if found {
                OutputConfigChanged::Applied
            } else {
                OutputConfigChanged::OutputWasMissing
            })
        }
        Request::PickWindow => {
            let (tx, rx) = async_channel::bounded(1);
            ctx.event_loop.insert_idle(move |state| {
                let pointer = state.niri.seat.get_pointer().unwrap();
                let start_data = PointerGrabStartData {
                    focus: None,
                    button: 0,
                    location: pointer.current_location(),
                };
                let grab = PickWindowGrab::new(start_data);
                // The `WindowPickGrab` ungrab handler will cancel the previous ongoing pick, if
                // any.
                pointer.set_grab(state, grab, SERIAL_COUNTER.next_serial(), Focus::Clear);
                state.niri.pick_window = Some(tx);
                state
                    .niri
                    .cursor_manager
                    .set_cursor_image(CursorImageStatus::Named(CursorIcon::Crosshair));
                // Redraw to update the cursor.
                state.niri.queue_redraw_all();
            });
            let result = rx.recv().await;
            let id = result.map_err(|_| String::from("error getting picked window info"))?;
            let window = match id {
                Some(id) => {
                    on_main(ctx, move |state| {
                        let groups = read::groups(state);
                        read::windows(state, &groups).remove(&id.get())
                    })
                    .await?
                }
                None => None,
            };
            Response::PickedWindow(window)
        }
        Request::PickColor => {
            let (tx, rx) = async_channel::bounded(1);
            ctx.event_loop.insert_idle(move |state| {
                state.handle_pick_color(tx);
            });
            let result = rx.recv().await;
            let color = result.map_err(|_| String::from("error getting picked color"))?;
            Response::PickedColor(color)
        }
        Request::LoadConfigFile { path } => {
            if let Some(path) = &path {
                let p = Path::new(path);
                // A relative path would resolve against bento's working directory, which is
                // almost certainly not what you want.
                if !p.is_absolute() {
                    return Err(format!("path must be absolute: {path}"));
                }
                if !p.is_file() {
                    return Err(format!("path does not point to a file: {path}"));
                }
            }
            run_action(ctx, Action::LoadConfigFile(path)).await;
            Response::Handled
        }
        Request::FocusWindow { id } => {
            run_action(ctx, Action::FocusWindow(id)).await;
            Response::Handled
        }
        Request::SetWindowUrgent { id, urgent } => {
            let action = match urgent {
                Some(true) => Action::SetWindowUrgent(id),
                Some(false) => Action::UnsetWindowUrgent(id),
                None => Action::ToggleWindowUrgent(id),
            };
            run_action(ctx, action).await;
            Response::Handled
        }
        Request::StopCast { session_id } => {
            run_action(ctx, Action::StopCast(session_id)).await;
            Response::Handled
        }
        // The connection turns into an event stream once the reply is written.
        Request::Subscribe { .. } => Response::Handled,
    };

    Ok(response)
}

async fn get(ctx: &ClientCtx, what: Get) -> Result<Response, String> {
    let response = match what {
        Get::Version => Response::Version(version()),
        Get::Monitors => {
            on_main(ctx, |state| {
                let workspaces = read::workspaces(state);
                Response::Monitors(read::monitors(state, &workspaces).into_values().collect())
            })
            .await?
        }
        Get::Workspaces => {
            on_main(ctx, |state| {
                Response::Workspaces(read::workspaces(state).into_values().collect())
            })
            .await?
        }
        Get::Windows => {
            on_main(ctx, |state| {
                let groups = read::groups(state);
                Response::Windows(read::windows(state, &groups).into_values().collect())
            })
            .await?
        }
        Get::Groups => {
            on_main(ctx, |state| {
                Response::Groups(read::groups(state).into_values().collect())
            })
            .await?
        }
        Get::Layers => on_main(ctx, |state| Response::Layers(read::layers(state))).await?,
        Get::Focused => {
            on_main(ctx, |state| {
                let workspaces = read::workspaces(state);
                let groups = read::groups(state);
                Response::Focused(Focused {
                    monitor: read::monitors(state, &workspaces)
                        .into_values()
                        .find(|monitor| monitor.focused),
                    window: read::windows(state, &groups)
                        .into_values()
                        .find(|window| window.focused),
                })
            })
            .await?
        }
        Get::KeyboardLayouts => {
            on_main(ctx, |state| {
                Response::KeyboardLayouts(read::keyboard_layouts(state))
            })
            .await?
        }
        Get::Overview => {
            on_main(ctx, |state| {
                Response::Overview(Overview {
                    is_open: state.niri.layout.is_overview_open(),
                })
            })
            .await?
        }
        Get::Casts => {
            on_main(ctx, |state| {
                Response::Casts(read::casts(state).into_values().collect())
            })
            .await?
        }
        // Idle callbacks run on the main thread, where the engine lives, with the config.
        Get::Engine => {
            let info = on_main(ctx, |state| {
                let config = state.niri.config.borrow();
                engine::ipc_info(&config.tiling)
            })
            .await?;
            let info = info.ok_or_else(|| String::from("no layout engine is loaded"))?;
            Response::Engine(info)
        }
        Get::EngineState => {
            let state = on_main(ctx, |_| engine::exposed_state()).await?;
            Response::EngineState(state.map_err(|err| format!("{err:#}"))?)
        }
        Get::Actions => Response::Actions(native_actions()),
    };

    Ok(response)
}

/// Runs the action on the main thread and waits until it's done.
///
/// This is important for a few actions, for instance for DoScreenTransition this wait ensures that
/// the screen contents were sampled into the texture.
async fn run_action(ctx: &ClientCtx, action: Action) {
    let _ = on_main(ctx, move |state| {
        // Make sure some logic like workspace clean-up has a chance to run before doing
        // actions.
        state.niri.advance_animations();
        state.do_action(action, false);
    })
    .await;
}

async fn handle_event_stream_client(client: EventStreamClient) -> anyhow::Result<()> {
    let EventStreamClient {
        replay,
        events,
        disconnect,
        mut write,
    } = client;

    let mut replay = replay.into_iter();
    loop {
        let event = match replay.next() {
            Some(event) => event,
            None => match events.recv().await {
                Ok(event) => event,
                Err(_) => return Ok(()),
            },
        };

        let mut buf = serde_json::to_vec(&event).context("error formatting event")?;
        buf.push(b'\n');

        let res = select_biased! {
            _ = disconnect.recv().fuse() => return Ok(()),
            res = write.write_all(&buf).fuse() => res,
        };

        match res {
            Ok(()) => (),
            // Normal client disconnection.
            Err(err) if err.kind() == io::ErrorKind::BrokenPipe => return Ok(()),
            res @ Err(_) => res.context("error writing event")?,
        }
    }
}

impl State {
    /// Tells the event streams what changed since the last time.
    ///
    /// Nothing is read while nobody listens.
    pub fn ipc_refresh(&mut self) {
        let Some(server) = &self.niri.ipc_server else {
            return;
        };
        if server.event_streams.is_empty() {
            return;
        }

        self.ipc_update();
    }

    fn ipc_update(&mut self) {
        let _span = tracy_client::span!("State::ipc_update");

        let snapshot = self.ipc_snapshot();
        let state_changed = self.niri.layout.take_engine_state_changed();

        let generation = engine::generation();
        let engine = self
            .niri
            .ipc_server
            .as_ref()
            .is_some_and(|server| server.engine_dirty || server.engine_generation != generation)
            .then(|| engine::ipc_info(&self.niri.config.borrow().tiling));

        let Some(server) = &mut self.niri.ipc_server else {
            return;
        };

        let mut events = snapshot.events_since(Some(&server.snapshot));
        server.snapshot = snapshot;

        if let Some(engine) = engine {
            server.engine_generation = generation;
            server.engine_dirty = false;
            if server.engine != engine {
                server.engine = engine.clone();
                events.extend(engine.map(|engine| Event::EngineChanged { engine }));
            }
        }

        if state_changed && server.wants(EventKind::EngineStateChanged) {
            match engine::exposed_state() {
                Ok(state) => events.push(Event::EngineStateChanged { state }),
                Err(err) => warn!("error reading the layout engine's state: {err:?}"),
            }
        }

        for event in events {
            server.send_event(event);
        }
    }

    fn ipc_snapshot(&mut self) -> Snapshot {
        let workspaces = read::workspaces(self);
        let monitors = read::monitors(self, &workspaces);
        let groups = read::groups(self);
        let windows = read::windows(self, &groups);

        let focus = FocusedIds {
            monitor: monitors
                .values()
                .find(|monitor| monitor.focused)
                .map(|monitor| monitor.name.clone()),
            window: windows
                .values()
                .find(|window| window.focused)
                .map(|window| window.id),
        };

        Snapshot {
            monitors,
            workspaces,
            windows,
            groups,
            focus,
            keyboard_layouts: Some(read::keyboard_layouts(self)),
            overview_open: self.niri.layout.is_overview_open(),
            casts: read::casts(self),
            touchpad_on: !self.niri.touchpad_disabled_by_toggle,
        }
    }

    /// Adds an event stream, and returns what its subscriber has to be told first.
    fn ipc_subscribe(&mut self, stream: EventStreamSender) -> Vec<Event> {
        // The other streams get whatever happened before this one starts, so that what follows is
        // only what happens next.
        self.ipc_update();

        let Some(server) = &mut self.niri.ipc_server else {
            return Vec::new();
        };

        let mut replay = server.snapshot.events_since(None);
        if let Some(failed) = server.config_failed {
            replay.push(Event::ConfigLoaded { failed });
        }
        if let Some(engine) = server.engine.clone() {
            replay.push(Event::EngineChanged { engine });
        }
        if stream.wants(EventKind::EngineStateChanged) {
            match engine::exposed_state() {
                Ok(state) => replay.push(Event::EngineStateChanged { state }),
                Err(err) => warn!("error reading the layout engine's state: {err:?}"),
            }
        }
        replay.retain(|event| stream.wants(event.kind()));

        server.event_streams.push(stream);
        replay
    }

    pub fn ipc_config_loaded(&mut self, failed: bool) {
        let Some(server) = &mut self.niri.ipc_server else {
            return;
        };

        server.config_failed = Some(failed);
        server.engine_dirty = true;
        server.send_event(Event::ConfigLoaded { failed });
    }

    pub fn ipc_screenshot_taken(&mut self, path: Option<String>) {
        let Some(server) = &mut self.niri.ipc_server else {
            return;
        };

        server.send_event(Event::ScreenshotCaptured { path });
    }
}
