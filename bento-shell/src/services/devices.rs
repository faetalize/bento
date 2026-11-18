//! The sound outputs and inputs, which one of each is the default, and each one's volume, through
//! PulseAudio or PipeWire's PulseAudio server.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use libpulse_binding::callbacks::ListResult;
use libpulse_binding::context::subscribe::InterestMaskSet;
use libpulse_binding::context::{Context, FlagSet, State};
use libpulse_binding::proplist::Proplist;
use libpulse_binding::volume::{ChannelVolumes, Volume};
use libpulse_glib_binding::Mainloop;

use super::audio::MAX_VOLUME;

/// How long to wait before connecting again after the sound server went away.
const RETRY: Duration = Duration::from_secs(1);

/// A sound output or input.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    /// What the sound server calls it, which stays the same between runs.
    pub name: String,
    /// What to call it for the user.
    pub label: String,
    /// A symbolic icon for the kind of device it is.
    pub icon: &'static str,
    /// Whether it's the default one.
    pub active: bool,
    /// From 0 to [`MAX_VOLUME`], where 1 is 100%.
    pub volume: f64,
    pub muted: bool,
    volumes: ChannelVolumes,
}

/// The outputs and the inputs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Devices {
    pub outputs: Vec<Device>,
    pub inputs: Vec<Device>,
}

/// Which of the two lists a device is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Output,
    Input,
}

/// Watches the sound devices, and changes which is the default and how loud each is. It connects
/// again when the sound server restarts.
pub struct DeviceService {
    // Declared before the main loop, so it's dropped first.
    context: RefCell<Option<Context>>,
    mainloop: Option<Mainloop>,
    state: RefCell<Devices>,
    /// Whether a read of the devices is already on its way.
    reading: Cell<bool>,
    /// Whether something changed while reading, so they're read once more after.
    stale: Cell<bool>,
    on_change: Box<dyn Fn(&Devices)>,
}

/// What a read of the devices has gathered so far.
#[derive(Default)]
struct Reading {
    default_output: Option<String>,
    default_input: Option<String>,
    outputs: Option<Vec<Device>>,
    inputs: Option<Vec<Device>>,
}

impl DeviceService {
    pub fn start(on_change: impl Fn(&Devices) + 'static) -> Rc<Self> {
        let mainloop = Mainloop::new(None);
        let this = Rc::new(Self {
            context: RefCell::default(),
            mainloop,
            state: RefCell::default(),
            reading: Cell::new(false),
            stale: Cell::new(false),
            on_change: Box::new(on_change),
        });
        (this.on_change)(&this.state.borrow());
        this.connect();
        this
    }

    /// Makes the device named `name` the default one, and moves what's playing or recording to
    /// it.
    pub fn set_default(&self, kind: Kind, name: &str) {
        let mut context = self.context.borrow_mut();
        let Some(context) = context.as_mut() else {
            return;
        };
        match kind {
            Kind::Output => {
                context.set_default_sink(name, |_| ());
                let mut mover = context.introspect();
                let target = name.to_owned();
                context
                    .introspect()
                    .get_sink_input_info_list(move |result| {
                        if let ListResult::Item(stream) = result {
                            mover.move_sink_input_by_name(stream.index, &target, None);
                        }
                    });
            }
            Kind::Input => {
                context.set_default_source(name, |_| ());
                let mut mover = context.introspect();
                let target = name.to_owned();
                context
                    .introspect()
                    .get_source_output_info_list(move |result| {
                        if let ListResult::Item(stream) = result {
                            mover.move_source_output_by_name(stream.index, &target, None);
                        }
                    });
            }
        }
    }

    /// Sets the volume of the device named `name`, from 0 to [`MAX_VOLUME`], keeping its balance.
    pub fn set_volume(&self, kind: Kind, name: &str, volume: f64) {
        let Some(mut volumes) = self.find(kind, name).map(|device| device.volumes) else {
            return;
        };
        let volume = if volume.is_finite() {
            volume.clamp(0., MAX_VOLUME)
        } else {
            0.
        };
        let volume = Volume((volume * f64::from(Volume::NORMAL.0)).round() as u32);
        if volumes.scale(volume).is_none() {
            return;
        }
        if let Some(context) = self.context.borrow_mut().as_mut() {
            let mut introspect = context.introspect();
            match kind {
                Kind::Output => introspect.set_sink_volume_by_name(name, &volumes, None),
                Kind::Input => introspect.set_source_volume_by_name(name, &volumes, None),
            };
        }
    }

    pub fn set_muted(&self, kind: Kind, name: &str, muted: bool) {
        if let Some(context) = self.context.borrow_mut().as_mut() {
            let mut introspect = context.introspect();
            match kind {
                Kind::Output => introspect.set_sink_mute_by_name(name, muted, None),
                Kind::Input => introspect.set_source_mute_by_name(name, muted, None),
            };
        }
    }

    fn find(&self, kind: Kind, name: &str) -> Option<Device> {
        let state = self.state.borrow();
        let list = match kind {
            Kind::Output => &state.outputs,
            Kind::Input => &state.inputs,
        };
        list.iter().find(|device| device.name == name).cloned()
    }

    fn connect(self: &Rc<Self>) {
        let Some(mainloop) = &self.mainloop else {
            return;
        };
        let Some(mut context) = Context::new(mainloop, "bento-shell devices") else {
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
        context.set_subscribe_callback(Some(Box::new(move |_, _, _| {
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                if let Some(this) = weak.upgrade() {
                    this.read();
                }
            });
        })));
        // Without failing, the context waits for a sound server to appear.
        if context.connect(None, FlagSet::NOFAIL, None).is_err() {
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
                    let interest = InterestMaskSet::SINK
                        | InterestMaskSet::SOURCE
                        | InterestMaskSet::SERVER
                        | InterestMaskSet::CARD;
                    context.subscribe(interest, |_| ());
                }
                self.read();
            }
            State::Failed | State::Terminated => {
                if let Some(mut context) = self.context.borrow_mut().take() {
                    context.set_state_callback(None);
                    context.set_subscribe_callback(None);
                }
                self.reading.set(false);
                self.publish(Devices::default());
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

    /// Reads the devices: the defaults, then the outputs and the inputs. Changes that come while
    /// it reads make it read once more, rather than once each.
    fn read(self: &Rc<Self>) {
        if self.reading.replace(true) {
            self.stale.set(true);
            return;
        }
        let context = self.context.borrow();
        let Some(context) = context.as_ref() else {
            self.reading.set(false);
            return;
        };
        let reading: Rc<RefCell<Reading>> = Rc::default();
        let introspect = context.introspect();

        let gathered = reading.clone();
        let weak = Rc::downgrade(self);
        introspect.get_server_info(move |info| {
            let mut reading = gathered.borrow_mut();
            reading.default_output = info.default_sink_name.as_deref().map(String::from);
            reading.default_input = info.default_source_name.as_deref().map(String::from);
            drop(reading);
            Self::finish(&weak, &gathered);
        });

        let gathered = reading.clone();
        let weak = Rc::downgrade(self);
        let mut outputs = Vec::new();
        introspect.get_sink_info_list(move |result| match result {
            ListResult::Item(info) => {
                let port = info.active_port.as_ref();
                outputs.push(Device {
                    name: info.name.as_deref().unwrap_or_default().to_owned(),
                    label: info.description.as_deref().unwrap_or_default().to_owned(),
                    icon: icon(
                        Kind::Output,
                        &info.proplist,
                        port.and_then(|port| port.name.as_deref()),
                    ),
                    active: false,
                    volume: fraction(info.volume.max()),
                    muted: info.mute,
                    volumes: info.volume,
                });
            }
            ListResult::End | ListResult::Error => {
                gathered.borrow_mut().outputs = Some(std::mem::take(&mut outputs));
                Self::finish(&weak, &gathered);
            }
        });

        let gathered = reading.clone();
        let weak = Rc::downgrade(self);
        let mut inputs = Vec::new();
        introspect.get_source_info_list(move |result| match result {
            ListResult::Item(info) => {
                // What an output plays can be recorded from its monitor, which isn't a device.
                if info.monitor_of_sink.is_some() {
                    return;
                }
                let port = info.active_port.as_ref();
                inputs.push(Device {
                    name: info.name.as_deref().unwrap_or_default().to_owned(),
                    label: info.description.as_deref().unwrap_or_default().to_owned(),
                    icon: icon(
                        Kind::Input,
                        &info.proplist,
                        port.and_then(|port| port.name.as_deref()),
                    ),
                    active: false,
                    volume: fraction(info.volume.max()),
                    muted: info.mute,
                    volumes: info.volume,
                });
            }
            ListResult::End | ListResult::Error => {
                gathered.borrow_mut().inputs = Some(std::mem::take(&mut inputs));
                Self::finish(&weak, &gathered);
            }
        });
    }

    /// Publishes a read once all of its parts are in, from the main loop.
    fn finish(weak: &Weak<Self>, reading: &Rc<RefCell<Reading>>) {
        let done = {
            let reading = reading.borrow();
            reading.outputs.is_some() && reading.inputs.is_some()
        };
        if !done {
            return;
        }
        // The server answers in the order it was asked, so the defaults are in by now.
        let reading = std::mem::take(&mut *reading.borrow_mut());
        let weak = weak.clone();
        glib::idle_add_local_once(move || {
            let Some(this) = weak.upgrade() else { return };
            let mut devices = Devices {
                outputs: reading.outputs.unwrap_or_default(),
                inputs: reading.inputs.unwrap_or_default(),
            };
            for device in &mut devices.outputs {
                device.active = Some(&device.name) == reading.default_output.as_ref();
            }
            for device in &mut devices.inputs {
                device.active = Some(&device.name) == reading.default_input.as_ref();
            }
            this.reading.set(false);
            this.publish(devices);
            if this.stale.replace(false) {
                this.read();
            }
        });
    }

    fn publish(&self, devices: Devices) {
        if *self.state.borrow() == devices {
            return;
        }
        *self.state.borrow_mut() = devices;
        (self.on_change)(&self.state.borrow());
    }
}

fn fraction(volume: Volume) -> f64 {
    (f64::from(volume.0) / f64::from(Volume::NORMAL.0)).clamp(0., MAX_VOLUME)
}

/// A symbolic icon for a device, from what the sound server says it is.
fn icon(kind: Kind, properties: &Proplist, port: Option<&str>) -> &'static str {
    let property = |key: &str| properties.get_str(key).unwrap_or_default().to_lowercase();
    icon_for(
        kind,
        &property("device.form_factor"),
        &property("device.icon_name"),
        &port.unwrap_or_default().to_lowercase(),
    )
}

fn icon_for(kind: Kind, form_factor: &str, icon_name: &str, port: &str) -> &'static str {
    let says = |word: &str| form_factor.contains(word) || icon_name.contains(word);
    if says("headset") {
        "audio-headset-symbolic"
    } else if says("headphone") || port.contains("headphone") {
        "audio-headphones-symbolic"
    } else if kind == Kind::Input {
        if says("webcam") || says("camera") {
            "camera-web-symbolic"
        } else {
            "audio-input-microphone-symbolic"
        }
    } else if port.contains("hdmi") || port.contains("displayport") || says("video") {
        "video-display-symbolic"
    } else {
        "audio-speakers-symbolic"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_gets_the_icon_of_its_kind() {
        let output = |form, icon, port| icon_for(Kind::Output, form, icon, port);
        assert_eq!(output("headset", "", ""), "audio-headset-symbolic");
        assert_eq!(
            output("", "audio-headphones-bluetooth", ""),
            "audio-headphones-symbolic"
        );
        assert_eq!(
            output("internal", "audio-card-pci", "analog-output-headphones"),
            "audio-headphones-symbolic"
        );
        assert_eq!(
            output("", "audio-card-pci", "hdmi-output-0"),
            "video-display-symbolic"
        );
        assert_eq!(
            output("internal", "audio-card-pci", "analog-output-lineout"),
            "audio-speakers-symbolic"
        );
        let input = |form, icon, port| icon_for(Kind::Input, form, icon, port);
        assert_eq!(
            input("microphone", "audio-card-usb", ""),
            "audio-input-microphone-symbolic"
        );
        assert_eq!(input("webcam", "camera-web-usb", ""), "camera-web-symbolic");
        assert_eq!(input("headset", "", ""), "audio-headset-symbolic");
    }
}
