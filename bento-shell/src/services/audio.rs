//! The default speaker's volume, through PulseAudio or PipeWire's PulseAudio server.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use libpulse_binding::context::subscribe::{Facility, InterestMaskSet};
use libpulse_binding::context::{Context, FlagSet, State};
use libpulse_binding::volume::{ChannelVolumes, Volume};
use libpulse_glib_binding::Mainloop;

/// The loudest volume the shell sets, as a fraction of 100%.
pub const MAX_VOLUME: f64 = 1.5;

/// How long to wait before connecting again after the sound server went away.
const RETRY: Duration = Duration::from_secs(1);

/// The default speaker's volume.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Audio {
    /// From 0 to [`MAX_VOLUME`], where 1 is 100%.
    pub volume: f64,
    pub muted: bool,
    /// Whether there's a sound server and a default speaker.
    pub available: bool,
}

/// Watches and changes the default speaker's volume. It follows the default speaker as it
/// changes, and connects again when the sound server restarts.
pub struct AudioService {
    // Declared before the main loop, so it's dropped first.
    context: RefCell<Option<Context>>,
    mainloop: Option<Mainloop>,
    sink: RefCell<Option<Sink>>,
    state: Cell<Audio>,
    on_change: Box<dyn Fn(&Audio)>,
}

/// The default speaker as the sound server last described it.
struct Sink {
    name: String,
    index: u32,
    volumes: ChannelVolumes,
}

impl AudioService {
    pub fn start(on_change: impl Fn(&Audio) + 'static) -> Rc<Self> {
        let mainloop = Mainloop::new(None);
        if mainloop.is_none() {
            eprintln!("Couldn't run PulseAudio on the main loop, so there's no volume to show");
            on_change(&Audio::default());
        }

        let this = Rc::new(Self {
            context: RefCell::default(),
            mainloop,
            sink: RefCell::default(),
            state: Cell::new(Audio::default()),
            on_change: Box::new(on_change),
        });
        this.connect();
        this
    }

    pub fn state(&self) -> Audio {
        self.state.get()
    }

    /// Sets the default speaker's volume, from 0 to [`MAX_VOLUME`], keeping its balance.
    pub fn set_volume(&self, volume: f64) {
        let mut sink = self.sink.borrow_mut();
        let Some(sink) = sink.as_mut() else { return };
        if sink.volumes.scale(to_volume(volume)).is_none() {
            return;
        }
        if let Some(context) = self.context.borrow_mut().as_mut() {
            context
                .introspect()
                .set_sink_volume_by_name(&sink.name, &sink.volumes, None);
        }
    }

    pub fn set_muted(&self, muted: bool) {
        let sink = self.sink.borrow();
        let Some(sink) = sink.as_ref() else { return };
        if let Some(context) = self.context.borrow_mut().as_mut() {
            context
                .introspect()
                .set_sink_mute_by_name(&sink.name, muted, None);
        }
    }

    fn connect(self: &Rc<Self>) {
        let Some(mainloop) = &self.mainloop else {
            return;
        };
        let Some(mut context) = Context::new(mainloop, "bento-shell") else {
            eprintln!("Couldn't make a PulseAudio context");
            return;
        };

        // The callbacks run while PulseAudio dispatches, sometimes from inside a call on the
        // context, so they hand the work to the main loop instead of touching the context.
        let weak = Rc::downgrade(self);
        context.set_state_callback(Some(Box::new(move || {
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                if let Some(this) = weak.upgrade() {
                    this.state_changed();
                }
            });
        })));

        let weak = Rc::downgrade(self);
        context.set_subscribe_callback(Some(Box::new(move |facility, _, index| {
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                if let Some(this) = weak.upgrade() {
                    this.changed(facility, index);
                }
            });
        })));

        // Without failing, the context waits for a sound server to appear.
        if let Err(err) = context.connect(None, FlagSet::NOFAIL, None) {
            eprintln!("Couldn't connect to PulseAudio: {err}");
            return;
        }
        *self.context.borrow_mut() = Some(context);
    }

    fn state_changed(self: &Rc<Self>) {
        let state = match self.context.borrow().as_ref() {
            Some(context) => context.get_state(),
            None => return,
        };
        match state {
            State::Ready => {
                if let Some(context) = self.context.borrow_mut().as_mut() {
                    context.subscribe(InterestMaskSet::SINK | InterestMaskSet::SERVER, |_| ());
                }
                self.refresh();
            }
            State::Failed | State::Terminated => {
                eprintln!("Lost the connection to PulseAudio");
                if let Some(mut context) = self.context.borrow_mut().take() {
                    context.set_state_callback(None);
                    context.set_subscribe_callback(None);
                }
                self.forget_sink();
                let weak: Weak<Self> = Rc::downgrade(self);
                glib::timeout_add_local_once(RETRY, move || {
                    if let Some(this) = weak.upgrade() {
                        this.connect();
                    }
                });
            }
            _ => (),
        }
    }

    fn changed(self: &Rc<Self>, facility: Option<Facility>, index: u32) {
        match facility {
            // The default speaker can change.
            Some(Facility::Server) => self.refresh(),
            Some(Facility::Sink) => {
                let current = self.sink.borrow().as_ref().map(|sink| sink.index);
                match current {
                    Some(current) if current == index => self.read_sink(),
                    // The default speaker came or went.
                    _ => self.refresh(),
                }
            }
            _ => (),
        }
    }

    /// Finds the default speaker again and reads it.
    fn refresh(self: &Rc<Self>) {
        let context = self.context.borrow();
        let Some(context) = context.as_ref() else {
            return;
        };
        let weak = Rc::downgrade(self);
        context.introspect().get_server_info(move |info| {
            let name = info.default_sink_name.as_deref().map(String::from);
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                let Some(this) = weak.upgrade() else { return };
                match name {
                    Some(name) => this.read_sink_named(name),
                    None => this.forget_sink(),
                }
            });
        });
    }

    fn read_sink(self: &Rc<Self>) {
        let name = self.sink.borrow().as_ref().map(|sink| sink.name.clone());
        if let Some(name) = name {
            self.read_sink_named(name);
        }
    }

    fn read_sink_named(self: &Rc<Self>, name: String) {
        let context = self.context.borrow();
        let Some(context) = context.as_ref() else {
            return;
        };
        let weak = Rc::downgrade(self);
        context
            .introspect()
            .get_sink_info_by_name(&name, move |result| {
                use libpulse_binding::callbacks::ListResult;
                let (sink, muted) = match result {
                    ListResult::Item(info) => {
                        let sink = Sink {
                            name: info.name.as_deref().unwrap_or_default().to_owned(),
                            index: info.index,
                            volumes: info.volume,
                        };
                        (Some(sink), info.mute)
                    }
                    ListResult::End => return,
                    ListResult::Error => (None, false),
                };
                let weak = weak.clone();
                glib::idle_add_local_once(move || {
                    let Some(this) = weak.upgrade() else { return };
                    match sink {
                        Some(sink) => this.set_sink(sink, muted),
                        None => this.forget_sink(),
                    }
                });
            });
    }

    fn set_sink(&self, sink: Sink, muted: bool) {
        let state = Audio {
            volume: to_fraction(sink.volumes.max()),
            muted,
            available: true,
        };
        *self.sink.borrow_mut() = Some(sink);
        self.state.set(state);
        self.notify();
    }

    fn forget_sink(&self) {
        *self.sink.borrow_mut() = None;
        self.state.set(Audio::default());
        self.notify();
    }

    fn notify(&self) {
        (self.on_change)(&self.state.get());
    }
}

/// A volume as a fraction of 100%, from 0 to [`MAX_VOLUME`].
fn to_fraction(volume: Volume) -> f64 {
    (f64::from(volume.0) / f64::from(Volume::NORMAL.0)).clamp(0., MAX_VOLUME)
}

fn to_volume(fraction: f64) -> Volume {
    let fraction = if fraction.is_finite() {
        fraction.clamp(0., MAX_VOLUME)
    } else {
        0.
    };
    Volume((fraction * f64::from(Volume::NORMAL.0)).round() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volumes_are_fractions_of_100_percent() {
        assert_eq!(to_fraction(Volume::NORMAL), 1.);
        assert_eq!(to_fraction(Volume::MUTED), 0.);
        assert_eq!(to_volume(1.), Volume::NORMAL);
        assert_eq!(to_volume(0.5).0, Volume::NORMAL.0 / 2);
        assert!((to_fraction(to_volume(0.37)) - 0.37).abs() < 1e-4);
    }

    #[test]
    fn volumes_stay_in_range() {
        assert_eq!(to_volume(-1.), Volume::MUTED);
        assert_eq!(to_volume(f64::NAN), Volume::MUTED);
        assert_eq!(to_fraction(to_volume(9.)), MAX_VOLUME);
        assert_eq!(to_fraction(Volume(Volume::NORMAL.0 * 3)), MAX_VOLUME);
    }
}
