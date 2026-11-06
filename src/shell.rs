//! The shell: the program the config names with `shell`, which bento starts with a private
//! Wayland connection of its own and starts again when it exits.

use std::collections::VecDeque;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use calloop::channel::{self, Sender};
use calloop::timer::{TimeoutAction, Timer};
use calloop::LoopHandle;
use smithay::reexports::wayland_server::backend::{ClientId, DisconnectReason};
use smithay::reexports::wayland_server::{Client, DisplayHandle};

use crate::niri::{ClientState, State};
use crate::utils::spawning::spawn_with_wayland_socket;

/// How long bento waits before starting the shell again after it exits.
const RESTART_DELAY: Duration = Duration::from_secs(1);

/// bento stops starting a shell that exits this many times within `CRASH_WINDOW`.
const MAX_STARTS: usize = 5;
const CRASH_WINDOW: Duration = Duration::from_secs(60);

pub struct Shell {
    command: Vec<String>,
    client: Option<Client>,
    /// When the shell was last started, to stop starting one that keeps exiting.
    starts: VecDeque<Instant>,
    /// Told the shell's client when it disconnects.
    exited: Sender<ClientId>,
    /// The style last sent to the shell.
    style: Option<(f64, f64)>,
}

impl Shell {
    pub fn new(event_loop: &LoopHandle<'static, State>) -> Self {
        let (exited, channel) = channel::channel();
        event_loop
            .insert_source(channel, |event, _, state| {
                if let channel::Event::Msg(id) = event {
                    state.niri.shell_exited(id);
                }
            })
            .unwrap();

        Self {
            command: Vec::new(),
            client: None,
            starts: VecDeque::new(),
            exited,
            style: None,
        }
    }

    pub fn client(&self) -> Option<&Client> {
        self.client.as_ref()
    }

    /// Whether the client is the shell.
    pub fn is_shell(client: &Client) -> bool {
        client
            .get_data::<ClientState>()
            .is_some_and(|data| data.shell.is_some())
    }

    /// Starts the shell with this command, replacing a running one if the command changed.
    pub fn set_command(&mut self, display: &DisplayHandle, command: Vec<String>) {
        if command == self.command && self.client.is_some() {
            return;
        }

        self.command = command;
        self.starts.clear();

        if let Some(client) = self.client.take() {
            // Its disconnection is for an old client by then, so it won't start another.
            display
                .backend_handle()
                .kill_client(client.id(), DisconnectReason::ConnectionClosed);
        }

        self.start(display);
    }

    fn start(&mut self, display: &DisplayHandle) {
        if self.command.is_empty() {
            return;
        }

        let now = Instant::now();
        while self
            .starts
            .front()
            .is_some_and(|start| now.duration_since(*start) > CRASH_WINDOW)
        {
            self.starts.pop_front();
        }
        if self.starts.len() >= MAX_STARTS {
            warn!(
                "the shell exited {MAX_STARTS} times within {}s, not starting it again",
                CRASH_WINDOW.as_secs()
            );
            return;
        }
        self.starts.push_back(now);

        let (ours, theirs) = match UnixStream::pair() {
            Ok(pair) => pair,
            Err(err) => {
                warn!("error making the shell's connection: {err:?}");
                return;
            }
        };

        let data = Arc::new(ClientState {
            compositor_state: Default::default(),
            can_view_decoration_globals: false,
            primary_selection_disabled: false,
            restricted: false,
            credentials_unknown: false,
            shell: Some(self.exited.clone()),
        });
        let client = match display.clone().insert_client(ours, data) {
            Ok(client) => client,
            Err(err) => {
                warn!("error inserting the shell's client: {err:?}");
                return;
            }
        };

        self.client = Some(client);
        spawn_with_wayland_socket(self.command.clone(), theirs.into());
    }
}

impl crate::niri::Niri {
    /// The gaps between windows and the corner radius the config gives every window.
    pub fn shell_style(&self) -> (f64, f64) {
        let config = self.config.borrow();
        let radius = config
            .window_rules
            .iter()
            .filter(|rule| rule.matches.is_empty() && rule.excludes.is_empty())
            .filter_map(|rule| rule.geometry_corner_radius)
            .next_back()
            .map_or(0., |radius| f64::from(radius.top_left));
        (config.layout.gaps, radius)
    }

    /// Starts the shell the config names, or replaces the running one, and tells it the style.
    pub fn update_shell(&mut self) {
        let command = self
            .config
            .borrow()
            .shell
            .as_ref()
            .map(|shell| shell.command.clone())
            .unwrap_or_default();
        let display = self.display_handle.clone();
        self.shell.set_command(&display, command);

        let style = self.shell_style();
        if self.shell.style != Some(style) {
            self.shell.style = Some(style);
            self.bento_shell_state.send_style(style.0, style.1);
        }
    }

    fn shell_exited(&mut self, id: ClientId) {
        if self.shell.client.as_ref().is_none_or(|c| c.id() != id) {
            return;
        }
        self.shell.client = None;

        warn!("the shell exited, starting it again");
        let timer = Timer::from_duration(RESTART_DELAY);
        self.event_loop
            .insert_source(timer, |_, _, state| {
                if state.niri.shell.client.is_none() {
                    let display = state.niri.display_handle.clone();
                    state.niri.shell.start(&display);
                }
                TimeoutAction::Drop
            })
            .unwrap();
    }
}
