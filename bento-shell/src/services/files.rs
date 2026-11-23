//! Finding files by their names, with `fd`, and opening them.
//!
//! `fd` reads the names in the folders searched anew for each search, the home folder unless it
//! says, so there's no index to keep, and leaves out what's hidden or ignored by git. It says which
//! names match, but not how well: what it finds is ranked here, and only the best of it is kept.
//!
//! `fd` reads a folder given twice twice, and one inside another given too again, and leaves out
//! folders by patterns rather than by their paths, so which folders it reads is settled here: see
//! [`plan`].

use std::cell::{Cell, RefCell};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use gio::prelude::*;
use gio::{BusType, DBusCallFlags};
use glib::{Variant, VariantDict};
use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};
use serde_json::{json, Value};

/// The most files a search finds.
const MOST: usize = 50;

/// What separates a name's words, besides spaces.
const SEPARATORS: [char; 3] = ['-', '_', '.'];

pub struct FileSearch {
    running: RefCell<Option<Running>>,
    /// Whether `fd` was there when last looked for.
    available: Cell<Option<bool>>,
    say: Say,
}

/// What hears the files a search found, from a thread of the search's own, and whether there's
/// finding any.
type Say = fn(&Value);

/// A search on its way, which ends when this goes.
struct Running {
    id: Value,
    ended: Arc<AtomicBool>,
    /// The `fd` reading a folder now.
    fd: Arc<Mutex<Option<Child>>>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.ended.store(true, Ordering::Relaxed);
        if let Some(fd) = self.fd.lock().unwrap().as_mut() {
            let _ = fd.kill();
        }
    }
}

impl FileSearch {
    pub fn start(say: Say) -> Self {
        let this = Self {
            running: RefCell::default(),
            available: Cell::default(),
            say,
        };
        this.find_fd();
        this
    }

    /// Finds `fd`, and says whether it's there when that's news: it's looked for anew for each
    /// search, as it may be installed or removed meanwhile.
    fn find_fd(&self) -> Option<PathBuf> {
        let fd = find_fd();
        let available = Some(fd.is_some());
        if self.available.replace(available) != available {
            (self.say)(&json!({ "type": "file-search", "available": available }));
        }
        fd
    }

    /// Finds the files that `text` names in `folders`, the home folder when there are none, but
    /// for those in `excluded`, and says them as `{ "type": "files", "id": id, … }`. It takes the
    /// place of a search on its way, which then says nothing.
    pub fn search(&self, id: Value, text: &str, folders: &[String], excluded: &[String]) {
        self.running.take();
        let asked = id.clone();
        let found = move |files: Vec<Value>| json!({ "type": "files", "id": id, "files": files });
        let (Some(fd), Some(query)) = (self.find_fd(), Query::new(text)) else {
            (self.say)(&found(Vec::new()));
            return;
        };
        let folders: Vec<PathBuf> = if folders.is_empty() {
            vec![glib::home_dir()]
        } else {
            folders.iter().map(|folder| resolve(folder)).collect()
        };
        let excluded: Vec<PathBuf> = excluded.iter().map(|folder| resolve(folder)).collect();
        let walks = plan(&folders, &excluded);

        let running = Running {
            id: asked,
            ended: Arc::default(),
            fd: Arc::default(),
        };
        let (ended, current) = (running.ended.clone(), running.fd.clone());
        self.running.replace(Some(running));
        let say = self.say;
        std::thread::spawn(move || {
            let recent = std::fs::read_to_string(glib::user_data_dir().join("recently-used.xbel"));
            let recent = recent.map(|xbel| read_recent(&xbel)).unwrap_or_default();
            let mut best = BinaryHeap::with_capacity(MOST + 1);
            // One folder after another, as each reads on every core.
            for (folder, left_out) in walks {
                // It reads the disk on every core, so it gives way to whatever draws.
                let mut command = Command::new("nice");
                command
                    .args(["-n", "19"])
                    .arg(&fd)
                    .args(["--fixed-strings", "--ignore-case", "--print0"])
                    .args(["--color", "never"]);
                for inside in &left_out {
                    command.arg("--exclude").arg(exclude_pattern(inside));
                }
                command
                    .arg("--")
                    .arg(query.pattern())
                    .arg(&folder)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped());
                let mut child = match command.spawn() {
                    Ok(child) => child,
                    Err(err) => {
                        eprintln!("bento-shell --data: couldn't run fd: {err}");
                        break;
                    }
                };
                let names = BufReader::new(child.stdout.take().expect("its stdout is a pipe"));
                *current.lock().unwrap() = Some(child);
                // Ended as it started: what ends it may have missed it.
                if ended.load(Ordering::Relaxed) {
                    if let Some(child) = current.lock().unwrap().as_mut() {
                        let _ = child.kill();
                    }
                }
                keep_best(&mut best, names, &query, &recent, MOST);
                let status = current.lock().unwrap().take().map(|mut child| child.wait());
                if ended.load(Ordering::Relaxed) {
                    return;
                }
                if !matches!(status, Some(Ok(status)) if status.success()) {
                    eprintln!("bento-shell --data: fd failed in {folder:?}: {status:?}");
                }
            }
            let best = best.into_sorted_vec();
            say(&found(best.iter().map(File::json).collect()));
        });
    }

    /// Ends the search `id`, if it's still on its way.
    pub fn stop(&self, id: &Value) {
        let mut running = self.running.borrow_mut();
        if running.as_ref().is_some_and(|running| running.id == *id) {
            running.take();
        }
    }
}

/// `fd`, which Debian calls `fdfind`, having another program called `fd`.
fn find_fd() -> Option<PathBuf> {
    ["fdfind", "fd"].iter().find_map(glib::find_program_in_path)
}

/// What a search looks for.
struct Query {
    /// Lowercase, each somewhere in the name.
    words: Vec<String>,
    /// Lowercase, the words between separators.
    parts: Vec<String>,
}

/// How a name fits a query, the best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Fit {
    /// The name is the query, with or without its extension.
    Whole,
    /// It starts with the query.
    Start,
    /// Each of the query's words starts one of its own.
    Words,
    /// It has the query's words in it.
    Within,
}

impl Query {
    /// `None` when no name can match `text`.
    fn new(text: &str) -> Option<Self> {
        // Neither can be in a name.
        if text.contains(['/', '\0']) {
            return None;
        }
        let text = text.to_lowercase();
        let words: Vec<String> = text.split_whitespace().map(String::from).collect();
        (!words.is_empty()).then(|| Self {
            words,
            parts: parts(&text),
        })
    }

    /// What `fd` looks for, the longest of the words. The others are looked for in what it finds,
    /// since only newer versions of it take more than one.
    fn pattern(&self) -> &str {
        let longest = self.words.iter().max_by_key(|word| word.len());
        longest.map(String::as_str).unwrap_or_default()
    }

    /// How a file's `name` fits, if it does.
    fn fit(&self, name: &str, folder: bool) -> Option<Fit> {
        let name = name.to_lowercase();
        if !self.words.iter().all(|word| name.contains(word.as_str())) {
            return None;
        }
        // Looking for nothing but separators.
        if self.parts.is_empty() {
            return Some(Fit::Within);
        }
        let named = parts(&name);
        // A folder's name has no extension.
        let stem = Path::new(&name).file_stem().and_then(|stem| stem.to_str());
        let stem = stem.filter(|_| !folder).map(parts);
        let (whole, query) = (named.join(" "), self.parts.join(" "));
        Some(
            if named == self.parts || stem.as_ref() == Some(&self.parts) {
                Fit::Whole
            } else if whole.starts_with(&query) {
                Fit::Start
            } else if self
                .parts
                .iter()
                .all(|part| named.iter().any(|named| named.starts_with(part.as_str())))
            {
                Fit::Words
            } else {
                Fit::Within
            },
        )
    }
}

fn parts(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || SEPARATORS.contains(&c))
        .filter(|part| !part.is_empty())
        .map(String::from)
        .collect()
}

/// A file that was found. They're ordered by how well they match, the best first: by how the name
/// fits, then those opened lately, then those fewest folders deep, then the shortest names. When
/// a file changed isn't looked at: it takes reading the disk for each, and what programs write
/// all the time would go before what people do.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct File {
    fit: Fit,
    /// When it was last opened, if it's known to have been.
    opened: Reverse<Option<String>>,
    depth: usize,
    length: usize,
    path: String,
    folder: bool,
}

impl File {
    fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or_default()
    }

    /// The folder it's in, with `~` for the home folder.
    fn location(&self) -> String {
        let folder = Path::new(&self.path).parent().unwrap_or(Path::new("/"));
        let home = glib::home_dir();
        match folder.strip_prefix(&home) {
            Ok(inside) if inside.as_os_str().is_empty() => String::from("~"),
            Ok(inside) => format!("~/{}", inside.display()),
            Err(_) => folder.display().to_string(),
        }
    }

    fn json(&self) -> Value {
        let icon = if self.folder {
            Some("folder".into())
        } else {
            let (kind, _) = gio::content_type_guess(Some(self.name()), None);
            // The kind's own icon is only in some themes.
            gio::content_type_get_generic_icon_name(&kind)
        };
        json!({
            "path": self.path,
            "name": self.name(),
            "location": self.location(),
            "folder": self.folder,
            "icon": icon.as_deref().unwrap_or("text-x-generic"),
        })
    }
}

/// The `most` files that fit `query` best among `paths`, which each end with a zero byte, a
/// folder's after a slash, the best first.
#[cfg(test)]
fn best(
    paths: impl BufRead,
    query: &Query,
    recent: &HashMap<PathBuf, String>,
    most: usize,
) -> Vec<File> {
    let mut best = BinaryHeap::with_capacity(most + 1);
    keep_best(&mut best, paths, query, recent, most);
    best.into_sorted_vec()
}

/// Keeps in `best` the `most` files that fit `query` best among what's there and `paths`, which
/// each end with a zero byte, a folder's after a slash. The worst of them is on top.
fn keep_best(
    best: &mut BinaryHeap<File>,
    paths: impl BufRead,
    query: &Query,
    recent: &HashMap<PathBuf, String>,
    most: usize,
) {
    for path in paths.split(0) {
        let Ok(path) = path else { break };
        // A path that isn't text can't be said to the shell.
        let Ok(mut path) = String::from_utf8(path) else {
            continue;
        };
        let folder = path.ends_with('/');
        if folder {
            path.pop();
        }
        let Some(name) = path.rsplit('/').next() else {
            continue;
        };
        let Some(fit) = query.fit(name, folder) else {
            continue;
        };
        best.push(File {
            fit,
            opened: Reverse(recent.get(Path::new(&path)).cloned()),
            depth: path.matches('/').count(),
            length: name.chars().count(),
            path,
            folder,
        });
        if best.len() > most {
            best.pop();
        }
    }
}

/// A folder as a setting writes it, with `~` at its start for the home folder, as a path: with
/// its links followed, so that a folder written two ways is one.
fn resolve(folder: &str) -> PathBuf {
    let home = glib::home_dir();
    let path = match folder.strip_prefix('~') {
        Some("") => home,
        Some(inside) if inside.starts_with('/') => home.join(inside.trim_start_matches('/')),
        _ => home.join(folder),
    };
    std::fs::canonicalize(&path).unwrap_or(path)
}

/// The folders to read, each with the folders in it to leave out, as paths from it, from the
/// folders asked for and those left out. A folder in one that's read is read with it, unless
/// one left out is between them: it's read on its own then. The most specific wins, so a folder
/// both asked for and left out is left out, and one left out that's in none read changes
/// nothing. Each is read once, and a folder that isn't there isn't read.
fn plan(folders: &[PathBuf], excluded: &[PathBuf]) -> Vec<(PathBuf, Vec<PathBuf>)> {
    let mut asked: Vec<&PathBuf> = folders.iter().filter(|f| !excluded.contains(f)).collect();
    asked.sort();
    asked.dedup();
    // Whether `outer`'s reading reaches `inner`, in it, with none left out between them.
    let reaches = |outer: &PathBuf, inner: &PathBuf| {
        outer != inner
            && inner.starts_with(outer)
            && !excluded
                .iter()
                .any(|e| e != outer && e.starts_with(outer) && inner.starts_with(e))
    };
    asked
        .iter()
        .filter(|folder| !asked.iter().any(|outer| reaches(outer, folder)))
        .filter(|folder| folder.is_dir())
        .map(|folder| {
            let mut left_out: Vec<PathBuf> = excluded
                .iter()
                .filter(|e| *e != *folder)
                .filter_map(|e| e.strip_prefix(folder).ok())
                .map(Path::to_path_buf)
                .collect();
            left_out.sort();
            left_out.dedup();
            ((*folder).clone(), left_out)
        })
        .collect()
}

/// `fd`'s pattern for the folder at `inside` from the one it reads, which only that matches: from
/// its start, with what patterns mean escaped.
fn exclude_pattern(inside: &Path) -> String {
    let mut pattern = String::from("/");
    for c in inside.to_string_lossy().chars() {
        if matches!(c, '\\' | '*' | '?' | '[' | ']' | '{' | '}' | '!') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern
}

/// The files apps said they opened, and when, in UTC as ISO 8601, which sorts as it reads. It's
/// the list GTK keeps, read here rather than by GTK, which needs a display this program hasn't.
fn read_recent(xbel: &str) -> HashMap<PathBuf, String> {
    let mut recent = HashMap::new();
    let mut reader = Reader::from_str(xbel);
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag) | Event::Empty(tag)) if tag.name().as_ref() == b"bookmark" => {
                let read = |name: &str| {
                    let attribute = tag.try_get_attribute(name).ok()??;
                    let value = attribute.normalized_value(XmlVersion::Implicit1_0).ok()?;
                    Some(value.into_owned())
                };
                let path = read("href").and_then(|uri| glib::filename_from_uri(&uri).ok());
                if let Some(((path, _), opened)) = path.zip(read("modified")) {
                    recent.insert(path, opened);
                }
            }
            Ok(Event::Eof) | Err(_) => return recent,
            Ok(_) => (),
        }
    }
}

/// Opens the file at `path` with the app that opens its kind, which can bring its window forward
/// with the activation `token`.
pub async fn open(path: &Path, token: Option<&str>) -> Result<(), glib::Error> {
    let file = gio::File::for_path(path);
    let uri = file.uri();
    let app = file
        .query_default_handler_future(glib::Priority::DEFAULT)
        .await?;
    let failed = |what: &str| glib::Error::new(gio::IOErrorEnum::NotSupported, what);
    let app = app
        .downcast::<gio::DesktopAppInfo>()
        .map_err(|_| failed("its app has no desktop entry"))?;

    // GIO starts such an app without the token, so it's asked here, the way GIO would.
    let id = app.id().unwrap_or_default();
    if let Some(name) = id
        .strip_suffix(".desktop")
        .filter(|_| app.boolean("DBusActivatable"))
    {
        let object = format!("/{}", name.replace('.', "/").replace('-', "_"));
        let data = VariantDict::new(None);
        if let Some(token) = token {
            data.insert("activation-token", token);
            data.insert("desktop-startup-id", token);
        }
        let args = Variant::tuple_from_iter([vec![uri.as_str()].to_variant(), data.end()]);
        let bus = gio::bus_get_future(BusType::Session).await?;
        bus.call_future(
            Some(name),
            &object,
            "org.freedesktop.Application",
            "Open",
            Some(&args),
            None,
            DBusCallFlags::NONE,
            -1,
        )
        .await?;
        return Ok(());
    }

    let context = gio::AppLaunchContext::new();
    if let Some(token) = token {
        context.setenv("XDG_ACTIVATION_TOKEN", token);
        context.setenv("DESKTOP_STARTUP_ID", token);
    }
    // What the app prints stays out of what this program says to bento.
    let null = std::fs::File::options()
        .write(true)
        .open("/dev/null")
        .map_err(|err| failed(&err.to_string()))?;
    app.launch_uris_as_manager_with_fds(
        &[uri.as_str()],
        Some(&context),
        glib::SpawnFlags::SEARCH_PATH,
        None,
        None,
        None::<std::fs::File>,
        Some(&null),
        Some(&null),
    )
}

/// Shows the file at `path` in its folder, in the file manager, which can bring its window
/// forward with the activation `token`.
pub async fn show(path: &Path, token: Option<&str>) -> Result<(), glib::Error> {
    let uri = gio::File::for_path(path).uri();
    let bus = gio::bus_get_future(BusType::Session).await?;
    let args = (vec![uri.as_str()], token.unwrap_or_default()).to_variant();
    bus.call_future(
        Some("org.freedesktop.FileManager1"),
        "/org/freedesktop/FileManager1",
        "org.freedesktop.FileManager1",
        "ShowItems",
        Some(&args),
        None,
        DBusCallFlags::NONE,
        -1,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(query: &str, paths: &[&str], recent: &HashMap<PathBuf, String>) -> Vec<String> {
        let query = Query::new(query).unwrap();
        let listed: String = paths.iter().map(|path| format!("{path}\0")).collect();
        best(listed.as_bytes(), &query, recent, 4)
            .into_iter()
            .map(|file| file.path)
            .collect()
    }

    #[test]
    fn it_ranks_files_by_how_their_names_fit() {
        let paths = [
            "/h/preport",
            "/h/a/my-report.txt",
            "/h/a/b/reporting.txt",
            "/h/a/b/c/Report.pdf",
        ];
        let ranked = [paths[3], paths[2], paths[1], paths[0]];
        assert_eq!(found("report", &paths, &HashMap::new()), ranked);
        // Only as many as asked for are kept, whatever order they come in.
        let mut more = vec!["/h/xreport"; 9];
        more.extend(paths);
        assert_eq!(found("report", &more, &HashMap::new()), ranked);
    }

    #[test]
    fn what_fits_alike_goes_by_when_it_was_opened_then_by_where_it_is() {
        let paths = [
            "/h/a/b/notes.md",
            "/h/notes.org",
            "/h/a/notes/",
            "/h/old/x/y/notes.md",
        ];
        let recent = HashMap::from([
            (
                PathBuf::from(paths[3]),
                String::from("2026-01-01T00:00:00Z"),
            ),
            (
                PathBuf::from(paths[0]),
                String::from("2026-02-01T00:00:00Z"),
            ),
        ]);
        assert_eq!(
            found("notes", &paths, &recent),
            [paths[0], paths[3], "/h/notes.org", "/h/a/notes"]
        );
    }

    #[test]
    fn it_fits_names_to_the_words_of_a_query() {
        let fit = |query: &str, name: &str| Query::new(query).unwrap().fit(name, false);
        assert_eq!(fit("engine design", "engine-design.md"), Some(Fit::Whole));
        assert_eq!(fit("engine-des", "Engine Design.md"), None);
        assert_eq!(fit("engine des", "engine-design.md"), Some(Fit::Start));
        assert_eq!(fit("des eng", "engine-design.md"), Some(Fit::Words));
        assert_eq!(fit("sign gin", "engine-design.md"), Some(Fit::Within));
        assert_eq!(fit("design shell", "engine-design.md"), None);
        assert_eq!(fit("--", "a--b"), Some(Fit::Within));
        // A dot in a folder's name starts no extension.
        let folder = |name: &str| Query::new("v1").unwrap().fit(name, true);
        assert_eq!(folder("v1.2"), Some(Fit::Start));
        // `fd` takes the longest word, and what no name holds isn't looked for.
        assert_eq!(Query::new("my Report").unwrap().pattern(), "report");
        for text in ["", "  ", "a/b", "a\0b"] {
            assert!(Query::new(text).is_none());
        }
    }

    #[test]
    fn each_folder_is_read_once_and_the_most_specific_rule_wins() {
        let dir = std::env::temp_dir().join(format!("bento-plan-{}", std::process::id()));
        for folder in ["h/a/b", "h/games/saves", "h/x/y/z", "other"] {
            std::fs::create_dir_all(dir.join(folder)).unwrap();
        }
        let at = |path: &str| dir.join(path);
        let rel = |path: &str| PathBuf::from(path);
        let plan_of = |folders: &[&str], excluded: &[&str]| {
            let folders: Vec<_> = folders.iter().map(|f| at(f)).collect();
            let excluded: Vec<_> = excluded.iter().map(|f| at(f)).collect();
            plan(&folders, &excluded)
        };
        // A folder in another, or the same twice, is read with it.
        assert_eq!(plan_of(&["h"], &[]), [(at("h"), vec![])]);
        assert_eq!(plan_of(&["h/a", "h", "h"], &[]), [(at("h"), vec![])]);
        assert_eq!(
            plan_of(&["h/a", "other"], &[]),
            [(at("h/a"), vec![]), (at("other"), vec![])]
        );
        // One left out isn't read, but one asked for in it is, on its own.
        assert_eq!(
            plan_of(&["h", "h/games/saves"], &["h/games"]),
            [(at("h"), vec![rel("games")]), (at("h/games/saves"), vec![])]
        );
        // Asked for and left out, it's left out, even in a folder that's read.
        assert_eq!(plan_of(&["h/a"], &["h/a"]), []);
        assert_eq!(
            plan_of(&["h", "h/a"], &["h/a"]),
            [(at("h"), vec![rel("a")])]
        );
        // What's left out of nothing read changes nothing, and what isn't there isn't read.
        assert_eq!(plan_of(&["h"], &["other"]), [(at("h"), vec![])]);
        assert_eq!(plan_of(&["h", "gone"], &[]), [(at("h"), vec![])]);
        // Deeper ones left out are left out of each folder they're in.
        assert_eq!(
            plan_of(&["h", "h/x/y"], &["h/x", "h/x/y/z"]),
            [
                (at("h"), vec![rel("x"), rel("x/y/z")]),
                (at("h/x/y"), vec![rel("z")])
            ]
        );
        std::fs::remove_dir_all(&dir).unwrap();
        // A pattern matches only that folder, from the one read.
        assert_eq!(exclude_pattern(Path::new("games")), "/games");
        assert_eq!(exclude_pattern(Path::new("a [b]/*")), r"/a \[b\]/\*");
    }

    #[test]
    fn it_reads_when_files_were_opened() {
        let xbel = r#"<?xml version="1.0" encoding="UTF-8"?>
<xbel version="1.0" xmlns:bookmark="http://www.freedesktop.org/standards/desktop-bookmarks">
  <bookmark href="file:///h/a%20b&amp;c.txt" added="2026-01-01T00:00:00Z" modified="2026-03-01T10:00:00.5Z" visited="2026-01-01T00:00:00Z">
    <info><metadata owner="http://freedesktop.org"/></info>
  </bookmark>
  <bookmark href="https://example.com/" modified="2026-03-01T10:00:00Z"/>
</xbel>"#;
        let opened = HashMap::from([(
            PathBuf::from("/h/a b&c.txt"),
            String::from("2026-03-01T10:00:00.5Z"),
        )]);
        assert_eq!(read_recent(xbel), opened);
        assert!(read_recent("not xml <").is_empty());
    }
}
