//! The blur settings as the person sees them: the config, with the changes they made that aren't
//! saved yet on top, so the preview and every control follow a change as it's made.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use niri_config::{Blur, Material};

use crate::store::Store;

/// A change the person made that isn't saved yet.
#[derive(Debug, Clone, Copy)]
pub enum Unsaved {
    /// A number or switch, by its setting's path.
    Setting(&'static str, f64),
    Material(Material),
}

impl Unsaved {
    /// The store's key for the change, which tells when it's saved.
    fn key(&self) -> &'static str {
        match self {
            Self::Setting(path, _) => path,
            Self::Material(_) => "blur.material",
        }
    }

    fn apply(&self, blur: &mut Blur) {
        let glass = &mut blur.glass;
        match *self {
            Self::Material(material) => blur.material = material,
            Self::Setting(path, x) => match path {
                "blur" => blur.off = x == 0.,
                "blur.offset" => blur.offset = x,
                "blur.passes" => blur.passes = x.round().clamp(0., 255.) as u8,
                "blur.noise" => blur.noise = x,
                "blur.saturation" => blur.saturation = x,
                "blur.glass.bezel" => glass.bezel = x,
                "blur.glass.thickness" => glass.thickness = x,
                "blur.glass.refractive-index" => glass.refractive_index = x,
                "blur.glass.dispersion" => glass.dispersion = x,
                "blur.glass.highlight" => glass.highlight = x,
                "blur.glass.reflection" => glass.reflection = x,
                "blur.glass.sheen" => glass.sheen = x,
                "blur.glass.light-size" => glass.light_size = x,
                "blur.glass.light-angle" => glass.light_angle = x,
                _ => (),
            },
        }
    }
}

/// The value of the setting at `path` in `blur`, for the numbers `Unsaved` sets.
pub fn value(blur: &Blur, path: &str) -> Option<f64> {
    let glass = &blur.glass;
    Some(match path {
        "blur.offset" => blur.offset,
        "blur.passes" => f64::from(blur.passes),
        "blur.noise" => blur.noise,
        "blur.saturation" => blur.saturation,
        "blur.glass.bezel" => glass.bezel,
        "blur.glass.thickness" => glass.thickness,
        "blur.glass.refractive-index" => glass.refractive_index,
        "blur.glass.dispersion" => glass.dispersion,
        "blur.glass.highlight" => glass.highlight,
        "blur.glass.reflection" => glass.reflection,
        "blur.glass.sheen" => glass.sheen,
        "blur.glass.light-size" => glass.light_size,
        "blur.glass.light-angle" => glass.light_angle,
        _ => return None,
    })
}

/// Called with the settings and the key of the edit that changed them, or `None` when the config
/// did.
type Listener = Box<dyn Fn(&Live, Option<&'static str>)>;

struct Inner {
    store: Store,
    /// Changes not saved yet, by the store's key for their edit.
    unsaved: RefCell<Vec<(&'static str, Unsaved)>>,
    listeners: RefCell<Vec<Listener>>,
}

#[derive(Clone)]
pub struct Live(Rc<Inner>);

impl Live {
    pub fn new(store: &Store) -> Self {
        let live = Self(Rc::new(Inner {
            store: store.clone(),
            unsaved: RefCell::default(),
            listeners: RefCell::default(),
        }));

        let weak: Weak<Inner> = Rc::downgrade(&live.0);
        store.subscribe(move || {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            // Saved changes come from the config from now on.
            let store = inner.store.clone();
            inner
                .unsaved
                .borrow_mut()
                .retain(|(key, _)| store.is_pending(key));
            Live(inner).notify(None);
        });

        live
    }

    /// Shows a change saved under its setting's own key.
    pub fn show(&self, change: Unsaved) {
        self.show_edit(change.key(), &[change]);
    }

    /// Shows changes that one edit saves together, under the edit's `key`.
    pub fn show_edit(&self, key: &'static str, changes: &[Unsaved]) {
        let mut unsaved = self.0.unsaved.borrow_mut();
        unsaved.retain(|(old, _)| *old != key);
        unsaved.extend(changes.iter().map(|change| (key, *change)));
        drop(unsaved);
        self.notify(Some(key));
    }

    /// The blur settings, with the unsaved changes.
    pub fn blur(&self) -> Blur {
        let mut blur = self.0.store.config().blur;
        for (_, change) in self.0.unsaved.borrow().iter() {
            change.apply(&mut blur);
        }
        blur
    }

    /// What the control for the setting at `path` shows: its value with the unsaved changes, so
    /// it follows another control that sets it too.
    pub fn shown(&self, path: &str) -> Option<f64> {
        value(&self.blur(), path)
    }

    /// Calls `f` after every change, with the settings and the key of the edit that made it, or
    /// `None` when it came from the config.
    pub fn connect(&self, f: impl Fn(&Live, Option<&'static str>) + 'static) {
        self.0.listeners.borrow_mut().push(Box::new(f));
    }

    fn notify(&self, key: Option<&'static str>) {
        for listener in self.0.listeners.borrow().iter() {
            listener(self, key);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::store::on_own_context;

    #[test]
    fn controls_show_what_another_control_changed_before_it_saves() {
        on_own_context(|| {
            let dir =
                std::env::temp_dir().join(format!("bento-settings-live-{}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            let config_path = dir.join("config.kdl");
            fs::write(&config_path, "").unwrap();
            let live = Live::new(&Store::load_from(config_path));

            // Like the Preset slider, one edit that sets sheen among others.
            live.show_edit(
                "blur.glass.preset",
                &[
                    Unsaved::Setting("blur.glass.sheen", 0.8),
                    Unsaved::Setting("blur.noise", 0.01),
                ],
            );
            assert_eq!(live.shown("blur.glass.sheen"), Some(0.8));
            assert_eq!(live.shown("blur.noise"), Some(0.01));

            fs::remove_dir_all(&dir).unwrap();
        });
    }
}
