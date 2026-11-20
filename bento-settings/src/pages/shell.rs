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
    /// its actions have a shortcut. The page is rebuilt only when this changes; the settings'
    /// rows keep themselves up to date.
    shown: RefCell<Option<Shape>>,
}

type Shape = (ShellScript, Result<Shell, String>, Option<String>);

fn shape(ctx: &PageContext, shell: &Result<Shell, String>) -> Shape {
    let shortcuts = shell
        .as_ref()
        .ok()
        .and_then(|shell| shortcuts_summary(ctx, shell));
    let shell = shell.clone().map(|mut shell| {
        for block in &mut shell.settings {
            for setting in &mut block.settings {
                setting.value = setting.default.clone();
            }
        }
        shell
    });
    (script(ctx), shell, shortcuts)
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

/// How many actions the shell and its modules have and how many have a shortcut, or nothing when
/// they have none.
fn shortcuts_summary(ctx: &PageContext, shell: &Shell) -> Option<String> {
    if shell.actions.is_empty() {
        return None;
    }
    // With its default keys and the user's, as bento settles which runs each key.
    let config = ctx.store.config();
    let declared: Vec<_> = shell.actions.iter().map(|a| a.action.clone()).collect();
    let mut defaults = niri_config::shortcuts::bento_defaults().to_vec();
    defaults.extend(niri_config::shortcuts::declared_binds(
        &declared,
        niri_config::shortcuts::shell_action,
    ));
    let resolved = niri_config::shortcuts::resolve(&config.binds.0, &config.unbound, &defaults);
    let bound = declared
        .iter()
        .filter(|action| {
            resolved.in_use.iter().any(|bind| {
                matches!(
                    niri_config::shortcuts::shortcut(bind),
                    Some(Action::ShellAction(name, ..)) if *name == action.name
                )
            })
        })
        .count();
    let total = declared.len();
    Some(format!(
        "This shell has {total} {}, {bound} with a shortcut",
        if total == 1 { "action" } else { "actions" }
    ))
}

/// The shell's shortcuts summed up, with a link to the Shortcuts page, which lists them by module.
fn shortcuts_row(view: &Rc<View>, summary: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .use_markup(false)
        .title("Shortcuts")
        .subtitle(summary)
        .activatable(true)
        .build();
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    row.connect_activated(|row| {
        let _ = row.activate_action("win.open-page", Some(&"shortcuts".to_variant()));
    });
    view.ctx.anchor("shell.actions", &row);
    row
}

fn set_script(ctx: &PageContext, script: String) {
    ctx.store.edit("shell.script", move |doc| {
        doc.set(&["shell", "script"], vec![Value::Str(script)]);
    });
}

/// The shell chosen, its file, and its shortcuts summed up last, when it has any.
fn choice_group(view: &Rc<View>, shortcuts: Option<&str>) -> adw::PreferencesGroup {
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
            let view = Rc::downgrade(view);
            row.connect_activated(move |row| {
                if let Some(view) = view.upgrade() {
                    choose_file(&view, row);
                }
            });
            group.add(&row);
        }
        ShellScript::BuiltIn(name) => {
            let row = adw::ActionRow::builder()
                .use_markup(false)
                .title("Make a Copy")
                .activatable(true)
                .build();
            row.add_suffix(&gtk::Image::from_icon_name("edit-copy-symbolic"));
            let view = Rc::downgrade(view);
            row.connect_activated(move |row| {
                let Some(view) = view.upgrade() else { return };
                match copy_built_in(&view.ctx, &name) {
                    Ok(path) => {
                        set_script(&view.ctx, tilde(&path));
                        view.ctx.toast(&format!("Copied to {}", tilde(&path)));
                        crate::page::open_file(row, &path);
                    }
                    Err(err) => view.ctx.toast(&err),
                }
            });
            group.add(&row);
        }
    }
    if let Some(summary) = shortcuts {
        group.add(&shortcuts_row(view, summary));
    }
    group
}

/// The shell's folder, `shell/` next to the config.
fn shell_dir(ctx: &PageContext) -> PathBuf {
    let config = ctx.store.config_path();
    config.parent().unwrap_or(Path::new("")).join("shell")
}

fn expand_home(path: &Path) -> PathBuf {
    match path.strip_prefix("~") {
        Ok(rest) => glib::home_dir().join(rest),
        Err(_) => path.to_path_buf(),
    }
}

/// Copies bento's shell `name` into the shell's folder, from the copy bento keeps of its shells,
/// as `<name>.tsx`, or `<name>-2.tsx` and so on if that's taken.
fn copy_built_in(ctx: &PageContext, name: &str) -> Result<PathBuf, String> {
    let source = glib::user_data_dir().join(format!("bento/shell/shells/{name}.tsx"));
    let text = std::fs::read_to_string(&source)
        .map_err(|err| format!("Couldn't read bento's {name} shell: {err}"))?;
    let dir = shell_dir(ctx);
    std::fs::create_dir_all(&dir).map_err(|err| format!("Couldn't make {}: {err}", tilde(&dir)))?;
    let path = (1..)
        .map(|n| match n {
            1 => dir.join(format!("{name}.tsx")),
            n => dir.join(format!("{name}-{n}.tsx")),
        })
        .find(|path| !path.exists())
        .expect("a name is free");
    std::fs::write(&path, text).map_err(|err| format!("Couldn't write {}: {err}", tilde(&path)))?;
    Ok(path)
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
        .initial_folder(&gtk::gio::File::for_path(shell_dir(&view.ctx)))
        .build();

    let view = Rc::downgrade(view);
    let window = widget.root().and_downcast::<gtk::Window>();
    dialog.open(
        window.as_ref(),
        None::<&gtk::gio::Cancellable>,
        move |result| {
            let Some(view) = view.upgrade() else { return };
            match result.ok().and_then(|file| file.path()) {
                Some(path) => set_script(&view.ctx, tilde(&path)),
                // Cancelled: rebuild, so the dropdown shows the configured shell again.
                None => {
                    view.shown.replace(None);
                    refresh(&view);
                }
            }
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
