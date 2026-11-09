// The right island: the status icons, the clipboard's button and the sound output switcher,
// which opens into the control center: the light or dark style, the screenshot and screen record
// buttons, the sound devices and the displays' brightness.

import {
  box, text, image, button, slider, island, expand, refill, on, config, data, action, last,
  after, run, notify, stored, store, closeAll,
} from "bento:ui";

const WIDTH = 340;
// How much of a device's name a row shows.
const NAME_LENGTH = 36;

const short = (name) =>
  name.length > NAME_LENGTH ? `${name.slice(0, NAME_LENGTH - 1).trimEnd()}…` : name;

const volumeIcon = ({ volume, muted }) =>
  muted || volume <= 0 ? "audio-volume-muted-symbolic"
  : volume < 0.34 ? "audio-volume-low-symbolic"
  : volume < 0.67 ? "audio-volume-medium-symbolic"
  : "audio-volume-high-symbolic";

const micIcon = ({ muted }) =>
  muted ? "microphone-sensitivity-muted-symbolic" : "microphone-sensitivity-high-symbolic";

// The sound devices, as the data program last told them.
const devices = () => last("audio-devices") ?? { outputs: [], inputs: [] };

// The sound outputs pinned for the switcher, by their names, in the order they were pinned.
let pinned = stored("pinnedOutputs", []);
// What to tell when the pinned outputs change.
const pinListeners = [];

function setPinned(name, pin) {
  pinned = pinned.filter((other) => other !== name);
  if (pin) pinned.push(name);
  store("pinnedOutputs", pinned);
  for (const listener of pinListeners) listener();
}

// Makes the next pinned output the default one, going around, past the ones that aren't
// connected, and says which it is.
export function switchOutput() {
  const { outputs } = devices();
  const connected = pinned.filter((name) => outputs.some((output) => output.name === name));
  if (pinned.length === 0) {
    notify("Sound Output", "No outputs are pinned. Pin some in the control center.");
    return;
  }
  if (connected.length === 0) {
    notify("Sound Output", "None of the pinned outputs are connected.");
    return;
  }
  const current = outputs.find((output) => output.active)?.name;
  const next = connected[(connected.indexOf(current) + 1) % connected.length];
  const device = outputs.find((output) => output.name === next);
  data("device", { kind: "output", name: next });
  notify("Sound Output Switched", device.label, device.icon);
}
on("switch-audio-output", switchOutput);

// The clipboard manager isn't built: its button says so, and this is where it opens.
function openClipboard() {
  notify("Clipboard", "The clipboard manager isn't here yet.", "edit-paste-symbolic");
}

// Runs what the config sets for the screenshot or the screen record button, once the control
// center is out of the picture, and shows its notification when it's done, if it has one.
function capture(which) {
  const set = config()[which];
  closeAll();
  if (!set && which === "screenRecord") {
    // Nothing records the screen until a command is set.
    action("spawn", ["bento-settings", "control-center"]);
    return;
  }
  after(350, () => {
    if (!set) {
      action("screenshot");
      return;
    }
    const icon = which === "screenshot" ? "camera-photo-symbolic" : "media-record-symbolic";
    run(set.command, (code) => {
      if (set.notification && code === 0) notify(set.notification, set.body ?? "", icon);
    });
  });
}

// A section's title.
const heading = (title) => text(title, { size: 12, weight: 700, color: "fg/0.6" });

// The right island of an output. Gives `{ island, panel }`: `panel` is the control center,
// which `onToggle(open)` hears open and close.
export function controlCenter(onToggle) {
  // The island: the switcher and the clipboard's button, then the status.
  const switcher = button({
    icon: "audio-speakers-symbolic", display: "none",
    onClick: switchOutput,
    onRightClick: () => panel.setOpen(true),
  });
  const clipboard = button({ icon: "edit-paste-symbolic", onClick: openClipboard });
  const network = image("network-offline-symbolic", { color: "fg" });
  const volume = image("audio-volume-muted-symbolic", { color: "fg" });
  const battery = image("battery-missing-symbolic", { color: "fg", display: "none" });
  const charge = text("", { weight: 600, tabular: true, display: "none" });
  const pill = island(
    { gap: 8, padding: [0, 14, 0, 6], cursor: "pointer", onClick: () => panel.toggle() },
    [switcher, clipboard, network, volume, battery, charge],
  );

  // The control center: quick buttons, sound, displays.
  const dark = button({
    icon: "weather-clear-night-symbolic", label: "Dark", fill: "fg/0.1",
    onClick: () => data("theme", { dark: !dark.active }),
  });
  const quick = box({ gap: 8 }, [
    dark,
    button({
      icon: "camera-photo-symbolic", label: "Screenshot", fill: "fg/0.1",
      onClick: () => capture("screenshot"),
    }),
    button({
      icon: "media-record-symbolic", label: "Record", fill: "fg/0.1",
      onClick: () => capture("screenRecord"),
    }),
  ]);

  const active = (kind) => devices()[kind].find((device) => device.active);
  const level = (kind, icon) =>
    slider({
      width: WIDTH, icon,
      onChange: (value) => {
        const device = active(kind);
        if (device) data("device-volume", { kind: kind.slice(0, -1), name: device.name, value });
      },
      onIcon: () => {
        const device = active(kind);
        if (device) {
          data("device-mute", { kind: kind.slice(0, -1), name: device.name, value: !device.muted });
        }
      },
    });
  const outputLevel = level("outputs", "audio-volume-high-symbolic");
  const inputLevel = level("inputs", "microphone-sensitivity-high-symbolic");
  const outputList = box({ direction: "column", gap: 2 });
  const inputList = box({ direction: "column", gap: 2 });

  // A device's row: a click makes it the default one. An output's pin puts it in the switcher.
  const deviceRow = (kind, device) => {
    const isPinned = pinned.includes(device.name);
    const row = box(
      {
        align: "center", gap: 10, height: 34, padding: [0, 6, 0, 10], radius: 12,
        fill: device.active ? "fg/0.1" : null, cursor: "pointer",
        onClick: () => data("device", { kind, name: device.name }),
        onHover: ({ inside }) =>
          row.set({ fill: device.active ? "fg/0.1" : inside ? "fg/0.06" : null }),
      },
      [
        image(device.icon, { color: device.active ? "accent" : "fg" }),
        box({ grow: 1, shrink: 1 }, [
          text(short(device.label), { weight: device.active ? 700 : 400 }),
        ]),
        kind === "output"
          ? button({
              icon: "view-pin-symbolic", active: isPinned, height: 24, radius: 12,
              onClick: () => setPinned(device.name, !isPinned),
            })
          : null,
      ],
    );
    return row;
  };

  // What the lists show, to build them again only when that changes, and not as a volume moves.
  let listed = "";
  const showDevices = () => {
    const { outputs, inputs } = devices();
    const output = outputs.find((device) => device.active);
    const input = inputs.find((device) => device.active);
    volume.set({ icon: volumeIcon(output ?? { muted: true }), display: output ? "flex" : "none" });
    outputLevel.show(Math.min(output?.volume ?? 0, 1), volumeIcon(output ?? { muted: true }));
    inputLevel.show(Math.min(input?.volume ?? 0, 1), micIcon(input ?? { muted: true }));

    // The switcher shows while outputs are pinned, as the one that plays.
    switcher.set({ display: pinned.length > 0 ? "flex" : "none" });
    switcher.show({ icon: output?.icon ?? "audio-speakers-symbolic" });

    const key = JSON.stringify([
      outputs.map((d) => [d.name, d.label, d.icon, d.active]),
      inputs.map((d) => [d.name, d.label, d.icon, d.active]),
      pinned,
    ]);
    if (key === listed) return;
    listed = key;
    refill(outputList, outputs.map((device) => deviceRow("output", device)));
    refill(inputList, inputs.map((device) => deviceRow("input", device)));
    panel.fit();
  };

  // The displays' brightness, a slider each, which stay as long as the displays do.
  const displayList = box({ direction: "column", gap: 8 });
  const displaySection = box({ direction: "column", gap: 8, display: "none" }, [
    heading("Displays"),
    displayList,
  ]);
  const sliders = new Map();
  const showBrightness = () => {
    const displays = last("brightness")?.displays ?? [];
    const ids = displays.map((display) => display.id).join("\n");
    if (ids !== [...sliders.keys()].join("\n")) {
      sliders.clear();
      refill(displayList, displays.map((display) => {
        const level = slider({
          width: WIDTH, icon: "display-brightness-symbolic", value: display.value,
          onChange: (value) => data("brightness", { id: display.id, value }),
        });
        sliders.set(display.id, level);
        return box({ direction: "column", gap: 4 }, [
          displays.length > 1 ? text(display.label, { size: 12, color: "fg/0.7" }) : null,
          level,
        ]);
      }));
      displaySection.set({ display: displays.length > 0 ? "flex" : "none" });
      panel.fit();
    }
    for (const display of displays) sliders.get(display.id)?.show(display.value);
  };

  const content = box({ direction: "column", gap: 14, width: WIDTH }, [
    quick,
    box({ direction: "column", gap: 8 }, [
      heading("Sound"),
      outputLevel,
      outputList,
    ]),
    box({ direction: "column", gap: 8 }, [
      heading("Microphone"),
      inputLevel,
      inputList,
    ]),
    displaySection,
  ]);
  const status = [switcher, clipboard, network, volume, battery, charge];
  const panel = expand(pill, content, {
    onToggle: (open) => {
      // The island's own row gives way to the panel.
      for (const node of status) node.set({ opacity: open ? 0 : 1 });
      // A monitor's own buttons change its brightness too.
      if (open) data("brightness-refresh");
      onToggle?.(open);
    },
  });

  const showTheme = () => dark.show({ active: Boolean(last("theme")?.dark) });
  const showNetwork = () => {
    const state = last("network");
    if (state) network.set({ icon: state.icon });
  };
  const showBattery = () => {
    const state = last("battery");
    const shown = state?.present ? "flex" : "none";
    battery.set({ icon: state?.icon || "battery-missing-symbolic", display: shown });
    charge.set({ text: `${Math.round(state?.percentage ?? 0)}%`, display: shown });
  };
  showDevices();
  showBrightness();
  showTheme();
  showNetwork();
  showBattery();
  on("audio-devices", showDevices);
  on("brightness", showBrightness);
  on("theme", showTheme);
  on("network", showNetwork);
  on("battery", showBattery);
  pinListeners.push(showDevices);

  return { island: pill, panel };
}
