//! The blur settings as the person sees them: the config, with the changes they made that aren't
//! saved yet on top, so the preview and every control follow a change as it's made.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use niri_config::{Blur, GlassLight, Material, MAX_GLASS_LIGHTS};

use crate::store::Store;

/// A change the person made that isn't saved yet.
#[derive(Debug, Clone, Copy)]
pub enum Unsaved {
    /// A number or switch, by its setting's path. A light's are under its place, like
    /// `blur.glass.light.1.angle`, with `off` 1 for off.
    Setting(&'static str, f64),
    Material(Material),
    /// Glass's lights, after adding or removing one.
    Lights([Option<GlassLight>; MAX_GLASS_LIGHTS]),
}

impl Unsaved {
    /// The store's key for the change, which tells when it's saved.
    fn key(&self) -> &'static str {
        match self {
            Self::Setting(path, _) => path,
            Self::Material(_) => "blur.material",
            Self::Lights(_) => "blur.glass.lights",
        }
    }

    fn apply(&self, blur: &mut Blur) {
        let glass = &mut blur.glass;
        match *self {
            Self::Material(material) => blur.material = material,
            Self::Lights(lights) => glass.lights = lights,
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
                "blur.glass.reflection" => glass.reflection = x,
                "blur.glass.sheen" => glass.sheen = x,
                _ => {
                    let Some((i, field)) = light_field(path) else {
                        return;
                    };
                    let Some(light) = glass.lights.get_mut(i) else {
                        return;
                    };
                    let light = light.get_or_insert_with(GlassLight::default);
                    match field {
                        "off" => light.on = x == 0.,
                        "intensity" => light.intensity = x,
                        "angle" => light.angle = x,
                        "height" => light.height = x,
                        "size" => light.size = x,
                        _ => (),
                    }
                }
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
        "blur.glass.reflection" => glass.reflection,
        "blur.glass.sheen" => glass.sheen,
        _ => {
            let (i, field) = light_field(path)?;
            let light = glass.lights.get(i)?.as_ref()?;
            match field {
                "off" => f64::from(u8::from(!light.on)),
                "intensity" => light.intensity,
                "angle" => light.angle,
                "height" => light.height,
                "size" => light.size,
                _ => return None,
            }
        }
    })
}

/// The light and its field a path like `blur.glass.light.1.angle` names.
fn light_field(path: &str) -> Option<(usize, &str)> {
    let (i, field) = path.strip_prefix("blur.glass.light.")?.split_once('.')?;
    Some((i.parse().ok()?, field))
}

/// Called with the settings and the key of the edit that changed them, or `None` when the config
/// did.
type Listener = Box<dyn Fn(&Live, Option<&str>)>;

struct Inner {
    store: Store,
    /// Changes not saved yet, by the store's key for their edit.
    unsaved: RefCell<Vec<(String, Unsaved)>>,
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
    pub fn show_edit(&self, key: &str, changes: &[Unsaved]) {
        let mut unsaved = self.0.unsaved.borrow_mut();
        unsaved.retain(|(old, _)| old != key);
        unsaved.extend(changes.iter().map(|change| (key.to_owned(), *change)));
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
    pub fn connect(&self, f: impl Fn(&Live, Option<&str>) + 'static) {
        self.0.listeners.borrow_mut().push(Box::new(f));
    }

    fn notify(&self, key: Option<&str>) {
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

    #[test]
    fn a_lights_settings_reach_that_light() {
        let mut blur = Blur::default();
        Unsaved::Setting("blur.glass.light.0.angle", 90.).apply(&mut blur);
        Unsaved::Setting("blur.glass.light.0.off", 1.).apply(&mut blur);
        assert_eq!(
            blur.glass.lights[0],
            Some(GlassLight {
                on: false,
                angle: 90.,
                ..GlassLight::default()
            })
        );
        assert_eq!(value(&blur, "blur.glass.light.0.angle"), Some(90.));
        assert_eq!(value(&blur, "blur.glass.light.0.off"), Some(1.));
        assert_eq!(value(&blur, "blur.glass.light.1.angle"), None);
    }
}
