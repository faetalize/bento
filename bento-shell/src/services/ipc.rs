//! bento's workspaces and actions, through its private socket.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::thread;
use std::time::Duration;

use bento_ipc::socket::{Socket, SOCKET_PATH_ENV};
use bento_ipc::{Event, EventKind, Request, Response, Workspace};
use serde_json::{Map, Value};

/// The shortest and longest wait before connecting to bento again.
const RETRY_MIN: Duration = Duration::from_millis(250);
const RETRY_MAX: Duration = Duration::from_secs(5);

/// What the helper thread hands the main loop.
enum Message {
    /// A new subscription started, so the events that follow describe everything from scratch.
    Connected,
    /// The connection closed.
    Lost,
    /// A workspace appeared or changed.
    Changed(Workspace),
    Removed(u64),
    /// The keyboard focus moved to this monitor, or off every monitor.
    Focus(Option<String>),
}

impl Message {
    fn from_event(event: Event) -> Option<Self> {
        match event {
            Event::WorkspaceAdded { workspace } | Event::WorkspaceChanged { workspace } => {
                Some(Self::Changed(workspace))
            }
            Event::WorkspaceRemoved { id } => Some(Self::Removed(id)),
            Event::FocusChanged { monitor, .. } => Some(Self::Focus(monitor)),
            _ => None,
        }
    }
}

/// The workspaces on every monitor, kept up to date from bento's event stream.
pub struct Workspaces {
    workspaces: RefCell<Vec<Workspace>>,
    /// The monitor that has the keyboard focus.
    focused_monitor: RefCell<Option<String>>,
}

impl Workspaces {
    /// Subscribes to bento's workspace events and calls `on_change` with the workspaces, sorted by
    /// monitor and then by their place on it, whenever they change. It connects again when bento
    /// restarts.
    pub fn start(on_change: impl Fn(&[Workspace]) + 'static) -> Rc<Self> {
        let this = Rc::new(Self {
            workspaces: RefCell::default(),
            focused_monitor: RefCell::default(),
        });

        if std::env::var_os(SOCKET_PATH_ENV).is_none() {
            eprintln!("{SOCKET_PATH_ENV} is not set, so there are no workspaces to show");
            on_change(&[]);
            return this;
        }

        let (sender, receiver) = async_channel::unbounded();
        let thread = thread::Builder::new()
            .name(String::from("bento events"))
            .spawn(move || subscribe(&sender));
        if let Err(err) = thread {
            eprintln!("Couldn't start reading bento's events: {err}");
            on_change(&[]);
            return this;
        }

        let weak: Weak<Self> = Rc::downgrade(&this);
        glib::spawn_future_local(async move {
            while let Ok(message) = receiver.recv().await {
                let Some(this) = weak.upgrade() else { break };
                let workspaces = {
                    let mut workspaces = this.workspaces.borrow_mut();
                    let mut changed = this.apply(&mut workspaces, message);
                    // Events come in bursts, like the whole state after connecting; one call
                    // covers it.
                    while let Ok(message) = receiver.try_recv() {
                        changed |= this.apply(&mut workspaces, message);
                    }
                    if !changed {
                        continue;
                    }
                    sort(&mut workspaces);
                    workspaces.clone()
                };
                on_change(&workspaces);
            }
        });

        this
    }

    /// The workspaces as they were last reported.
    pub fn current(&self) -> Vec<Workspace> {
        self.workspaces.borrow().clone()
    }

    /// Switches to the workspace with `id`, on its own monitor. This runs off the main loop.
    /// The name of the monitor that has the keyboard focus, as bento last said.
    pub fn focused_monitor(&self) -> Option<String> {
        self.focused_monitor.borrow().clone()
    }

    /// Keeps the focus, or applies a workspace change and says whether the workspaces changed.
    fn apply(&self, workspaces: &mut Vec<Workspace>, message: Message) -> bool {
        if let Message::Focus(monitor) = message {
            self.focused_monitor.replace(monitor);
            return false;
        }
        apply(workspaces, message)
    }

    pub fn focus(&self, id: u64) {
        let thread = thread::Builder::new().spawn(move || {
            if let Err(err) = request(Request::FocusWorkspace { id }) {
                eprintln!("Couldn't switch to workspace {id}: {err}");
            }
        });
        if let Err(err) = thread {
            eprintln!("Couldn't switch to workspace {id}: {err}");
        }
    }
}

/// Runs one of bento's actions, written like in a bind, and waits for bento to answer.
///
/// Logging out is `run_action("quit", vec![], {"skip-confirmation": true})`.
pub fn run_action(
    name: &str,
    args: Vec<Value>,
    properties: Map<String, Value>,
) -> Result<(), String> {
    request(Request::Action {
        name: String::from(name),
        args,
        properties,
        window: None,
        workspace: None,
    })
}

/// Sends bento a request and waits for it to answer.
fn request(request: Request) -> Result<(), String> {
    let mut socket = Socket::connect().map_err(|err| format!("Couldn't reach bento: {err}"))?;
    match socket.send(&request) {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(message)) => Err(message),
        Err(err) => Err(format!("Couldn't talk to bento: {err}")),
    }
}

/// Reads bento's workspace events into `sender` until the main loop stops listening, connecting
/// again whenever the connection drops.
fn subscribe(sender: &async_channel::Sender<Message>) {
    let mut retry = RETRY_MIN;
    loop {
        match open_stream() {
            Ok(mut next) => {
                retry = RETRY_MIN;
                if sender.send_blocking(Message::Connected).is_err() {
                    return;
                }
                let err = loop {
                    match next() {
                        Ok(event) => {
                            let Some(message) = Message::from_event(event) else {
                                continue;
                            };
                            if sender.send_blocking(message).is_err() {
                                return;
                            }
                        }
                        Err(err) => break err,
                    }
                };
                eprintln!("Lost bento's event stream: {err}");
                if sender.send_blocking(Message::Lost).is_err() {
                    return;
                }
            }
            Err(err) => eprintln!("Couldn't subscribe to bento's events: {err}"),
        }

        if sender.is_closed() {
            return;
        }
        thread::sleep(retry);
        retry = (retry * 2).min(RETRY_MAX);
    }
}

fn open_stream() -> Result<impl FnMut() -> std::io::Result<Event>, String> {
    let mut socket = Socket::connect().map_err(|err| err.to_string())?;
    let request = Request::Subscribe {
        events: Some(vec![
            EventKind::WorkspaceAdded,
            EventKind::WorkspaceChanged,
            EventKind::WorkspaceRemoved,
            EventKind::FocusChanged,
        ]),
    };
    match socket.send(&request) {
        Ok(Ok(Response::Handled)) => Ok(socket.read_events()),
        Ok(Ok(_)) => Err(String::from("bento sent an unexpected reply")),
        Ok(Err(message)) => Err(message),
        Err(err) => Err(err.to_string()),
    }
}

/// Applies `message` to `workspaces`, and says whether they changed.
fn apply(workspaces: &mut Vec<Workspace>, message: Message) -> bool {
    match message {
        Message::Connected | Message::Lost => {
            let changed = !workspaces.is_empty();
            workspaces.clear();
            changed
        }
        Message::Changed(workspace) => match workspaces.iter_mut().find(|w| w.id == workspace.id) {
            Some(old) if *old == workspace => false,
            Some(old) => {
                *old = workspace;
                true
            }
            None => {
                workspaces.push(workspace);
                true
            }
        },
        Message::Focus(_) => false,
        Message::Removed(id) => {
            let len = workspaces.len();
            workspaces.retain(|w| w.id != id);
            workspaces.len() != len
        }
    }
}

/// Sorts workspaces by monitor, those without one last, then top to bottom.
fn sort(workspaces: &mut [Workspace]) {
    workspaces.sort_by(|a, b| {
        (a.monitor.is_none(), &a.monitor, a.index).cmp(&(b.monitor.is_none(), &b.monitor, b.index))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(id: u64, monitor: Option<&str>, index: usize) -> Workspace {
        Workspace {
            id,
            name: None,
            monitor: monitor.map(String::from),
            index,
            windows: Vec::new(),
            focused: None,
            active: false,
            urgent: false,
        }
    }

    fn event(event: Event) -> Message {
        Message::from_event(event).unwrap()
    }

    fn ids(workspaces: &[Workspace]) -> Vec<u64> {
        workspaces.iter().map(|w| w.id).collect()
    }

    #[test]
    fn events_add_change_and_remove_workspaces() {
        let mut workspaces = Vec::new();
        let added = Event::WorkspaceAdded {
            workspace: workspace(1, Some("DP-1"), 0),
        };
        assert!(apply(&mut workspaces, event(added.clone())));
        assert!(!apply(&mut workspaces, event(added)));

        let mut active = workspace(1, Some("DP-1"), 0);
        active.active = true;
        let changed = Event::WorkspaceChanged { workspace: active };
        assert!(apply(&mut workspaces, event(changed)));
        assert!(workspaces[0].active);

        assert!(!apply(
            &mut workspaces,
            event(Event::WorkspaceRemoved { id: 9 })
        ));
        assert!(apply(
            &mut workspaces,
            event(Event::WorkspaceRemoved { id: 1 })
        ));
        assert!(workspaces.is_empty());
    }

    #[test]
    fn a_new_connection_starts_from_scratch() {
        let mut workspaces = vec![workspace(1, None, 0)];
        assert!(apply(&mut workspaces, Message::Connected));
        assert!(workspaces.is_empty());
        assert!(!apply(&mut workspaces, Message::Lost));
    }

    #[test]
    fn workspaces_sort_by_monitor_then_index() {
        let mut workspaces = vec![
            workspace(1, None, 0),
            workspace(2, Some("HDMI-A-1"), 1),
            workspace(3, Some("DP-1"), 1),
            workspace(4, Some("HDMI-A-1"), 0),
            workspace(5, Some("DP-1"), 0),
        ];
        sort(&mut workspaces);
        assert_eq!(ids(&workspaces), [5, 3, 4, 2, 1]);
    }
}
