//! Searching windows: bento's state, which layout engines present however they like.
//!
//! An engine starts a search over a group's windows, a workspace's, or every window it places.
//! bento then owns the query, which people type in a field that bento draws (see
//! `surfaces::search_field`) and the keyboard goes to, and the matching of the query against the
//! windows' titles and app ids, see [`matching`]. At most one search runs at a time. The engine is
//! told whenever the query or the matches change, and when the search ends: with Enter, which
//! picks the best match, with Escape, when the engine cancels it, when its scope is gone, or when
//! the session locks. A search also ends, without anyone being told, when the engine is reloaded.

use serde::Deserialize;
use serde_json::json;

use super::{tiling, Layout, LayoutElement};

mod field;
mod matching;

pub use self::field::{Edit, Field};
pub use self::matching::Rank;
use self::matching::{matching, Candidate};

/// What a search is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Scope {
    /// Every window the engine places.
    All,
    Workspace {
        id: u64,
    },
    Group {
        id: u64,
    },
}

/// A window that matches the query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    /// The window, by the id that the layout engine knows it by.
    pub window: u64,
    pub rank: Rank,
}

/// The running search.
#[derive(Debug)]
pub struct Search {
    scope: Scope,
    field: Field,
    /// The windows in scope that match the field's text, in no particular order: the layout
    /// engine ranks the ones that match equally in its own order.
    hits: Vec<Hit>,
}

/// A window in scope, as matching sees it.
struct Windowed {
    window: u64,
    title: Option<String>,
    app_id: Option<String>,
}

impl Search {
    pub fn new(scope: Scope) -> Self {
        Self {
            scope,
            field: Field::default(),
            hits: Vec::new(),
        }
    }

    pub fn field(&self) -> &Field {
        &self.field
    }

    pub fn hits(&self) -> &[Hit] {
        &self.hits
    }
}

impl<W: LayoutElement> Layout<W> {
    /// Whether a search is running, and has the keyboard.
    pub fn search_active(&self) -> bool {
        self.search.is_some()
    }

    /// The running search's field, for drawing and for screen readers.
    pub fn search_field(&self) -> Option<&Field> {
        self.search.as_ref().map(|search| &search.field)
    }

    /// The selected text of the running search's field.
    pub fn search_selection(&self) -> Option<String> {
        self.search_field()?.selected_text().map(str::to_owned)
    }

    /// Types in the running search's field.
    pub fn search_edit(&mut self, edit: &Edit) {
        let Some(search) = &mut self.search else {
            return;
        };
        search.field.apply(edit);
        self.flush_engine();
    }

    /// Ends the running search with its best match picked, which the engine is asked to activate.
    pub fn search_submit(&mut self) {
        if let Some(window) = self.end_search(true) {
            self.activate_window_inner(&window);
        }
        self.flush_engine();
    }

    /// Ends the running search without picking a window.
    pub fn search_cancel(&mut self) {
        self.end_search(false);
        self.flush_engine();
    }

    /// Starts a search, which the engine asked for. A scope that isn't there ends it again when it
    /// is next refreshed.
    pub(super) fn start_search(&mut self, scope: Scope) {
        self.search = Some(Search::new(scope));
    }

    /// Ends the search without telling the engine, which ended it.
    pub(super) fn forget_search(&mut self) {
        self.search = None;
    }

    /// Makes the matches current: windows may have opened, closed or changed their titles, and
    /// the search ends if its scope is gone.
    pub(super) fn refresh_search(&mut self) {
        let Some(search) = &self.search else {
            return;
        };
        let Some(windows) = self.search_windows(search.scope) else {
            self.search = None;
            tiling::host::queue(json!({ "type": "searchend", "pick": false }));
            return;
        };

        let candidates: Vec<Candidate> = windows
            .iter()
            .map(|win| Candidate {
                window: win.window,
                title: win.title.as_deref(),
                app_id: win.app_id.as_deref(),
            })
            .collect();
        let hits = matching(search.field.text(), &candidates);
        if let Some(search) = &mut self.search {
            search.hits = hits;
        }
    }

    /// The windows a search over `scope` covers, or `None` if there's no such group or workspace.
    fn search_windows(&self, scope: Scope) -> Option<Vec<Windowed>> {
        let windowed = |tile: &crate::layout::tile::Tile<W>| Windowed {
            window: tile.engine_id,
            title: tile.window().title(),
            app_id: tile.window().app_id(),
        };

        match scope {
            Scope::All => Some(
                self.workspaces()
                    .flat_map(|(_, _, ws)| ws.scrolling().tiles())
                    .map(windowed)
                    .collect(),
            ),
            Scope::Workspace { id } => self
                .workspaces()
                .find(|(_, _, ws)| ws.id().get() == id)
                .map(|(_, _, ws)| ws.scrolling().tiles().map(windowed).collect()),
            Scope::Group { id } => {
                let group = self.groups.get(id)?;
                let tiles: Vec<_> = self
                    .workspaces()
                    .flat_map(|(_, _, ws)| ws.scrolling().tiles())
                    .collect();
                Some(
                    group
                        .windows
                        .iter()
                        .filter_map(|id| tiles.iter().find(|tile| tile.engine_id == *id))
                        .map(|tile| windowed(tile))
                        .collect(),
                )
            }
        }
    }

    /// Gives every workspace's tree the field that its search nodes draw.
    pub(super) fn sync_search_field(&mut self) {
        let field = self.search.as_ref().map(|search| search.field.clone());
        for ws in self.workspaces_mut() {
            ws.scrolling_mut()
                .tree_mut()
                .set_search_field(field.as_ref());
        }
    }
}
