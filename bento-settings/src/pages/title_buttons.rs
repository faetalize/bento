//! The buttons in the title bars apps draw themselves, from GNOME's window manager settings, as
//! GNOME Tweaks sets them. GTK, libadwaita and apps that follow GTK, like Firefox, read it.
//!
//! `button-layout` is the left side and the right side split by `:`, each a list of `,`
//! separated items, like `appmenu:minimize,maximize,close`.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;

use crate::page::PageContext;

const SCHEMA: &str = "org.gnome.desktop.wm.preferences";
const KEY: &str = "button-layout";

/// The buttons that work the window, as they sit when they're on the right; on the left, close
/// comes first, so it's always at the title bar's edge.
const CONTROLS: [&str; 3] = ["minimize", "maximize", "close"];

#[derive(Debug, Clone, PartialEq)]
struct Layout {
    /// What isn't a window button, like `appmenu` or `icon`, on each side.
    left: Vec<String>,
    right: Vec<String>,
    /// Which of the window buttons show.
    shown: [bool; 3],
    /// Whether the window buttons are on the left.
    on_left: bool,
}

impl Layout {
    fn parse(layout: &str) -> Self {
        let (left, right) = layout.split_once(':').unwrap_or((layout, ""));
        let items = |side: &str| -> Vec<String> {
            side.split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(String::from)
                .collect()
        };
        let (left, right) = (items(left), items(right));
        let shown = CONTROLS.map(|button| left.iter().chain(&right).any(|item| item == button));
        // The side close is on, or the side with more of the buttons.
        let on_left = if left.iter().any(|item| item == "close") {
            true
        } else if right.iter().any(|item| item == "close") {
            false
        } else {
            let count = |side: &[String]| {
                side.iter()
                    .filter(|i| CONTROLS.contains(&i.as_str()))
                    .count()
            };
            count(&left) > count(&right)
        };
        let others = |side: Vec<String>| -> Vec<String> {
            side.into_iter()
                .filter(|item| !CONTROLS.contains(&item.as_str()))
                .collect()
        };
        Self {
            left: others(left),
            right: others(right),
            shown,
            on_left,
        }
    }

    fn write(&self) -> String {
        let mut controls: Vec<&str> = CONTROLS
            .iter()
            .zip(self.shown)
            .filter(|(_, shown)| *shown)
            .map(|(button, _)| *button)
            .collect();
        let (mut left, mut right): (Vec<&str>, Vec<&str>) = (
            self.left.iter().map(String::as_str).collect(),
            self.right.iter().map(String::as_str).collect(),
        );
        if self.on_left {
            controls.rotate_right(usize::from(self.shown[2]));
            controls.extend(left);
            left = controls;
        } else {
            right.extend(controls);
        }
        format!("{}:{}", left.join(","), right.join(","))
    }

    fn shows(&self, button: &str) -> bool {
        CONTROLS
            .iter()
            .position(|b| *b == button)
            .is_some_and(|at| self.shown[at])
    }

    fn with(mut self, button: &str, shown: bool) -> Self {
        if let Some(at) = CONTROLS.iter().position(|b| *b == button) {
            self.shown[at] = shown;
        }
        self
    }

    /// The window buttons on `left` or the right; what else is on the title bar goes to the other
    /// side, as it would sit across from them.
    fn placed(mut self, left: bool) -> Self {
        if self.on_left != left {
            std::mem::swap(&mut self.left, &mut self.right);
            self.on_left = left;
        }
        self
    }
}

pub fn build(ctx: &PageContext) -> gtk::Widget {
    let group = adw::PreferencesGroup::builder()
        .title("Title Bar Buttons")
        .description(
            "For apps that draw their own title bar. Minimize shows only with a layout engine \
             that minimizes windows.",
        )
        .build();
    let Some(settings) = wm_settings() else {
        group.set_visible(false);
        return group.upcast();
    };

    let maximize = adw::SwitchRow::builder().title("Maximize").build();
    ctx.anchor("appearance.title-buttons.maximize", &maximize);
    let minimize = adw::SwitchRow::builder().title("Minimize").build();
    ctx.anchor("appearance.title-buttons.minimize", &minimize);
    let placement = adw::ComboRow::builder()
        .title("Placement")
        .model(&gtk::StringList::new(&["Left", "Right"]))
        .build();
    ctx.anchor("appearance.title-buttons.placement", &placement);
    group.add(&maximize);
    group.add(&minimize);
    group.add(&placement);

    // Set while the rows show the setting, so they don't write it back.
    let updating = Rc::new(Cell::new(false));
    let show = {
        let (maximize, minimize, placement, updating) = (
            maximize.clone(),
            minimize.clone(),
            placement.clone(),
            updating.clone(),
        );
        move |settings: &gio::Settings| {
            let layout = Layout::parse(&settings.string(KEY));
            updating.set(true);
            maximize.set_active(layout.shows("maximize"));
            minimize.set_active(layout.shows("minimize"));
            placement.set_selected(if layout.on_left { 0 } else { 1 });
            updating.set(false);
        }
    };
    show(&settings);
    settings.connect_changed(Some(KEY), move |settings, _| show(settings));

    let change = {
        let (settings, updating) = (settings.clone(), updating.clone());
        move |f: &dyn Fn(Layout) -> Layout| {
            if updating.get() {
                return;
            }
            let layout = f(Layout::parse(&settings.string(KEY))).write();
            if settings.string(KEY) != layout {
                let _ = settings.set_string(KEY, &layout);
            }
        }
    };
    for (row, button) in [(&maximize, "maximize"), (&minimize, "minimize")] {
        let change = change.clone();
        row.connect_active_notify(move |row| {
            let shown = row.is_active();
            change(&|layout| layout.with(button, shown));
        });
    }
    placement.connect_selected_notify(move |row| {
        let left = row.selected() == 0;
        change(&|layout| layout.placed(left));
    });
    group.upcast()
}

fn wm_settings() -> Option<gio::Settings> {
    let schema = gio::SettingsSchemaSource::default()?.lookup(SCHEMA, true)?;
    schema.has_key(KEY).then(|| gio::Settings::new(SCHEMA))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changed(layout: &str, f: impl FnOnce(Layout) -> Layout) -> String {
        f(Layout::parse(layout)).write()
    }

    #[test]
    fn buttons_join_close_in_their_usual_order() {
        let layout = changed("appmenu:close", |l| l.with("maximize", true));
        assert_eq!(layout, "appmenu:maximize,close");
        let layout = changed(&layout, |l| l.with("minimize", true));
        assert_eq!(layout, "appmenu:minimize,maximize,close");
        assert_eq!(
            changed(&layout, |l| l.with("maximize", false)),
            "appmenu:minimize,close"
        );
    }

    #[test]
    fn on_the_left_close_is_at_the_edge_and_the_rest_crosses_over() {
        let layout = changed("appmenu:minimize,maximize,close", |l| l.placed(true));
        assert_eq!(layout, "close,minimize,maximize:appmenu");
        assert!(Layout::parse(&layout).on_left);
        assert_eq!(
            changed(&layout, |l| l.placed(false)),
            "appmenu:minimize,maximize,close"
        );
    }

    #[test]
    fn what_it_doesnt_know_stays_where_it_is() {
        let layout = changed("icon,appmenu:spacer,close", |l| l.with("minimize", true));
        assert_eq!(layout, "icon,appmenu:spacer,minimize,close");
        let parsed = Layout::parse("close,maximize:");
        assert!(parsed.on_left && parsed.shows("maximize") && !parsed.shows("minimize"));
    }
}
