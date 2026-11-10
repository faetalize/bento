// The right island: the status icons, the clipboard's button and the sound output switcher,
// which opens into the control center: the light or dark style, the screenshot and screen record
// buttons, the sound devices and the displays' brightness. It opens into the clipboard's history
// too.

import {
  box, text, image, button, slider, island, expand, refill, drawer, on, config, data, action, last,
  after, run, notify, stored, store, closeAll,
} from "bento:ui";
import { clipboardPanel } from "bento:clipboard";
import { quiet, volumeIcon, micIcon } from "bento:osd";

const WIDTH = 340;

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

// The right island of `output`. Gives `{ island, panel, busy() }`: `panel` is the control
// center, and `busy()` says whether the island is open into anything. `onToggle()` hears it open
// or close into something.
export function controlCenter(output, onToggle) {
  // The island: the switcher and the clipboard's button, then the status.
  const switcher = button({
    icon: "audio-speakers-symbolic", display: "none",
    onClick: switchOutput,
    onRightClick: () => panel.setOpen(true),
  });
  const clipboard = button({ icon: "edit-paste-symbolic", onClick: () => clips.toggle() });
  const network = image("network-offline-symbolic", { color: "fg" });
  const volume = image("audio-volume-muted-symbolic", { color: "fg" });
  const battery = image("battery-missing-symbolic", { color: "fg", display: "none" });
  const charge = text("", { weight: 600, tabular: true, display: "none" });
  const pill = island(
    // Held to its right edge, which stays put as the island grows to the left.
    {
      gap: 8, padding: [0, 14, 0, 6], justify: "end", cursor: "pointer",
      onClick: () => panel.toggle(),
    },
    [switcher, clipboard, network, volume, battery, charge],
  );

  // The control center: quick buttons, sound, displays.
  // The quick buttons share the width, each its icon over its label.
  const TILE = {
    fill: "fg/0.1", direction: "column", gap: 4, height: 60, radius: 16, grow: 1, width: 0,
    size: 20,
  };
  const dark = button({
    icon: "weather-clear-night-symbolic", label: "Dark", ...TILE,
    onClick: () => data("theme", { dark: !dark.active }),
  });
  const quick = box({ gap: 8, width: WIDTH }, [
    dark,
    button({
      icon: "camera-photo-symbolic", label: "Screenshot", ...TILE,
      onClick: () => capture("screenshot"),
    }),
    button({
      icon: "media-record-symbolic", label: "Record", ...TILE,
      onClick: () => capture("screenRecord"),
    }),
  ]);

  const active = (kind) => devices()[kind].find((device) => device.active);
  const level = (kind, icon) =>
    slider({
      width: WIDTH, icon,
      // What it changes shows here, not in the on-screen indicator.
      onChange: (value) => {
        const device = active(kind);
        quiet();
        if (device) data("device-volume", { kind: kind.slice(0, -1), name: device.name, value });
      },
      onIcon: () => {
        const device = active(kind);
        quiet();
        if (device) {
          data("device-mute", { kind: kind.slice(0, -1), name: device.name, value: !device.muted });
        }
      },
    });
  const outputLevel = level("outputs", "audio-volume-high-symbolic");
  const inputLevel = level("inputs", "microphone-sensitivity-high-symbolic");
  const outputList = box({ direction: "column", gap: 2 });
  const inputList = box({ direction: "column", gap: 2 });

  // Each list shows the default device, and opens on a click into the others: a click on one
  // makes it the default and closes the list. An output's pin puts it in the switcher.
  const listOpen = { output: false, input: false };
  const deviceRow = (kind, device, { lead = false } = {}) => {
    const isPinned = pinned.includes(device.name);
    const others = devices()[`${kind}s`].length > 1;
    const lit = lead ? "fg/0.1" : null;
    const row = box(
      {
        align: "center", gap: 10, height: 34, padding: [0, 6, 0, 10], radius: 12,
        fill: lit, cursor: "pointer",
        onClick: () => {
          if (lead) {
            listOpen[kind] = !listOpen[kind];
          } else {
            listOpen[kind] = false;
            data("device", { kind, name: device.name });
          }
          showDevices(true);
        },
        onHover: ({ inside }) => row.set({ fill: inside ? "fg/0.14" : lit }),
      },
      [
        image(device.icon, { color: lead ? "accent" : "fg" }),
        box({ grow: 1, shrink: 1, minWidth: 0 }, [
          text(device.label, { weight: lead ? 700 : 400, lines: 1 }),
        ]),
        kind === "output"
          ? button({
              icon: "view-pin-symbolic", active: isPinned, height: 24, radius: 12,
              onClick: () => setPinned(device.name, !isPinned),
            })
          : null,
        lead && others
          ? image(listOpen[kind] ? "pan-up-symbolic" : "pan-down-symbolic", {
              color: "fg/0.6",
            })
          : null,
      ],
    );
    return row;
  };
  const deviceList = (kind, list) => {
    const active = list.find((device) => device.active) ?? list[0];
    if (!active) return [];
    const rest = listOpen[kind] ? list.filter((device) => device !== active) : [];
    return [
      deviceRow(kind, active, { lead: true }),
      ...rest.map((device) => deviceRow(kind, device)),
    ];
  };

  // What the lists show, to build them again only when that changes, and not as a volume moves.
  let listed = "";
  const showDevices = (force = false) => {
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
      listOpen,
    ]);
    if (key === listed && !force) return;
    listed = key;
    drawer(outputList, deviceList("output", outputs));
    drawer(inputList, deviceList("input", inputs));
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
  // The island's own row gives way to whatever it opens into.
  const status = [switcher, clipboard, network, volume, battery, charge];
  const busy = () => pill.panels.some((other) => other.open);
  const toggled = () => {
    const away = busy();
    for (const node of status) node.set({ opacity: away ? 0 : 1, blur: away ? 4 : 0 });
    onToggle?.();
  };
  const panel = expand(pill, content, {
    hold: "end",
    onToggle: (open) => {
      // The lists open closed.
      if (open && (listOpen.output || listOpen.input)) {
        listOpen.output = listOpen.input = false;
        showDevices(true);
      }
      // A monitor's own buttons change its brightness too.
      if (open) data("brightness-refresh");
      toggled();
    },
  });
  const clips = clipboardPanel(output, pill, toggled);


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
  on("audio-devices", () => showDevices());
  on("brightness", showBrightness);
  on("theme", showTheme);
  on("network", showNetwork);
  on("battery", showBattery);
  pinListeners.push(() => showDevices());

  return { island: pill, panel, busy };
}
