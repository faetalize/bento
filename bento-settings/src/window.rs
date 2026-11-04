//! The window: a sidebar of categories with search on top, and the chosen page beside it.
//!
//! Search results replace the category list in the sidebar, so search works the same when the
//! window is narrow and shows one pane at a time. Picking a result opens its page and flashes the
//! setting.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::model::{Category, Content, Section};
use crate::page::{self, PageContext};
use crate::search::{self, Entry};
use crate::store::Store;

pub struct Ui {
    categories: Vec<&'static Category>,
    /// Section index of each category.
    section_of: Vec<usize>,
    sections: &'static [Section],
    entries: Vec<Entry>,
    ctx: PageContext,

    split: adw::NavigationSplitView,
    sidebar_stack: gtk::Stack,
    category_list: gtk::ListBox,
    category_rows: Vec<gtk::ListBoxRow>,
    results_list: gtk::ListBox,
    /// Entry index for each row in `results_list`.
    result_entries: RefCell<Vec<usize>>,
    search_entry: gtk::SearchEntry,
    content_page: adw::NavigationPage,
    pages: gtk::Stack,
    banner: adw::Banner,

    built: RefCell<HashSet<usize>>,
    current: Cell<usize>,
    /// Set while the app selects a sidebar row itself.
    selecting: Cell<bool>,
}

pub fn build(
    app: &adw::Application,
    sections: &'static [Section],
    store: Store,
) -> (adw::ApplicationWindow, Rc<Ui>) {
    let categories: Vec<&'static Category> = sections.iter().flat_map(|s| &s.categories).collect();
    let section_of = sections
        .iter()
        .enumerate()
        .flat_map(|(idx, s)| std::iter::repeat_n(idx, s.categories.len()))
        .collect();
    let entries = search::index(sections);

    let toasts = adw::ToastOverlay::new();
    let ctx = PageContext::new(store.clone(), toasts.clone());

    // Sidebar.
    let search_entry = gtk::SearchEntry::builder()
        .placeholder_text("Search")
        .hexpand(true)
        .build();

    let category_list = gtk::ListBox::builder()
        .css_classes(["navigation-sidebar"])
        .build();
    let mut category_rows = Vec::new();
    for category in &categories {
        let row = sidebar_row(category);
        category_list.append(&row);
        category_rows.push(row);
    }

    let results_list = gtk::ListBox::builder()
        .css_classes(["navigation-sidebar", "search-results"])
        .build();
    // A page of its own rather than the list's placeholder, which gets no height in a scrolled
    // list.
    let no_results = adw::StatusPage::builder()
        .icon_name("ph-magnifying-glass-symbolic")
        .title("No Results")
        .css_classes(["compact"])
        .build();

    let sidebar_stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    sidebar_stack.add_named(&scrolled(&category_list), Some("categories"));
    sidebar_stack.add_named(&scrolled(&results_list), Some("results"));
    sidebar_stack.add_named(&no_results, Some("no-results"));

    // Nothing else goes in this header bar: it centers its title, so a button on one side would
    // leave the same gap on the other.
    let sidebar_header = adw::HeaderBar::builder()
        .title_widget(&search_entry)
        .build();
    let sidebar_toolbar = adw::ToolbarView::new();
    sidebar_toolbar.add_top_bar(&sidebar_header);
    sidebar_toolbar.set_content(Some(&sidebar_stack));
    let sidebar_page = adw::NavigationPage::builder()
        .title("Bento Settings")
        .tag("sidebar")
        .child(&sidebar_toolbar)
        .build();

    // Content.
    let pages = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(120)
        .build();
    let banner = adw::Banner::new("");
    let content_toolbar = adw::ToolbarView::new();
    content_toolbar.add_top_bar(&adw::HeaderBar::new());
    content_toolbar.add_top_bar(&banner);
    content_toolbar.set_content(Some(&pages));
    let content_page = adw::NavigationPage::builder()
        .title(categories[0].title)
        .tag("content")
        .child(&content_toolbar)
        .build();

    let split = adw::NavigationSplitView::builder()
        .sidebar(&sidebar_page)
        .content(&content_page)
        .min_sidebar_width(230.)
        .max_sidebar_width(300.)
        .build();
    toasts.set_child(Some(&split));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Bento Settings")
        .default_width(1000)
        .default_height(720)
        .width_request(360)
        .height_request(400)
        .content(&toasts)
        .build();

    let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        640.,
        adw::LengthUnit::Sp,
    ));
    breakpoint.add_setter(&split, "collapsed", Some(&true.to_value()));
    window.add_breakpoint(breakpoint);

    let ui = Rc::new(Ui {
        categories,
        section_of,
        sections,
        entries,
        ctx,
        split,
        sidebar_stack,
        category_list,
        category_rows,
        results_list,
        result_entries: RefCell::new(Vec::new()),
        search_entry,
        content_page,
        pages,
        banner,
        built: RefCell::new(HashSet::new()),
        current: Cell::new(0),
        selecting: Cell::new(false),
    });

    connect(&ui, &window);
    ui.show(0, None);
    update_banner(&ui);
    {
        let weak = Rc::downgrade(&ui);
        store.subscribe(move || {
            if let Some(ui) = weak.upgrade() {
                update_banner(&ui);
            }
        });
    }

    // Write anything still pending before the app quits.
    window.connect_close_request(move |_| {
        store.flush();
        glib::Propagation::Proceed
    });

    (window, ui)
}

fn connect(ui: &Rc<Ui>, window: &adw::ApplicationWindow) {
    // Section titles above the first category of each section.
    {
        let ui_weak = Rc::downgrade(ui);
        ui.category_list.set_header_func(move |row, before| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let section = ui.section_of[row.index() as usize];
            let first =
                before.is_none_or(|before| ui.section_of[before.index() as usize] != section);
            if let Some(title) = ui.sections[section].title.filter(|_| first) {
                let label = gtk::Label::builder()
                    .label(title)
                    .xalign(0.)
                    .css_classes(["heading", "dim-label", "sidebar-heading"])
                    .build();
                row.set_header(Some(&label));
            } else {
                row.set_header(None::<&gtk::Widget>);
            }
        });
    }

    {
        let ui_weak = Rc::downgrade(ui);
        ui.category_list.connect_row_selected(move |_, row| {
            let (Some(ui), Some(row)) = (ui_weak.upgrade(), row) else {
                return;
            };
            if !ui.selecting.get() {
                ui.show(row.index() as usize, None);
            }
        });
    }
    {
        // Selecting an already selected row doesn't emit row-selected, but narrow windows still
        // need to open the page.
        let ui_weak = Rc::downgrade(ui);
        ui.category_list.connect_row_activated(move |_, row| {
            if let Some(ui) = ui_weak.upgrade() {
                ui.show(row.index() as usize, None);
            }
        });
    }

    {
        let ui_weak = Rc::downgrade(ui);
        ui.search_entry.connect_search_changed(move |entry| {
            if let Some(ui) = ui_weak.upgrade() {
                ui.search(&entry.text());
            }
        });
    }
    {
        let ui_weak = Rc::downgrade(ui);
        ui.search_entry.connect_stop_search(move |entry| {
            entry.set_text("");
            if let Some(ui) = ui_weak.upgrade() {
                ui.category_rows[ui.current.get()].grab_focus();
            }
        });
    }
    {
        // Enter opens the best result.
        let ui_weak = Rc::downgrade(ui);
        ui.search_entry.connect_activate(move |_| {
            if let Some(ui) = ui_weak.upgrade() {
                let first = ui.result_entries.borrow().first().copied();
                if let Some(entry) = first {
                    ui.open_result(entry);
                }
            }
        });
    }
    {
        let ui_weak = Rc::downgrade(ui);
        ui.results_list.connect_row_activated(move |_, row| {
            if let Some(ui) = ui_weak.upgrade() {
                let entry = ui
                    .result_entries
                    .borrow()
                    .get(row.index() as usize)
                    .copied();
                if let Some(entry) = entry {
                    ui.open_result(entry);
                }
            }
        });
    }
    ui.search_entry.set_key_capture_widget(Some(window));

    // Window actions.
    let search = gio::ActionEntry::builder("search")
        .activate({
            let ui_weak = Rc::downgrade(ui);
            move |_: &adw::ApplicationWindow, _, _| {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.split.set_show_content(false);
                    ui.search_entry.grab_focus();
                }
            }
        })
        .build();
    let reload = gio::ActionEntry::builder("reload")
        .activate({
            let ui_weak = Rc::downgrade(ui);
            move |_: &adw::ApplicationWindow, _, _| {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.ctx.store.reload();
                    ui.ctx.toast("Reloaded the config");
                }
            }
        })
        .build();
    window.add_action_entries([search, reload]);
    if let Some(app) = window.application() {
        app.set_accels_for_action("win.search", &["<Control>f"]);
        app.set_accels_for_action("win.reload", &["<Control>r"]);
        app.set_accels_for_action("window.close", &["<Control>w"]);
    }

    {
        let ui_weak = Rc::downgrade(ui);
        ui.banner.connect_button_clicked(move |banner| {
            if let Some(ui) = ui_weak.upgrade() {
                banner_action(&ui, banner);
            }
        });
    }
}

impl Ui {
    /// Opens the page with this id. Returns false if there's none.
    pub fn open_page(self: &Rc<Self>, id: &str) -> bool {
        match self.categories.iter().position(|c| c.id == id) {
            Some(idx) => {
                self.show(idx, None);
                true
            }
            None => false,
        }
    }

    pub fn start_search(&self, query: &str) {
        self.split.set_show_content(false);
        self.search_entry.set_text(query);
        self.search_entry.grab_focus();
        self.search_entry.set_position(-1);
    }

    /// Opens a category's page, and flashes the setting at `anchor` if given.
    fn show(self: &Rc<Self>, idx: usize, anchor: Option<String>) {
        let category = self.categories[idx];
        if self.built.borrow_mut().insert(idx) {
            let content = match &category.content {
                Content::Settings(groups) => page::frame(&page::settings_page(groups, &self.ctx)),
                Content::Custom { build, .. } => page::frame(&build(&self.ctx)),
                Content::Planned => unreachable!("planned pages stay out of the sidebar"),
            };
            self.pages.add_named(&content, Some(category.id));
        }
        self.pages.set_visible_child_name(category.id);
        self.content_page.set_title(category.title);
        self.current.set(idx);

        self.selecting.set(true);
        self.category_list
            .select_row(Some(&self.category_rows[idx]));
        self.selecting.set(false);
        self.split.set_show_content(true);

        if let Some(anchor) = anchor {
            // Wait for the page to be laid out before scrolling to the setting.
            let ui = Rc::downgrade(self);
            glib::timeout_add_local_once(Duration::from_millis(80), move || {
                if let Some(ui) = ui.upgrade() {
                    if let Some(widget) = ui.ctx.find_anchor(&anchor) {
                        reveal(&widget);
                    }
                }
            });
        }
    }

    fn search(self: &Rc<Self>, query: &str) {
        let query = query.trim();
        if query.is_empty() {
            self.sidebar_stack.set_visible_child_name("categories");
            self.result_entries.borrow_mut().clear();
            return;
        }

        let hits = search::search(&self.entries, query);
        self.results_list.remove_all();
        let mut result_entries = self.result_entries.borrow_mut();
        result_entries.clear();
        for hit in hits.iter().take(50) {
            let entry = &self.entries[hit.entry];
            self.results_list.append(&result_row(
                self.categories[entry.category],
                entry,
                &hit.positions,
            ));
            result_entries.push(hit.entry);
        }
        self.sidebar_stack
            .set_visible_child_name(if hits.is_empty() {
                "no-results"
            } else {
                "results"
            });
    }

    fn open_result(self: &Rc<Self>, entry: usize) {
        let entry = &self.entries[entry];
        let (category, anchor) = (entry.category, entry.anchor.clone());
        self.search_entry.set_text("");
        self.show(category, anchor);
    }
}

/// Scrolls a widget into view and flashes it, opening the expander row it's in first.
fn reveal(widget: &gtk::Widget) {
    let expander = widget
        .parent()
        .and_then(|parent| parent.ancestor(adw::ExpanderRow::static_type()))
        .and_downcast::<adw::ExpanderRow>();
    if let Some(expander) = expander.filter(|e| !e.is_expanded() && e.enables_expansion()) {
        expander.set_expanded(true);
        // Wait for the rows to slide open, so the scroll lands where they end up.
        let widget = widget.downgrade();
        glib::timeout_add_local_once(Duration::from_millis(300), move || {
            if let Some(widget) = widget.upgrade() {
                scroll_and_flash(&widget);
            }
        });
    } else {
        scroll_and_flash(widget);
    }
}

fn scroll_and_flash(widget: &gtk::Widget) {
    if let Some(scrolled) = widget
        .ancestor(gtk::ScrolledWindow::static_type())
        .and_downcast::<gtk::ScrolledWindow>()
    {
        if let Some(child) = scrolled.child() {
            if let Some(point) = widget.compute_point(&child, &gtk::graphene::Point::new(0., 0.)) {
                let adjustment = scrolled.vadjustment();
                let target = f64::from(point.y()) - adjustment.page_size() / 3.;
                adjustment.set_value(target.clamp(adjustment.lower(), adjustment.upper()));
            }
        }
    }
    widget.grab_focus();

    widget.remove_css_class("search-highlight");
    widget.add_css_class("search-highlight");
    let widget = widget.downgrade();
    glib::timeout_add_local_once(Duration::from_millis(1600), move || {
        if let Some(widget) = widget.upgrade() {
            widget.remove_css_class("search-highlight");
        }
    });
}

fn update_banner(ui: &Ui) {
    let status = ui.ctx.store.status();
    let title = if status.write_error.is_some() {
        "Couldn't save the last change"
    } else if status.config_error.is_some() {
        "The config has an error"
    } else {
        ""
    };
    ui.banner.set_title(title);
    ui.banner.set_button_label(Some("Details"));
    ui.banner.set_revealed(!title.is_empty());
}

fn banner_action(ui: &Ui, banner: &adw::Banner) {
    let status = ui.ctx.store.status();
    let details = status
        .write_error
        .or(status.config_error)
        .unwrap_or_default();
    let text = gtk::Label::builder()
        .label(&details)
        .selectable(true)
        .wrap(true)
        .xalign(0.)
        .css_classes(["monospace"])
        .build();
    let scrolled = gtk::ScrolledWindow::builder()
        .child(&text)
        .min_content_height(160)
        .max_content_height(360)
        .propagate_natural_height(true)
        .build();
    let dialog = adw::AlertDialog::builder()
        .heading("Config Error")
        .extra_child(&scrolled)
        .build();
    dialog.add_response("open", "Open config.kdl");
    dialog.add_response("close", "Close");
    dialog.set_default_response(Some("close"));
    let path = ui.ctx.store.config_path().to_owned();
    dialog.connect_response(Some("open"), move |dialog, _| {
        page::open_file(dialog, &path)
    });
    dialog.present(Some(banner));
}

fn sidebar_row(category: &Category) -> gtk::ListBoxRow {
    if let Some(build) = category.sidebar_row {
        return gtk::ListBoxRow::builder().child(&build()).build();
    }
    let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row_box.append(&gtk::Image::from_icon_name(category.icon));
    row_box.append(
        &gtk::Label::builder()
            .label(category.title)
            .xalign(0.)
            .build(),
    );
    gtk::ListBoxRow::builder().child(&row_box).build()
}

fn result_row(category: &Category, entry: &Entry, positions: &[usize]) -> gtk::ListBoxRow {
    let title = gtk::Label::builder()
        .use_markup(true)
        .label(highlight(&entry.title, positions))
        .xalign(0.)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    let context = gtk::Label::builder()
        .label(&entry.context)
        .xalign(0.)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["caption", "dim-label"])
        .build();
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_valign(gtk::Align::Center);
    text.append(&title);
    text.append(&context);

    let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row_box.append(&gtk::Image::from_icon_name(category.icon));
    row_box.append(&text);
    gtk::ListBoxRow::builder().child(&row_box).build()
}

/// Bolds the matched characters of a title, as Pango markup.
fn highlight(text: &str, positions: &[usize]) -> String {
    let mut out = String::new();
    let mut bold = false;
    for (idx, c) in text.chars().enumerate() {
        let matched = positions.binary_search(&idx).is_ok();
        if matched != bold {
            out.push_str(if matched { "<b>" } else { "</b>" });
            bold = matched;
        }
        out.push_str(&glib::markup_escape_text(&c.to_string()));
    }
    if bold {
        out.push_str("</b>");
    }
    out
}

fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(child)
        .build()
}
