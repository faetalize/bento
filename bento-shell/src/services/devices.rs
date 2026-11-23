//! The sound outputs and inputs, which one of each is the default, and each one's volume; and the
//! apps playing and recording on them, each with its own volume, through PulseAudio or PipeWire's
//! PulseAudio server.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use libpulse_binding::callbacks::ListResult;
use libpulse_binding::context::subscribe::InterestMaskSet;
use libpulse_binding::context::{Context, FlagSet, State};
use libpulse_binding::def::{SinkFlagSet, SourceFlagSet};
use libpulse_binding::proplist::Proplist;
use libpulse_binding::volume::{ChannelVolumes, Volume};
use libpulse_glib_binding::Mainloop;

use super::audio::MAX_VOLUME;
use super::equalizer;
use super::targets::TargetWatch;

/// How long to wait before connecting again after the sound server went away.
const RETRY: Duration = Duration::from_secs(1);

/// A sound output or input.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    /// What the sound server calls it, which stays the same between runs.
    pub name: String,
    /// The sound server's description of it.
    pub label: String,
    /// A short name for the user, like "Line Out" or the headset's own name.
    pub title: String,
    /// What it is, like "USB" or its sound card, under the title.
    pub detail: String,
    /// Whether it's made by a program, rather than being hardware.
    pub is_virtual: bool,
    /// A symbolic icon for the kind of device it is.
    pub icon: &'static str,
    /// Whether it's the default one.
    pub active: bool,
    /// From 0 to [`MAX_VOLUME`], where 1 is 100%.
    pub volume: f64,
    pub muted: bool,
    volumes: ChannelVolumes,
    index: u32,
    /// Its node's `object.serial`, which a stream's target names.
    serial: Option<String>,
    /// The equalizer it plays through, whose volume is its own while it's on, so that the
    /// default's volume, as keys set it, is what it shows.
    through: Option<Through>,
}

/// An output's equalizer, as the sound server says it.
#[derive(Debug, Clone, PartialEq)]
struct Through {
    name: String,
    index: u32,
    serial: Option<String>,
    /// The output's own volume, under the equalizer's.
    own: ChannelVolumes,
}

/// An app's stream, playing on an output or recording from an input.
#[derive(Debug, Clone, PartialEq)]
pub struct Stream {
    /// Its index in the sound server, which commands take.
    pub index: u32,
    /// The app's name.
    pub app: String,
    /// The app's icon, when it says.
    pub icon: Option<String>,
    /// The app's program, to find its desktop entry by.
    pub binary: Option<String>,
    /// What it plays or records, like a song's or a page's title.
    pub title: Option<String>,
    /// The name of the device it's on.
    pub device: String,
    /// From 0, where 1 is 100%.
    pub volume: f64,
    pub muted: bool,
    /// Whether it's paused.
    pub paused: bool,
    /// Whether it's kept on its device rather than following the default, as it is once moved.
    pub kept: bool,
    /// Whether the app asked for a device itself, which wins again the next time it opens one.
    pub own: bool,
    /// Whether the app said it can't be moved.
    pub fixed: bool,
    volumes: ChannelVolumes,
    /// Its node's id, which its target in the metadata is kept by.
    node: Option<u32>,
    /// The name of the sink it's on: its device's, or its device's equalizer's.
    actual: String,
}

/// The outputs and the inputs, and the apps on them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Devices {
    pub outputs: Vec<Device>,
    pub inputs: Vec<Device>,
    pub playing: Vec<Stream>,
    pub recording: Vec<Stream>,
    /// Whether streams can be told to follow the default, or be kept where they are.
    pub can_keep: bool,
    /// The default output's name as the sound server says it, an equalizer's when it's one.
    default_output: Option<String>,
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
    targets: RefCell<Option<Rc<TargetWatch>>>,
    /// Whether a read of the devices is already on its way.
    reading: Cell<bool>,
    /// Whether something changed while reading, so they're read once more after.
    stale: Cell<bool>,
    on_change: Box<dyn Fn(&Devices)>,
    /// Others that hear the devices changed, after `on_change`.
    listeners: RefCell<Vec<Box<dyn Fn()>>>,
}

/// What a read of the devices has gathered so far.
#[derive(Default)]
struct Reading {
    default_output: Option<String>,
    default_input: Option<String>,
    outputs: Option<Vec<Device>>,
    inputs: Option<Vec<Device>>,
    /// The outputs' equalizers, read as outputs of their own.
    equalizers: Vec<Device>,
    /// Streams, with the index of the device they're on for now.
    playing: Option<Vec<(Stream, u32)>>,
    recording: Option<Vec<(Stream, u32)>>,
}

impl DeviceService {
    pub fn start(on_change: impl Fn(&Devices) + 'static) -> Rc<Self> {
        let mainloop = Mainloop::new(None);
        let this = Rc::new(Self {
            context: RefCell::default(),
            mainloop,
            state: RefCell::default(),
            targets: RefCell::default(),
            reading: Cell::new(false),
            stale: Cell::new(false),
            on_change: Box::new(on_change),
            listeners: RefCell::default(),
        });
        (this.on_change)(&this.state.borrow());
        let weak = Rc::downgrade(&this);
        let targets = TargetWatch::start(move || {
            if let Some(this) = weak.upgrade() {
                this.targets_changed();
            }
        });
        *this.targets.borrow_mut() = Some(targets);
        this.connect();
        this
    }

    /// The targets changed: which streams are kept is read again.
    fn targets_changed(&self) {
        let mut devices = self.state.borrow().clone();
        self.mark_kept(&mut devices);
        self.publish(devices);
    }

    fn mark_kept(&self, devices: &mut Devices) {
        let watch = self.targets.borrow();
        let targets = watch
            .as_ref()
            .map(|watch| watch.targets())
            .unwrap_or_default();
        devices.can_keep = watch.as_ref().is_some_and(|watch| watch.available());
        for stream in devices.playing.iter_mut().chain(&mut devices.recording) {
            stream.kept = stream.node.is_some_and(|node| targets.kept(node));
        }
    }

    /// Makes the device named `name` the default one. What plays or records follows it, but for
    /// apps kept on a device: the session manager moves them, as it keeps the others where they
    /// are.
    pub fn set_default(&self, kind: Kind, name: &str) {
        let mut context = self.context.borrow_mut();
        let Some(context) = context.as_mut() else {
            return;
        };
        match kind {
            Kind::Output => {
                let through = self.find(kind, name).and_then(|device| device.through);
                let name = through.as_ref().map_or(name, |through| &through.name);
                context.set_default_sink(name, |_| ())
            }
            Kind::Input => context.set_default_source(name, |_| ()),
        };
    }

    /// The equalizer of the output named `device` is there: what plays on the output goes through
    /// it, with the output's volume, the output at its full volume under it.
    pub fn adopt_equalizer(&self, device: &str) {
        let state = self.state.borrow().clone();
        let Some(output) = state.outputs.iter().find(|output| output.name == device) else {
            return;
        };
        let Some(through) = &output.through else {
            return;
        };
        let mut context = self.context.borrow_mut();
        let Some(context) = context.as_mut() else {
            return;
        };
        let mut introspect = context.introspect();
        let mut full = through.own;
        full.scale(Volume::NORMAL);
        introspect.set_sink_volume_by_name(&through.name, &through.own, None);
        introspect.set_sink_volume_by_name(device, &full, None);
        if state.default_output.as_deref() == Some(device) {
            context.set_default_sink(&through.name, |_| ());
        }
        let mut introspect = context.introspect();
        for stream in &state.playing {
            if stream.actual == device && stream.kept {
                introspect.move_sink_input_by_name(stream.index, &through.name, None);
            }
        }
    }

    /// The equalizer of the output named `device` is going: what plays through it goes back to
    /// the output, with its volume.
    pub fn release_equalizer(&self, device: &str) {
        let state = self.state.borrow().clone();
        let Some(output) = state.outputs.iter().find(|output| output.name == device) else {
            return;
        };
        let Some(through) = &output.through else {
            return;
        };
        let mut context = self.context.borrow_mut();
        let Some(context) = context.as_mut() else {
            return;
        };
        let mut introspect = context.introspect();
        introspect.set_sink_volume_by_name(device, &output.volumes, None);
        if state.default_output.as_deref() == Some(through.name.as_str()) {
            context.set_default_sink(device, |_| ());
        }
        let mut introspect = context.introspect();
        for stream in &state.playing {
            if stream.actual == through.name && stream.kept {
                introspect.move_sink_input_by_name(stream.index, device, None);
            }
        }
    }

    /// Sets the volume of an app's stream, from 0, keeping its balance.
    pub fn set_stream_volume(&self, kind: Kind, index: u32, volume: f64) {
        let Some(mut volumes) = self.stream(kind, index).map(|stream| stream.volumes) else {
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
                Kind::Output => introspect.set_sink_input_volume(index, &volumes, None),
                Kind::Input => introspect.set_source_output_volume(index, &volumes, None),
            };
        }
    }

    pub fn set_stream_muted(&self, kind: Kind, index: u32, muted: bool) {
        if let Some(context) = self.context.borrow_mut().as_mut() {
            let mut introspect = context.introspect();
            match kind {
                Kind::Output => introspect.set_sink_input_mute(index, muted, None),
                Kind::Input => introspect.set_source_output_mute(index, muted, None),
            };
        }
    }

    /// Moves an app's stream to the device named `device`, where the session manager keeps the
    /// app from then on.
    pub fn move_stream(&self, kind: Kind, index: u32, device: &str) {
        let Some(found) = self.find(kind, device) else {
            return;
        };
        let device = found
            .through
            .as_ref()
            .map_or(device, |through| &through.name);
        if let Some(context) = self.context.borrow_mut().as_mut() {
            let mut introspect = context.introspect();
            match kind {
                Kind::Output => introspect.move_sink_input_by_name(index, device, None),
                Kind::Input => introspect.move_source_output_by_name(index, device, None),
            };
        }
    }

    /// Makes an app's stream follow the default device, and the session manager forget where it
    /// was kept.
    pub fn follow_default(&self, kind: Kind, index: u32) {
        let Some(node) = self.stream(kind, index).and_then(|stream| stream.node) else {
            return;
        };
        if let Some(targets) = self.targets.borrow().as_ref() {
            targets.follow_default(node);
        }
    }

    /// Keeps an app's stream on the device it's on, even when the default changes.
    pub fn keep_here(&self, kind: Kind, index: u32) {
        let Some(stream) = self.stream(kind, index) else {
            return;
        };
        let device = self.find(kind, &stream.device);
        let serial = device.and_then(|device| match device.through {
            Some(through) if through.name == stream.actual => through.serial,
            _ => device.serial,
        });
        let (Some(node), Some(serial)) = (stream.node, serial) else {
            return;
        };
        if let Some(targets) = self.targets.borrow().as_ref() {
            targets.keep_on(node, &serial);
        }
    }

    /// The apps kept on the output named `device`, by their streams' indices.
    pub fn kept_on(&self, device: &str) -> Vec<u32> {
        let state = self.state.borrow();
        let streams = state.playing.iter();
        streams
            .filter(|stream| stream.kept && stream.actual == device)
            .map(|stream| stream.index)
            .collect()
    }

    fn stream(&self, kind: Kind, index: u32) -> Option<Stream> {
        let state = self.state.borrow();
        let list = match kind {
            Kind::Output => &state.playing,
            Kind::Input => &state.recording,
        };
        list.iter().find(|stream| stream.index == index).cloned()
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
        let name = self.volume_of(kind, name);
        if let Some(context) = self.context.borrow_mut().as_mut() {
            let mut introspect = context.introspect();
            match kind {
                Kind::Output => introspect.set_sink_volume_by_name(&name, &volumes, None),
                Kind::Input => introspect.set_source_volume_by_name(&name, &volumes, None),
            };
        }
    }

    /// The volume of the device named `name`, as it shows.
    pub fn volume(&self, kind: Kind, name: &str) -> Option<f64> {
        self.find(kind, name).map(|device| device.volume)
    }

    /// Whose volume is the device named `name`'s: its equalizer's, while it has one.
    fn volume_of(&self, kind: Kind, name: &str) -> String {
        let through = self.find(kind, name).and_then(|device| device.through);
        through.map_or_else(|| name.to_owned(), |through| through.name)
    }

    pub fn set_muted(&self, kind: Kind, name: &str, muted: bool) {
        let name = self.volume_of(kind, name);
        if let Some(context) = self.context.borrow_mut().as_mut() {
            let mut introspect = context.introspect();
            match kind {
                Kind::Output => introspect.set_sink_mute_by_name(&name, muted, None),
                Kind::Input => introspect.set_source_mute_by_name(&name, muted, None),
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
                        | InterestMaskSet::SINK_INPUT
                        | InterestMaskSet::SOURCE_OUTPUT
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
        let mut equalizers = Vec::new();
        introspect.get_sink_info_list(move |result| match result {
            ListResult::Item(info) => {
                if info.name.as_deref().is_some_and(hidden) {
                    return;
                }
                let is_equalizer = info
                    .name
                    .as_deref()
                    .is_some_and(|name| name.starts_with(equalizer::PREFIX));
                let port = info.active_port.as_ref();
                let label = info.description.as_deref().unwrap_or_default().to_owned();
                let is_virtual = !info.flags.contains(SinkFlagSet::HARDWARE);
                let (title, detail) = naming(
                    &info.proplist,
                    &label,
                    port.and_then(|port| port.name.as_deref()),
                    port.and_then(|port| port.description.as_deref()),
                    is_virtual,
                );
                let list = if is_equalizer {
                    &mut equalizers
                } else {
                    &mut outputs
                };
                list.push(Device {
                    name: info.name.as_deref().unwrap_or_default().to_owned(),
                    label,
                    title,
                    detail,
                    is_virtual,
                    icon: icon(
                        Kind::Output,
                        &info.proplist,
                        port.and_then(|port| port.name.as_deref()),
                    ),
                    active: false,
                    volume: fraction(info.volume.max()),
                    muted: info.mute,
                    volumes: info.volume,
                    index: info.index,
                    serial: info.proplist.get_str("object.serial"),
                    through: None,
                });
            }
            ListResult::End | ListResult::Error => {
                let mut reading = gathered.borrow_mut();
                reading.outputs = Some(std::mem::take(&mut outputs));
                reading.equalizers = std::mem::take(&mut equalizers);
                drop(reading);
                Self::finish(&weak, &gathered);
            }
        });

        let gathered = reading.clone();
        let weak = Rc::downgrade(self);
        let mut inputs = Vec::new();
        introspect.get_source_info_list(move |result| match result {
            ListResult::Item(info) => {
                // What an output plays can be recorded from its monitor, which isn't a device.
                if info.monitor_of_sink.is_some() || info.name.as_deref().is_some_and(hidden) {
                    return;
                }
                let port = info.active_port.as_ref();
                let label = info.description.as_deref().unwrap_or_default().to_owned();
                let is_virtual = !info.flags.contains(SourceFlagSet::HARDWARE);
                let (title, detail) = naming(
                    &info.proplist,
                    &label,
                    port.and_then(|port| port.name.as_deref()),
                    port.and_then(|port| port.description.as_deref()),
                    is_virtual,
                );
                inputs.push(Device {
                    name: info.name.as_deref().unwrap_or_default().to_owned(),
                    label,
                    title,
                    detail,
                    is_virtual,
                    icon: icon(
                        Kind::Input,
                        &info.proplist,
                        port.and_then(|port| port.name.as_deref()),
                    ),
                    active: false,
                    volume: fraction(info.volume.max()),
                    muted: info.mute,
                    volumes: info.volume,
                    index: info.index,
                    serial: info.proplist.get_str("object.serial"),
                    through: None,
                });
            }
            ListResult::End | ListResult::Error => {
                gathered.borrow_mut().inputs = Some(std::mem::take(&mut inputs));
                Self::finish(&weak, &gathered);
            }
        });

        let gathered = reading.clone();
        let weak = Rc::downgrade(self);
        let mut playing = Vec::new();
        introspect.get_sink_input_info_list(move |result| match result {
            ListResult::Item(info) => {
                if let Some(stream) = stream(
                    info.index,
                    &info.proplist,
                    info.volume,
                    info.mute,
                    info.corked,
                ) {
                    playing.push((stream, info.sink));
                }
            }
            ListResult::End | ListResult::Error => {
                gathered.borrow_mut().playing = Some(std::mem::take(&mut playing));
                Self::finish(&weak, &gathered);
            }
        });

        let gathered = reading.clone();
        let weak = Rc::downgrade(self);
        let mut recording = Vec::new();
        introspect.get_source_output_info_list(move |result| match result {
            ListResult::Item(info) => {
                if let Some(stream) = stream(
                    info.index,
                    &info.proplist,
                    info.volume,
                    info.mute,
                    info.corked,
                ) {
                    recording.push((stream, info.source));
                }
            }
            ListResult::End | ListResult::Error => {
                gathered.borrow_mut().recording = Some(std::mem::take(&mut recording));
                Self::finish(&weak, &gathered);
            }
        });
    }

    /// Publishes a read once all of its parts are in, from the main loop.
    fn finish(weak: &Weak<Self>, reading: &Rc<RefCell<Reading>>) {
        let done = {
            let reading = reading.borrow();
            reading.outputs.is_some()
                && reading.inputs.is_some()
                && reading.playing.is_some()
                && reading.recording.is_some()
        };
        if !done {
            return;
        }
        // The server answers in the order it was asked, so the defaults are in by now.
        let reading = std::mem::take(&mut *reading.borrow_mut());
        let weak = weak.clone();
        glib::idle_add_local_once(move || {
            let Some(this) = weak.upgrade() else { return };
            let mut outputs = reading.outputs.unwrap_or_default();
            let inputs = reading.inputs.unwrap_or_default();
            // An output with an equalizer has the equalizer's volume.
            for output in &mut outputs {
                let name = equalizer::sink(&output.name);
                if let Some(eq) = reading.equalizers.iter().find(|eq| eq.name == name) {
                    output.through = Some(Through {
                        name: eq.name.clone(),
                        index: eq.index,
                        serial: eq.serial.clone(),
                        own: output.volumes,
                    });
                    output.volume = eq.volume;
                    output.muted = eq.muted;
                    output.volumes = eq.volumes;
                }
            }
            // A stream is on a device by its index; one on what isn't a device, like an output's
            // monitor, isn't an app's to show.
            let on = |streams: Option<Vec<(Stream, u32)>>, devices: &[Device]| -> Vec<Stream> {
                let streams = streams.unwrap_or_default().into_iter();
                streams
                    .filter_map(|(mut stream, index)| {
                        // On an output's equalizer, it's on the output.
                        let device = devices.iter().find(|device| {
                            device.index == index
                                || device.through.as_ref().is_some_and(|t| t.index == index)
                        })?;
                        stream.device = device.name.clone();
                        stream.actual = match &device.through {
                            Some(through) if through.index == index => through.name.clone(),
                            _ => device.name.clone(),
                        };
                        Some(stream)
                    })
                    .collect()
            };
            let mut devices = Devices {
                playing: on(reading.playing, &outputs),
                recording: on(reading.recording, &inputs),
                default_output: reading.default_output.clone(),
                outputs,
                inputs,
                can_keep: false,
            };
            this.mark_kept(&mut devices);
            // The default being an output's equalizer is the output being the default.
            let default = reading
                .default_output
                .as_deref()
                .map(|name| name.strip_prefix(equalizer::PREFIX).unwrap_or(name));
            for device in &mut devices.outputs {
                device.active = Some(device.name.as_str()) == default;
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
        for listener in self.listeners.borrow().iter() {
            listener();
        }
    }

    /// Hears each change of the devices, after the one `start` was given.
    pub fn listen(&self, heard: impl Fn() + 'static) {
        self.listeners.borrow_mut().push(Box::new(heard));
    }
}

/// Whether the device named `name` is a part of something bento built, like the filter one output
/// of a combination plays through, rather than a device of its own.
fn hidden(name: &str) -> bool {
    name.starts_with(super::combine::CHAIN_PREFIX)
}

/// An app's stream, from what the sound server says of it, but for the sound server's own and
/// bento's, and level meters like pavucontrol's.
fn stream(
    index: u32,
    properties: &Proplist,
    volumes: ChannelVolumes,
    muted: bool,
    paused: bool,
) -> Option<Stream> {
    let property = |key: &str| properties.get_str(key).filter(|value| !value.is_empty());
    if property("media.name").as_deref() == Some("Peak detect")
        || property("node.name").is_some_and(|name| name.starts_with("bento."))
        || property("bento.internal").as_deref() == Some("true")
    {
        return None;
    }
    let app = property("application.name")
        .or_else(|| property("node.description"))
        .or_else(|| property("node.name"))?;
    let flag = |key: &str| property(key).as_deref() == Some("true");
    Some(Stream {
        index,
        app,
        icon: property("application.icon_name"),
        binary: property("application.process.binary"),
        title: property("media.name"),
        device: String::new(),
        volume: f64::from(volumes.max().0) / f64::from(Volume::NORMAL.0),
        muted,
        paused,
        kept: false,
        own: property("target.object").is_some() || property("node.target").is_some(),
        fixed: flag("node.dont-move"),
        volumes,
        node: property("object.id").and_then(|id| id.parse().ok()),
        actual: String::new(),
    })
}

/// A short title for a device, and what it is under it: a monitor's own name for its HDMI or
/// DisplayPort, a USB or Bluetooth device's own name, and a sound card's port, like "Line Out",
/// over the card. A virtual one keeps its description.
fn naming(
    properties: &Proplist,
    label: &str,
    port: Option<&str>,
    port_label: Option<&str>,
    is_virtual: bool,
) -> (String, String) {
    let property = |key: &str| properties.get_str(key).unwrap_or_default();
    title_of(
        label,
        &property("device.product.name"),
        &property("node.nick"),
        &property("device.bus"),
        &property("device.api"),
        port.unwrap_or_default(),
        port_label.unwrap_or_default(),
        is_virtual,
    )
}

#[allow(clippy::too_many_arguments)]
fn title_of(
    label: &str,
    product: &str,
    nick: &str,
    bus: &str,
    api: &str,
    port: &str,
    port_label: &str,
    is_virtual: bool,
) -> (String, String) {
    let product = if product.is_empty() { label } else { product };
    let port = port.to_lowercase();
    if is_virtual {
        return (label.to_owned(), "Virtual".to_owned());
    }
    if port.contains("hdmi") || port.contains("displayport") {
        // A monitor names itself; without, the port's own name says which.
        let title = if nick.is_empty() || nick == product {
            if port_label.is_empty() {
                label
            } else {
                port_label
            }
        } else {
            nick
        };
        return (title.to_owned(), product.to_owned());
    }
    if api == "bluez5" || bus == "bluetooth" {
        return (product.to_owned(), "Bluetooth".to_owned());
    }
    if bus == "usb" {
        return (product.to_owned(), "USB".to_owned());
    }
    if port_label.is_empty() {
        (label.to_owned(), String::new())
    } else {
        (port_label.to_owned(), product.to_owned())
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
    fn a_device_is_called_by_what_the_user_knows_it_as() {
        let title = |label, product, nick, bus, api, port, port_label, is_virtual| {
            title_of(label, product, nick, bus, api, port, port_label, is_virtual)
        };
        let both = |a: &str, b: &str| (a.to_owned(), b.to_owned());
        assert_eq!(
            title(
                "Navi 21/23 HDMI/DP Audio Controller Digital Stereo (HDMI 3)",
                "Navi 21/23 HDMI/DP Audio Controller",
                "BenQ EX240N",
                "pci",
                "alsa",
                "hdmi-output-2",
                "HDMI / DisplayPort 3",
                false
            ),
            both("BenQ EX240N", "Navi 21/23 HDMI/DP Audio Controller"),
            "a monitor by its own name"
        );
        assert_eq!(
            title(
                "G435 Wireless Gaming Headset Digital Stereo (IEC958)",
                "G435 Wireless Gaming Headset",
                "G435 Wireless Gaming Headset",
                "usb",
                "alsa",
                "iec958-stereo-output",
                "Digital Output (S/PDIF)",
                false
            ),
            both("G435 Wireless Gaming Headset", "USB"),
            "a USB device by its own name, not its port's"
        );
        assert_eq!(
            title(
                "Ryzen HD Audio Controller Analog Stereo",
                "Ryzen HD Audio Controller",
                "ALC897 Analog",
                "pci",
                "alsa",
                "analog-output-lineout",
                "Line Out",
                false
            ),
            both("Line Out", "Ryzen HD Audio Controller"),
            "a sound card by its port"
        );
        assert_eq!(
            title(
                "WH-1000XM4",
                "",
                "",
                "",
                "bluez5",
                "headphone-output",
                "Headphone",
                false
            ),
            both("WH-1000XM4", "Bluetooth")
        );
        assert_eq!(
            title("Easy Effects Sink", "", "", "", "", "", "", true),
            both("Easy Effects Sink", "Virtual")
        );
    }

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
