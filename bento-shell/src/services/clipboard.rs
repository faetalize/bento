//! The clipboard's history: what was copied, newest first, and copying one of them again, through
//! the compositor's `ext-data-control-v1`.
//!
//! The history is kept only while the session lasts: text in memory, and pictures as files in
//! the runtime directory, which goes at logout. What's copied is often a password or a token,
//! so none of it is written to the disk, and what a password manager marks as secret isn't kept
//! at all.
//!
//! When the app that copied something goes, the clipboard would empty; this offers the newest
//! thing kept in its place, so what was copied can still be pasted.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::Write as _;
use std::os::fd::{AsFd as _, AsRawFd as _, OwnedFd};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex};

use gio::prelude::*;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::{event_created_child, Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::ext::data_control::v1::client::ext_data_control_device_v1::{
    self, ExtDataControlDeviceV1,
};
use wayland_protocols::ext::data_control::v1::client::ext_data_control_manager_v1::{
    self, ExtDataControlManagerV1,
};
use wayland_protocols::ext::data_control::v1::client::ext_data_control_offer_v1::{
    self, ExtDataControlOfferV1,
};
use wayland_protocols::ext::data_control::v1::client::ext_data_control_source_v1::{
    self, ExtDataControlSourceV1,
};

/// How many things are kept.
const KEEP: usize = 50;
/// The most text kept from one copy, in bytes; a larger copy isn't kept.
const MAX_TEXT: usize = 1 << 20;
/// The largest picture kept, in bytes.
const MAX_IMAGE: usize = 32 << 20;
/// How much of a text the shell is told, to show and search.
const PREVIEW: usize = 2000;
/// What password managers offer alongside a secret they copy (KDE's convention, which KeePassXC
/// follows too).
const SECRET: &str = "x-kde-passwordManagerHint";
/// What this offers alongside what it copies, to know its own copies when they come back.
const OWN: &str = "application/x-bento-clipboard";
/// The kinds of text taken, best first.
const TEXT: &[&str] = &[
    "text/plain;charset=utf-8",
    "UTF8_STRING",
    "text/plain",
    "TEXT",
    "STRING",
];
/// The kinds of picture taken, best first: those the shell can show.
const IMAGE: &[&str] = &["image/png", "image/jpeg", "image/webp"];

/// Something copied.
#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    Text(Arc<str>),
    Image {
        mime: String,
        bytes: Arc<[u8]>,
        /// Where it's written, for the shell to show.
        path: PathBuf,
    },
}

impl Content {
    /// The kinds it's offered as.
    fn mimes(&self) -> Vec<&str> {
        match self {
            Content::Text(_) => TEXT.to_vec(),
            Content::Image { mime, .. } => vec![mime.as_str()],
        }
    }

    fn bytes(&self) -> Arc<[u8]> {
        match self {
            Content::Text(text) => Arc::from(text.as_bytes()),
            Content::Image { bytes, .. } => bytes.clone(),
        }
    }

    /// Whether `other` is the same thing copied again.
    fn same(&self, other: &Content) -> bool {
        match (self, other) {
            (Content::Text(a), Content::Text(b)) => a == b,
            (Content::Image { bytes: a, .. }, Content::Image { bytes: b, .. }) => a == b,
            _ => false,
        }
    }
}

/// A thing in the history.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: u64,
    pub content: Content,
}

impl Entry {
    /// The start of its text, for the shell.
    pub fn preview(&self) -> Option<String> {
        match &self.content {
            Content::Text(text) => Some(text.chars().take(PREVIEW).collect()),
            Content::Image { .. } => None,
        }
    }
}

/// Which of the kinds an offer has to take, and whether it's text, or `None` to leave it.
fn pick(mimes: &[String]) -> Option<(&'static str, bool)> {
    let has = |kind: &str| mimes.iter().any(|mime| mime == kind);
    if has(SECRET) || has(OWN) {
        return None;
    }
    let text = TEXT.iter().find(|kind| has(kind)).map(|kind| (*kind, true));
    text.or_else(|| IMAGE.iter().find(|kind| has(kind)).map(|kind| (*kind, false)))
}

/// What was copied, as `bytes` of the kind `mime`, if it's kept.
fn content(mime: &str, bytes: Vec<u8>, path: impl FnOnce(&str) -> PathBuf) -> Option<Content> {
    if TEXT.contains(&mime) {
        let text = String::from_utf8(bytes).ok()?;
        // Only whitespace isn't worth keeping.
        if text.trim().is_empty() || text.len() > MAX_TEXT {
            return None;
        }
        return Some(Content::Text(Arc::from(text)));
    }
    if bytes.is_empty() || bytes.len() > MAX_IMAGE {
        return None;
    }
    let extension = mime.strip_prefix("image/")?;
    Some(Content::Image {
        mime: mime.to_owned(),
        bytes: Arc::from(bytes),
        path: path(extension),
    })
}

/// The history, newest first: `content` goes in front, or moves there if it's in it. Gives the
/// entries that fell out of it.
fn add(entries: &mut VecDeque<Entry>, id: u64, content: Content) -> Vec<Entry> {
    let mut gone = Vec::new();
    let entry = match entries.iter().position(|entry| entry.content.same(&content)) {
        Some(at) => {
            let old = entries.remove(at).unwrap();
            // The one copied again keeps its own file, and the new file goes.
            if old.content != content {
                gone.push(Entry { id, content });
            }
            old
        }
        None => Entry { id, content },
    };
    entries.push_front(entry);
    while entries.len() > KEEP {
        gone.extend(entries.pop_back());
    }
    gone
}

struct State {
    seat: Option<WlSeat>,
    manager: Option<ExtDataControlManagerV1>,
    device: Option<ExtDataControlDeviceV1>,
    /// What this offers as the clipboard, while it does.
    source: Option<ExtDataControlSourceV1>,
    entries: VecDeque<Entry>,
    next: u64,
    /// Goes up with each new clipboard, so that a read another one overtook is dropped.
    generation: u64,
    service: Weak<ClipboardService>,
    dir: PathBuf,
}

struct Inner {
    connection: Connection,
    queue: EventQueue<State>,
    qh: QueueHandle<State>,
    state: State,
}

pub struct ClipboardService {
    inner: RefCell<Option<Inner>>,
    on_change: Box<dyn Fn(bool, &VecDeque<Entry>)>,
}

impl ClipboardService {
    /// Starts watching the clipboard, with pictures written to `dir`. `on_change` hears whether
    /// there's a clipboard to watch, and the history.
    pub fn start(dir: PathBuf, on_change: impl Fn(bool, &VecDeque<Entry>) + 'static) -> Rc<Self> {
        let this = Rc::new(Self {
            inner: RefCell::new(None),
            on_change: Box::new(on_change),
        });
        // Pictures from an earlier run aren't in this one's history.
        if let Ok(files) = std::fs::read_dir(&dir) {
            for file in files.flatten() {
                if file.file_name().to_string_lossy().starts_with("clipboard-") {
                    let _ = std::fs::remove_file(file.path());
                }
            }
        }
        match Self::connect(&this, dir) {
            Ok(inner) => {
                let fd = inner.connection.backend().poll_fd().as_raw_fd();
                *this.inner.borrow_mut() = Some(inner);
                let weak = Rc::downgrade(&this);
                glib::source::unix_fd_add_local(fd, glib::IOCondition::IN, move |_, _| {
                    match weak.upgrade() {
                        Some(this) if this.pump() => glib::ControlFlow::Continue,
                        _ => glib::ControlFlow::Break,
                    }
                });
                this.pump();
                this.changed();
            }
            Err(err) => {
                eprintln!("bento-shell --data: no clipboard: {err}");
                (this.on_change)(false, &VecDeque::new());
            }
        }
        this
    }

    fn connect(this: &Rc<Self>, dir: PathBuf) -> Result<Inner, String> {
        let connection = Connection::connect_to_env().map_err(|err| err.to_string())?;
        let (globals, queue) = registry_queue_init::<State>(&connection).map_err(|err| err.to_string())?;
        let qh = queue.handle();
        let manager: ExtDataControlManagerV1 =
            globals.bind(&qh, 1..=1, ()).map_err(|err| err.to_string())?;
        let seat: WlSeat = globals.bind(&qh, 1..=1, ()).map_err(|err| err.to_string())?;
        let device = manager.get_data_device(&seat, &qh, ());
        let state = State {
            seat: Some(seat),
            manager: Some(manager),
            device: Some(device),
            source: None,
            entries: VecDeque::new(),
            next: 1,
            generation: 0,
            service: Rc::downgrade(this),
            dir,
        };
        Ok(Inner {
            connection,
            queue,
            qh,
            state,
        })
    }

    /// Reads and handles what the compositor said. Gives whether the connection's still there.
    fn pump(&self) -> bool {
        let mut inner = self.inner.borrow_mut();
        let Some(Inner {
            connection,
            queue,
            state,
            ..
        }) = inner.as_mut()
        else {
            return false;
        };
        if let Some(guard) = queue.prepare_read() {
            if let Err(err) = guard.read() {
                let would_block = matches!(
                    &err,
                    wayland_client::backend::WaylandError::Io(io)
                        if io.kind() == std::io::ErrorKind::WouldBlock
                );
                if !would_block {
                    eprintln!("bento-shell --data: the clipboard's connection went: {err}");
                    *inner = None;
                    return false;
                }
            }
        }
        let alive = queue.dispatch_pending(state).is_ok();
        let _ = connection.flush();
        alive
    }

    fn with(&self, f: impl FnOnce(&mut State, &QueueHandle<State>)) {
        let mut inner = self.inner.borrow_mut();
        if let Some(inner) = inner.as_mut() {
            f(&mut inner.state, &inner.qh);
            let _ = inner.connection.flush();
        }
    }

    fn changed(&self) {
        if let Some(inner) = self.inner.borrow().as_ref() {
            (self.on_change)(true, &inner.state.entries);
        }
    }

    /// Puts the thing `id` on the clipboard, and in front of the history.
    pub fn copy(&self, id: u64) {
        self.with(|state, qh| state.copy(qh, id));
        self.changed();
    }

    /// Takes the thing `id` out of the history.
    pub fn delete(&self, id: u64) {
        self.with(|state, _| {
            if let Some(at) = state.entries.iter().position(|entry| entry.id == id) {
                let gone = state.entries.remove(at);
                forget(gone);
            }
        });
        self.changed();
    }

    /// Empties the history, and the clipboard if it has one of its things.
    pub fn clear(&self) {
        self.with(|state, _| {
            for entry in state.entries.drain(..) {
                forget(Some(entry));
            }
            if let (Some(source), Some(device)) = (state.source.take(), &state.device) {
                device.set_selection(None);
                source.destroy();
            }
        });
        self.changed();
    }

    /// What was copied, read, unless another copy came meanwhile.
    fn took(&self, generation: u64, content: Content) {
        let mut kept = false;
        self.with(|state, _| {
            if state.generation != generation {
                return;
            }
            if let Content::Image { path, bytes, .. } = &content {
                if std::fs::write(path, bytes).is_err() {
                    return;
                }
            }
            let id = state.next;
            state.next += 1;
            for gone in add(&mut state.entries, id, content) {
                forget(Some(gone));
            }
            kept = true;
        });
        if kept {
            self.changed();
        }
    }
}

/// Lets an entry go, with its picture's file.
fn forget(entry: Option<Entry>) {
    if let Some(Entry {
        content: Content::Image { path, .. },
        ..
    }) = entry
    {
        let _ = std::fs::remove_file(path);
    }
}

impl State {
    fn copy(&mut self, qh: &QueueHandle<State>, id: u64) {
        let (Some(manager), Some(device)) = (&self.manager, &self.device) else {
            return;
        };
        let Some(at) = self.entries.iter().position(|entry| entry.id == id) else {
            return;
        };
        let entry = self.entries.remove(at).unwrap();
        let source = manager.create_data_source(qh, entry.content.clone());
        for mime in entry.content.mimes() {
            source.offer(mime.to_owned());
        }
        source.offer(OWN.to_owned());
        device.set_selection(Some(&source));
        if let Some(old) = self.source.replace(source) {
            old.destroy();
        }
        self.entries.push_front(entry);
    }

    /// The clipboard is `offer` now, or empty.
    fn selection(&mut self, conn: &Connection, qh: &QueueHandle<State>, offer: Option<ExtDataControlOfferV1>) {
        self.generation += 1;
        let Some(offer) = offer else {
            // The app that had it went: what it copied is still here to paste.
            if self.source.is_none() {
                if let Some(id) = self.entries.front().map(|entry| entry.id) {
                    self.copy(qh, id);
                }
            }
            return;
        };
        let mimes = offer
            .data::<Mutex<Vec<String>>>()
            .map(|mimes| mimes.lock().unwrap().clone())
            .unwrap_or_default();
        if mimes.iter().all(|mime| mime != OWN) {
            // Another app has the clipboard now, so this doesn't.
            if let Some(source) = self.source.take() {
                source.destroy();
            }
        }
        let Some((mime, _)) = pick(&mimes) else {
            offer.destroy();
            return;
        };
        let Ok((reader, writer)) = std::io::pipe() else {
            offer.destroy();
            return;
        };
        offer.receive(mime.to_owned(), writer.as_fd());
        drop(writer);
        offer.destroy();
        let _ = conn.flush();

        let generation = self.generation;
        let service = self.service.clone();
        let path = self.dir.join(format!("clipboard-{generation}"));
        glib::spawn_future_local(async move {
            let Some(bytes) = read(OwnedFd::from(reader), MAX_IMAGE.max(MAX_TEXT)).await else {
                return;
            };
            let content = content(mime, bytes, |extension| path.with_extension(extension));
            if let (Some(service), Some(content)) = (service.upgrade(), content) {
                service.took(generation, content);
            }
        });
    }
}

/// Reads all of `fd`, or nothing if it's more than `limit` bytes.
async fn read(fd: OwnedFd, limit: usize) -> Option<Vec<u8>> {
    let stream = gio::UnixInputStream::take_fd(fd);
    let mut bytes = Vec::new();
    loop {
        let chunk = stream
            .read_bytes_future(1 << 16, glib::Priority::DEFAULT)
            .await
            .ok()?;
        if chunk.is_empty() {
            return Some(bytes);
        }
        bytes.extend_from_slice(&chunk);
        if bytes.len() > limit {
            return None;
        }
    }
}

/// Writes `bytes` to `fd` away from the main loop, since the app pasting reads at its own pace.
fn write(fd: OwnedFd, bytes: Arc<[u8]>) {
    std::thread::spawn(move || {
        let _ = std::fs::File::from(fd).write_all(&bytes);
    });
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(_: &mut Self, _: &WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ExtDataControlManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ExtDataControlManagerV1,
        _: ext_data_control_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtDataControlDeviceV1, ()> for State {
    fn event(
        state: &mut Self,
        device: &ExtDataControlDeviceV1,
        event: ext_data_control_device_v1::Event,
        _: &(),
        conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_device_v1::Event::Selection { id } => state.selection(conn, qh, id),
            ext_data_control_device_v1::Event::PrimarySelection { id: Some(offer) } => {
                offer.destroy();
            }
            ext_data_control_device_v1::Event::Finished => {
                device.destroy();
                state.device = None;
                state.seat.take();
            }
            _ => (),
        }
    }

    event_created_child!(State, ExtDataControlDeviceV1, [
        ext_data_control_device_v1::EVT_DATA_OFFER_OPCODE =>
            (ExtDataControlOfferV1, Mutex::new(Vec::<String>::new())),
    ]);
}

impl Dispatch<ExtDataControlOfferV1, Mutex<Vec<String>>> for State {
    fn event(
        _: &mut Self,
        _: &ExtDataControlOfferV1,
        event: ext_data_control_offer_v1::Event,
        mimes: &Mutex<Vec<String>>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_data_control_offer_v1::Event::Offer { mime_type } = event {
            mimes.lock().unwrap().push(mime_type);
        }
    }
}

impl Dispatch<ExtDataControlSourceV1, Content> for State {
    fn event(
        state: &mut Self,
        source: &ExtDataControlSourceV1,
        event: ext_data_control_source_v1::Event,
        content: &Content,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_source_v1::Event::Send { mime_type, fd } => {
                let bytes = if mime_type == OWN {
                    Arc::from(&[][..])
                } else {
                    content.bytes()
                };
                write(fd, bytes);
            }
            ext_data_control_source_v1::Event::Cancelled => {
                if state.source.as_ref() == Some(source) {
                    state.source = None;
                }
                source.destroy();
            }
            _ => (),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn mimes(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn it_takes_text_before_a_picture_and_leaves_secrets() {
        assert_eq!(
            pick(&mimes(&["text/html", "UTF8_STRING", "text/plain;charset=utf-8"])),
            Some(("text/plain;charset=utf-8", true))
        );
        assert_eq!(
            pick(&mimes(&["text/html", "image/png"])),
            Some(("image/png", false))
        );
        // A password manager's copy, and this one's own coming back.
        assert_eq!(pick(&mimes(&["text/plain", SECRET])), None);
        assert_eq!(pick(&mimes(&["text/plain", OWN])), None);
        // Nothing it can show.
        assert_eq!(pick(&mimes(&["image/tiff", "text/html"])), None);
    }

    #[test]
    fn it_keeps_text_worth_keeping() {
        let path = |_: &str| PathBuf::from("/nowhere");
        assert_eq!(
            content("UTF8_STRING", b"hello".to_vec(), path),
            Some(Content::Text(Arc::from("hello")))
        );
        assert_eq!(content("text/plain", b"  \n".to_vec(), path), None);
        assert_eq!(content("text/plain", vec![0xff, 0xfe], path), None);
        let picture = content("image/png", vec![1, 2, 3], |ext| PathBuf::from(format!("/a.{ext}")));
        assert!(matches!(picture, Some(Content::Image { path, .. }) if path == Path::new("/a.png")));
    }

    #[test]
    fn copying_again_moves_it_to_the_front_and_the_oldest_go() {
        let text = |s: &str| Content::Text(Arc::from(s));
        let mut entries = VecDeque::new();
        add(&mut entries, 1, text("a"));
        add(&mut entries, 2, text("b"));
        assert!(add(&mut entries, 3, text("a")).is_empty());
        let ids: Vec<u64> = entries.iter().map(|entry| entry.id).collect();
        assert_eq!(ids, [1, 2]);

        let mut gone = Vec::new();
        for id in 10..10 + KEEP as u64 {
            gone.extend(add(&mut entries, id, text(&id.to_string())));
        }
        assert_eq!(entries.len(), KEEP);
        assert_eq!(gone.len(), 2);
        assert_eq!(entries.back().unwrap().id, 10);
    }
}
