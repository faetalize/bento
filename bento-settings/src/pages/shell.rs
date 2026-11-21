//! Which shell bento draws, and its settings.
//!
//! Which shell runs comes from the config, chosen like the engine: one of bento's own, or a file
//! of one's own. bento's own aren't edited in place; one is copied into a file of one's own to
//! start from. The settings come from the running shell over IPC, since its modules and the shell
//! itself declare them, and are written to `shell/settings.kdl` next to the config, a block for
//! each module and shell by its name.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use bento_ipc::Shell;
use gtk::glib;
use niri_config::{Action, ShellScript};

use super::settings_file;
use crate::ipc;
use crate::kdl::Value;
use crate::page::PageContext;
use crate::store::tilde;

/// bento's own shells, as the config names them and as the app shows them.
const BUILT_IN: &[(&str, &str)] = &[("unix", "Unix"), ("windows", "Windows")];

struct View {
    ctx: PageContext,
    column: gtk::Box,
    /// The configured shell, the running one with its setting values left out, and how many of
    /// its actions have a bind. The page is rebuilt only when this changes; the settings'
    /// rows keep themselves up to date.
    shown: RefCell<Option<Shape>>,
}

/// The configured shell, the running one, how many of its actions have a bind, and a shell
/// file's settings file with whether it's there.
type Shape = (
    ShellScript,
    Result<Shell, String>,
    Option<String>,
    Option<(PathBuf, bool)>,
);

fn shape(ctx: &PageContext, shell: &Result<Shell, String>) -> Shape {
    let binds = shell
        .as_ref()
        .ok()
        .and_then(|shell| binds_summary(ctx, shell));
    let shell = shell.clone().map(|mut shell| {
        for block in &mut shell.settings {
            for setting in &mut block.settings {
                setting.value = setting.default.clone();
            }
        }
        shell
    });
    let script = script(ctx);
    let settings = match script {
        ShellScript::File(_) => ctx.store.shell_path().map(|path| {
            let exists = path.exists();
            (path, exists)
        }),
        ShellScript::BuiltIn(_) => None,
    };
    (script, shell, binds, settings)
}

fn script(ctx: &PageContext) -> ShellScript {
    let config = ctx.store.config();
    config.shell.clone().unwrap_or_default().script()
}

/// The groups that choose the shell and set its settings.
pub fn build(ctx: &PageContext) -> gtk::Widget {
    let view = Rc::new(View {
        ctx: ctx.clone(),
        column: gtk::Box::new(gtk::Orientation::Vertical, 24),
        shown: RefCell::new(None),
    });
    refresh(&view);

    // bento reads the config and the settings a moment after a write, so ask again a few times.
    let weak = Rc::downgrade(&view);
    ctx.store.subscribe(move || {
        for delay in [200, 700, 2000] {
            let weak = weak.clone();
            glib::timeout_add_local_once(Duration::from_millis(delay), move || {
                if let Some(view) = weak.upgrade() {
                    refresh(&view);
                }
            });
        }
    });

    let column = view.column.clone();
    crate::page::keep_alive(&column, view);
    column.upcast()
}

fn refresh(view: &Rc<View>) {
    let shell = ipc::shell();
    let shape = shape(&view.ctx, &shell);
    if view.shown.borrow().as_ref() == Some(&shape) {
        return;
    }

    while let Some(child) = view.column.first_child() {
        view.column.remove(&child);
    }
    view.column.append(&choice_group(view, shape.2.as_deref()));
    if let Some((path, _)) = &shape.3 {
        if let Some(missing) = settings_file::missing(settings_file::Kind::Shell, path) {
            view.column.append(&missing);
        }
    }
    match &shell {
        Ok(shell) => {
            if let Some(error) = &shell.error {
                view.column.append(&problem_group(
                    "Shell Failed",
                    &format!("The last shell that loaded stays on screen.\n{error}"),
                ));
            }
            if let Some(error) = &shell.settings_error {
                view.column.append(&problem_group(
                    "Settings File Has an Error",
                    &format!("The settings it last had stay until it's fixed.\n{error}"),
                ));
            }
            // The settings of the shell and its modules, but those they put on another page.
            let settings = crate::pages::placed(&view.ctx, "shell");
            view.ctx.anchor("shell.settings", &settings);
            view.column.append(&settings);
            if !shell.unused.is_empty() {
                view.column.append(&problem_group(
                    "Not Used",
                    &format!(
                        "The settings file sets what this shell doesn't use. It stays in the \
                         file, for a shell that does: {}",
                        shell.unused.join(", ")
                    ),
                ));
            }
        }
        Err(err) => view
            .column
            .append(&problem_group("Couldn't Get the Shell's Settings", err)),
    }
    view.shown.replace(Some(shape));
}

/// How many actions the shell and its modules have and how many have a bind, or nothing when
/// they have none.
fn binds_summary(ctx: &PageContext, shell: &Shell) -> Option<String> {
    if shell.actions.is_empty() {
        return None;
    }
    // With its default binds and the user's, as bento settles which runs each chord.
    let declared: Vec<_> = shell.actions.iter().map(|a| a.action.clone()).collect();
    let mut defaults = niri_config::bind_set::bento_defaults().to_vec();
    defaults.extend(niri_config::bind_set::declared_binds(
        &declared,
        niri_config::bind_set::shell_action,
    ));
    let resolved = {
        let (user, unbound) = ctx.store.user_binds();
        niri_config::bind_set::resolve(&user, &unbound, &defaults)
    };
    let bound = declared
        .iter()
        .filter(|action| {
            resolved.in_use.iter().any(|bind| {
                matches!(
                    niri_config::bind_set::action_of(bind),
                    Some(Action::ShellAction(name, ..)) if *name == action.name
                )
            })
        })
        .count();
    let total = declared.len();
    Some(format!(
        "This shell has {total} {}, {bound} with a bind",
        if total == 1 { "action" } else { "actions" }
    ))
}

/// The shell's binds summed up, with a link to the Binds page's Shell tab, which lists them by
/// module.
fn binds_row(view: &Rc<View>, summary: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .use_markup(false)
        .title("Binds")
        .subtitle(summary)
        .activatable(true)
        .build();
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    row.connect_activated(|row| {
        let _ = row.activate_action("win.open-page", Some(&"binds#binds.shell".to_variant()));
    });
    view.ctx.anchor("shell.actions", &row);
    row
}

fn set_script(ctx: &PageContext, script: String) {
    ctx.store.edit("shell.script", move |doc| {
        doc.set(&["shell", "script"], vec![Value::Str(script)]);
    });
}

/// Runs the shell file at `script`, with its settings file at `settings`.
fn set_script_file(ctx: &PageContext, script: &Path, settings: &Path) {
    let (script, settings) = (tilde(script), tilde(settings));
    ctx.store.edit("shell.script", move |doc| {
        let properties = vec![(String::from("settings"), Value::Str(settings))];
        doc.set_node(&["shell", "script"], vec![Value::Str(script)], properties);
    });
}

/// The shell chosen, its file, and its binds summed up last, when it has any actions.
fn choice_group(view: &Rc<View>, binds: Option<&str>) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    view.ctx.anchor("shell.choice", &group);

    let configured = script(&view.ctx);
    let mut labels: Vec<&str> = BUILT_IN.iter().map(|(_, label)| *label).collect();
    labels.push("Custom File");
    let custom = BUILT_IN.len() as u32;
    let selected = match &configured {
        ShellScript::BuiltIn(name) => BUILT_IN
            .iter()
            .position(|(n, _)| n == name)
            .map_or(gtk::INVALID_LIST_POSITION, |i| i as u32),
        ShellScript::File(_) => custom,
    };
    let combo = adw::ComboRow::builder()
        .use_markup(false)
        .title("Shell")
        .model(&gtk::StringList::new(&labels))
        .selected(selected)
        .build();
    {
        let view = Rc::downgrade(view);
        let has_file = matches!(configured, ShellScript::File(_));
        combo.connect_selected_notify(move |combo| {
            let Some(view) = view.upgrade() else { return };
            let idx = combo.selected();
            if let Some((name, _)) = BUILT_IN.get(idx as usize) {
                set_script(&view.ctx, (*name).to_owned());
            } else if idx == custom && !has_file {
                choose_file(&view, combo);
            }
        });
    }
    group.add(&combo);

    match configured {
        ShellScript::File(path) => {
            let row = adw::ActionRow::builder()
                .use_markup(false)
                .title("File")
                .subtitle(path.to_string_lossy())
                .activatable(true)
                .build();
            let edit = gtk::Button::builder()
                .icon_name("document-edit-symbolic")
                .tooltip_text("Edit")
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .build();
            let full = expand_home(&path);
            edit.connect_clicked(move |button| crate::page::open_file(button, &full));
            row.add_suffix(&edit);
            row.add_suffix(&gtk::Image::from_icon_name("ph-folder-open-symbolic"));
            let weak = Rc::downgrade(view);
            row.connect_activated(move |row| {
                if let Some(view) = weak.upgrade() {
                    choose_file(&view, row);
                }
            });
            group.add(&row);

            // Its settings file, which picks another, the shell staying.
            let row = settings_file::row(view.ctx.store.shell_path().as_deref());
            let weak = Rc::downgrade(view);
            row.connect_activated(move |row| {
                let Some(view) = weak.upgrade() else { return };
                let ctx = view.ctx.clone();
                let script = expand_home(&path);
                let done = move |settings: PathBuf| set_script_file(&ctx, &script, &settings);
                let script = expand_home(&path);
                settings_file::ask(
                    &view.ctx,
                    row,
                    settings_file::Kind::Shell,
                    &script,
                    done,
                    || (),
                );
            });
            group.add(&row);
        }
        ShellScript::BuiltIn(name) => {
            let row = adw::ActionRow::builder()
                .use_markup(false)
                .title("Make a Copy")
                .subtitle(
                    "Copy the currently selected shell and use as a starting point to customize",
                )
                .activatable(true)
                .build();
            row.add_suffix(&gtk::Image::from_icon_name("edit-copy-symbolic"));
            let view = Rc::downgrade(view);
            row.connect_activated(move |row| {
                if let Some(view) = view.upgrade() {
                    make_copy(&view, row, &name);
                }
            });
            group.add(&row);
        }
    }
    if let Some(summary) = binds {
        group.add(&binds_row(view, summary));
    }
    group
}

fn expand_home(path: &Path) -> PathBuf {
    match path.strip_prefix("~") {
        Ok(rest) => glib::home_dir().join(rest),
        Err(_) => path.to_path_buf(),
    }
}

/// Copies bento's shell `name` to where the user saves it, from the copy bento keeps of its
/// shells, then asks where its settings come from, and runs the copy with them.
fn make_copy(view: &Rc<View>, widget: &impl IsA<gtk::Widget>, name: &str) {
    let source = glib::user_data_dir().join(format!("bento/shell/shells/{name}.tsx"));
    let text = match std::fs::read_to_string(&source) {
        Ok(text) => text,
        Err(err) => {
            view.ctx
                .toast(&format!("Couldn't read bento's {name} shell: {err}"));
            return;
        }
    };
    let dialog = gtk::FileDialog::builder()
        .title("Save a Copy of the Shell")
        .initial_name(format!("{name}.tsx"))
        .build();
    let weak = Rc::downgrade(view);
    let window = widget.root().and_downcast::<gtk::Window>();
    dialog.save(
        window.as_ref(),
        None::<&gtk::gio::Cancellable>,
        move |result| {
            let Some(view) = weak.upgrade() else { return };
            let Some(path) = result.ok().and_then(|file| file.path()) else {
                return;
            };
            if let Err(err) = std::fs::write(&path, &text) {
                view.ctx
                    .toast(&format!("Couldn't write {}: {err}", tilde(&path)));
                return;
            }
            let ctx = view.ctx.clone();
            let script = path.clone();
            let done = move |settings: PathBuf| {
                set_script_file(&ctx, &script, &settings);
                // Where an editor learns bento's API, which nothing sets up next to the copy.
                let toast = adw::Toast::builder()
                    .title(format!("Copied to {}", tilde(&script)))
                    .button_label("Editor Setup")
                    .timeout(8)
                    .build();
                toast.connect_button_clicked(|toast| {
                    let uri = "https://github.com/faetalize/bento#the-shell";
                    gtk::UriLauncher::new(uri).launch(
                        None::<&gtk::Window>,
                        None::<&gtk::gio::Cancellable>,
                        |_| (),
                    );
                    toast.dismiss();
                });
                ctx.toasts.add_toast(toast);
                crate::page::open_file(&ctx.toasts, &script);
            };
            settings_file::ask(
                &view.ctx,
                &view.column,
                settings_file::Kind::Shell,
                &path,
                done,
                || (),
            );
        },
    );
}

fn choose_file(view: &Rc<View>, widget: &impl IsA<gtk::Widget>) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Shells"));
    for pattern in ["*.tsx", "*.ts", "*.jsx", "*.js"] {
        filter.add_pattern(pattern);
    }
    let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title("Choose a Shell")
        .filters(&filters)
        .build();

    let view = Rc::downgrade(view);
    let window = widget.root().and_downcast::<gtk::Window>();
    dialog.open(
        window.as_ref(),
        None::<&gtk::gio::Cancellable>,
        move |result| {
            let Some(view) = view.upgrade() else { return };
            // Cancelled: rebuild, so the dropdown shows the configured shell again.
            let cancelled = {
                let view = Rc::downgrade(&view);
                move || {
                    if let Some(view) = view.upgrade() {
                        view.shown.replace(None);
                        refresh(&view);
                    }
                }
            };
            let Some(path) = result.ok().and_then(|file| file.path()) else {
                cancelled();
                return;
            };
            // Then where its settings come from; the shell changes only with them.
            let ctx = view.ctx.clone();
            let script = path.clone();
            let done = move |settings: PathBuf| set_script_file(&ctx, &script, &settings);
            settings_file::ask(
                &view.ctx,
                &view.column,
                settings_file::Kind::Shell,
                &path,
                done,
                cancelled,
            );
        },
    );
}

fn problem_group(title: &str, text: &str) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    let row = adw::ActionRow::builder()
        .use_markup(false)
        .title(title)
        .subtitle(text)
        .subtitle_selectable(true)
        .build();
    group.add(&row);
    group
}
