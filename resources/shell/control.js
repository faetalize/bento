// The right island: the status icons, the clipboard's button and the sound output switcher. It
// opens into the control center, laid out like a phone's quick settings:
//
// - along the top, buttons for a screenshot, a screen recording, the settings and the power menu;
// - the volume, and the brightness of the island's own display;
// - tiles, two to a row, that switch Wi-Fi, Bluetooth, Do Not Disturb, the dark style, the power
//   mode and the microphone, each only when there's something behind it;
// - what's playing;
// - the notifications.
//
// What has more to it than a switch opens a page of its own in the panel, with a way back: a
// tile's arrow, the volume's and the brightness's arrows, and the power button. The island opens
// into the clipboard's history too.

import {
  box, text, image, input, button, slider, toggle, island, expand, refill, drawer, on, config,
  data, action, last, after, cancel, run, notify, stored, store, closeAll, focus, ACTIVE,
} from "bento:ui";
import { clipboardPanel } from "bento:clipboard";
import { quiet, volumeIcon, micIcon } from "bento:osd";
import {
  onNotifications, kept, doNotDisturb, setDoNotDisturb, unseenOnes, looked, dismiss,
  dismissAll, invoke, hasDefault, plain, ago,
} from "bento:notifications";

const WIDTH = 360;
const GAP = 8;
// A tile is half a row.
const TILE = (WIDTH - GAP) / 2;
// A slider beside its arrow.
const SLIDER = WIDTH - 32 - GAP;
// How many notifications show before "Show More", and the most listed.
const NOTES = 3;
const MOST_NOTES = 20;

// The sound devices, as the data program last told them.
const devices = () => last("audio-devices") ?? { outputs: [], inputs: [] };
const activeDevice = (kind) => devices()[kind].find((device) => device.active);

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
  const say = (summary, body, icon = "") => notify(summary, body, icon, { transient: true });
  if (pinned.length === 0) {
    say("Sound Output", "No outputs are pinned. Pin some in the control center.");
    return;
  }
  if (connected.length === 0) {
    say("Sound Output", "None of the pinned outputs are connected.");
    return;
  }
  const current = outputs.find((output) => output.active)?.name;
  const next = connected[(connected.indexOf(current) + 1) % connected.length];
  const device = outputs.find((output) => output.name === next);
  data("device", { kind: "output", name: next });
  say("Sound Output Switched", device.label, device.icon);
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

// A round button with an icon.
const round = (icon, onClick) =>
  button({ icon, onClick, width: 38, height: 38, radius: 19, fill: "fg/0.08" });

// The arrow that opens a page.
const more = (onClick, props = {}) =>
  button({ icon: "go-next-symbolic", onClick, width: 32, height: 28, radius: 14, ...props });

// A row of a page's list: an icon, a title and a word under it, and what's at its end.
function row({ icon, title, subtitle, lit = false, end = [], onClick }) {
  const idle = lit ? "fg/0.1" : null;
  const node = box(
    {
      align: "center", gap: 12, minHeight: 40, padding: [6, 6, 6, 12], radius: 12, fill: idle,
      cursor: onClick ? "pointer" : undefined,
      onClick,
      onHover: onClick ? ({ inside }) => node.set({ fill: inside ? "fg/0.14" : idle }) : undefined,
    },
    [
      icon ? image(icon, { color: lit ? "accent" : "fg" }) : null,
      box({ direction: "column", grow: 1, shrink: 1, minWidth: 0 }, [
        text(title, { weight: lit ? 700 : 400, lines: 1 }),
        subtitle ? text(subtitle, { size: 12, color: "fg/0.6", lines: 1 }) : null,
      ]),
      ...end.filter(Boolean),
    ],
  );
  return node;
}

// A dim line, for a list with nothing in it.
const note = (words) => box({ padding: [8, 12] }, [text(words, { color: "fg/0.6" })]);

// A tile: an icon, a title and a word on how it is. A click does `onClick`; its arrow, when it
// also has `onMore`, opens what's behind it, and without `onClick` the whole tile does.
// `show({ active, icon, subtitle })` shows how it is.
function tile({ icon, title, onClick, onMore }) {
  let lit = false;
  const glyph = image(icon, { size: 20, color: "fg" });
  const name = text(title, { weight: 700, size: 14, lines: 1 });
  const said = text("", { size: 12, lines: 1, color: "fg/0.7" });
  const idle = () => (lit ? ACTIVE : "fg/0.1");
  const node = box(
    {
      width: TILE, height: 60, radius: 18, padding: [0, 8, 0, 14], gap: 12, align: "center",
      shrink: 0, cursor: "pointer", fill: idle(),
      onClick: onClick ?? onMore,
      onHover: ({ inside }) => node.set({ fill: inside && !lit ? "fg/0.14" : idle() }),
    },
    [
      glyph,
      box({ direction: "column", grow: 1, shrink: 1, minWidth: 0 }, [name, said]),
      onClick && onMore ? more(onMore, { width: 30, height: 30, radius: 15 }) : null,
    ],
  );
  node.show = ({ active, icon: newIcon, subtitle } = {}) => {
    lit = Boolean(active);
    const ink = lit ? "#ffffff" : "fg";
    glyph.set({ color: ink, ...(newIcon ? { icon: newIcon } : {}) });
    name.set({ color: ink });
    said.set({ color: lit ? "#ffffff" : "fg/0.7", text: subtitle ?? "" });
    node.set({ fill: idle() });
  };
  return node;
}

// The icon for a Wi-Fi signal of `strength`, from 0 to 100.
const signalIcon = (strength) =>
  `network-wireless-signal-${
    strength > 75 ? "excellent" : strength > 50 ? "good" : strength > 25 ? "ok" : "weak"
  }-symbolic`;

const PROFILES = {
  "power-saver": ["Power Saver", "power-profile-power-saver-symbolic"],
  balanced: ["Balanced", "power-profile-balanced-symbolic"],
  performance: ["Performance", "power-profile-performance-symbolic"],
};
const profileName = (name) => PROFILES[name]?.[0] ?? name;
const profileIcon = (name) => PROFILES[name]?.[1] ?? "power-profile-balanced-symbolic";

// Whether the computer is online by cable.
const wired = () => {
  const network = last("network");
  return Boolean(network?.connected && network.icon?.includes("wired"));
};

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
  const quietIcon = image("notifications-disabled-symbolic", { color: "fg", display: "none" });
  const network = image("network-offline-symbolic", { color: "fg" });
  const volume = image("audio-volume-muted-symbolic", { color: "fg", display: "none" });
  const battery = image("battery-missing-symbolic", { color: "fg", display: "none" });
  const charge = text("", { weight: 600, tabular: true, display: "none" });
  // A dot while there are notifications that haven't been looked at.
  const dot = box({ width: 6, height: 6, radius: 3, fill: "accent", display: "none" });
  const status = [switcher, clipboard, quietIcon, network, volume, battery, charge, dot];
  const pill = island(
    // Held to its right edge, which stays put as the island grows to the left.
    {
      gap: 8, padding: [0, 14, 0, 6], justify: "end", cursor: "pointer",
      onClick: () => panel.toggle(),
    },
    status,
  );

  // Along the top: the battery, then the quick actions.
  const batteryWords = text("", { color: "fg/0.7", weight: 600, tabular: true });
  const top = box({ align: "center", gap: GAP }, [
    box({ grow: 1 }, [batteryWords]),
    round("camera-photo-symbolic", () => capture("screenshot")),
    round("media-record-symbolic", () => capture("screenRecord")),
    round("emblem-system-symbolic", () => {
      closeAll();
      action("spawn", ["bento-settings"]);
    }),
    round("system-shutdown-symbolic", () => openPage(powerPage)),
  ]);

  // The volume, with the sound page behind it.
  const level = (kind, icon, width) =>
    slider({
      width, icon,
      // What it changes shows here, not in the on-screen indicator.
      onChange: (value) => {
        const device = activeDevice(kind);
        quiet();
        if (device) data("device-volume", { kind: kind.slice(0, -1), name: device.name, value });
      },
      onIcon: () => {
        const device = activeDevice(kind);
        quiet();
        if (device) {
          data("device-mute", { kind: kind.slice(0, -1), name: device.name, value: !device.muted });
        }
      },
    });
  const outputLevel = level("outputs", "audio-volume-high-symbolic", SLIDER);
  const volumeRow = box({ align: "center", gap: GAP }, [
    outputLevel,
    more(() => openPage(soundPage)),
  ]);

  // The brightness of this island's display, or the first one's, with them all behind it.
  const displays = () => last("brightness")?.displays ?? [];
  const shownDisplay = () =>
    displays().find((display) => display.id === output) ?? displays()[0];
  const brightnessLevel = slider({
    width: SLIDER, icon: "display-brightness-symbolic",
    onChange: (value) => {
      const display = shownDisplay();
      if (display) data("brightness", { id: display.id, value });
    },
  });
  const brightnessRow = box({ align: "center", gap: GAP, display: "none" }, [
    brightnessLevel,
    more(() => openPage(displaysPage)),
  ]);

  // The tiles: what each does, whether there's something behind it, and how it is.
  const TILES = {
    wifi: {
      icon: "network-wireless-symbolic", title: "Wi-Fi",
      onClick: () => data("wifi-enable", { on: !last("wifi")?.enabled }),
      onMore: () => openPage(wifiPage),
      there: () => Boolean(last("wifi")?.available),
      how: () => {
        const wifi = last("wifi");
        const joined = wifi?.networks?.find((n) => n.active);
        return {
          active: Boolean(wifi?.enabled),
          icon: joined ? signalIcon(joined.strength) : "network-wireless-symbolic",
          subtitle: wifi?.joining ? "Connecting…" : joined ? joined.ssid
            : wired() ? "Ethernet" : wifi?.enabled ? "Not Connected" : "Off",
        };
      },
    },
    bluetooth: {
      icon: "bluetooth-symbolic", title: "Bluetooth",
      onClick: () => data("bluetooth-power", { on: !last("bluetooth")?.powered }),
      onMore: () => openPage(bluetoothPage),
      there: () => Boolean(last("bluetooth")?.available),
      how: () => {
        const bluetooth = last("bluetooth");
        const connected = bluetooth?.devices?.filter((d) => d.connected) ?? [];
        return {
          active: Boolean(bluetooth?.powered),
          subtitle: !bluetooth?.powered ? "Off"
            : connected.length === 1 ? connected[0].name
            : connected.length ? `${connected.length} Devices` : "On",
        };
      },
    },
    quiet: {
      icon: "notifications-disabled-symbolic", title: "Do Not Disturb",
      onClick: () => setDoNotDisturb(!doNotDisturb()),
      there: () => true,
      how: () => ({ active: doNotDisturb(), subtitle: doNotDisturb() ? "On" : "Off" }),
    },
    dark: {
      icon: "weather-clear-night-symbolic", title: "Dark Style",
      onClick: () => data("theme", { dark: !last("theme")?.dark }),
      there: () => Boolean(last("theme")),
      how: () => ({ active: Boolean(last("theme")?.dark), subtitle: last("theme")?.dark ? "On" : "Off" }),
    },
    profile: {
      icon: "power-profile-balanced-symbolic", title: "Power Mode",
      onMore: () => openPage(profilePage),
      there: () => Boolean(last("power-profile")?.available),
      how: () => {
        const active = last("power-profile")?.active ?? "balanced";
        return { active: active !== "balanced", icon: profileIcon(active), subtitle: profileName(active) };
      },
    },
    mic: {
      icon: "microphone-sensitivity-high-symbolic", title: "Microphone",
      onClick: () => {
        const device = activeDevice("inputs");
        if (device) data("device-mute", { kind: "input", name: device.name, value: !device.muted });
      },
      // The microphone's devices and level are on the sound page, behind the volume.
      there: () => devices().inputs.length > 0,
      how: () => {
        const mic = activeDevice("inputs");
        return {
          active: mic ? !mic.muted : false,
          icon: micIcon(mic ?? { muted: true }),
          subtitle: !mic ? "None" : mic.muted ? "Muted" : mic.label,
        };
      },
    },
  };
  const tilesBox = box({ direction: "column", gap: GAP });
  let tiles = {};
  let tiled = "";
  // Lays out the tiles there's something behind, two to a row, and shows how each is. A tile
  // that comes or goes lays them out again.
  const showTiles = () => {
    const shown = Object.keys(TILES).filter((key) => TILES[key].there());
    const key = shown.join(",");
    if (key !== tiled) {
      tiled = key;
      tiles = Object.fromEntries(shown.map((name) => [name, tile(TILES[name])]));
      const rows = [];
      for (let i = 0; i < shown.length; i += 2) {
        rows.push(box({ gap: GAP }, [tiles[shown[i]], tiles[shown[i + 1]] ?? box({ width: TILE })]));
      }
      refill(tilesBox, rows);
    }
    for (const [name, node] of Object.entries(tiles)) node.show(TILES[name].how());
    panel?.fit();
  };

  // What's playing.
  const media = box({ direction: "column", display: "none" });
  const showMedia = () => {
    const player = last("media")?.players?.[0];
    if (!player) {
      refill(media, []);
      media.set({ display: "none" });
      panel?.fit();
      return;
    }
    const control = (icon, act, can) =>
      button({
        icon, width: 34, height: 34, radius: 17, size: 18, opacity: can ? 1 : 0.4,
        onClick: () => can && data("media", { player: player.id, action: act }),
      });
    const cover = player.art
      ? box({ width: 48, height: 48, radius: 12, clip: true, shrink: 0 }, [
          image(player.art, { width: 48, height: 48 }),
        ])
      : box({
          width: 48, height: 48, radius: 12, fill: "fg/0.1", shrink: 0, align: "center",
          justify: "center",
        }, [image(player.icon ?? "audio-x-generic-symbolic", { size: 28, color: "fg" })]);
    refill(media, [
      box({ align: "center", gap: 10, padding: 10, radius: 18, fill: "fg/0.08" }, [
        cover,
        box({ direction: "column", grow: 1, shrink: 1, minWidth: 0 }, [
          text(player.title, { weight: 700, lines: 1 }),
          text(player.artist ?? player.name, { size: 12, color: "fg/0.7", lines: 1 }),
        ]),
        control("media-skip-backward-symbolic", "Previous", player.canPrevious),
        control(
          player.playing ? "media-playback-pause-symbolic" : "media-playback-start-symbolic",
          "PlayPause",
          player.canPlay,
        ),
        control("media-skip-forward-symbolic", "Next", player.canNext),
      ]),
    ]);
    media.set({ display: "flex" });
    panel?.fit();
  };

  // The notifications, the newest few, and the rest behind "Show More".
  let allNotes = false;
  const notesList = box({ direction: "column", gap: 6 });
  const clearNotes = button({
    label: "Clear All", height: 26, fill: "fg/0.08", onClick: () => dismissAll(),
  });
  const moreNotes = button({
    label: "Show More", height: 30, fill: "fg/0.08",
    onClick: () => {
      allNotes = true;
      showNotes();
    },
  });
  const noteCard = (n) => {
    const body = plain(n.body, 140);
    const actions = (n.actions ?? []).filter(([key]) => key !== "default");
    return box(
      {
        gap: 10, align: "start", padding: [10, 6, 10, 10], radius: 16, fill: "fg/0.08",
        cursor: hasDefault(n) ? "pointer" : undefined,
        onClick: hasDefault(n)
          ? () => {
              invoke(n.id, "default");
              closeAll();
            }
          : undefined,
      },
      [
        image(n.image ?? (n.appIcon || "dialog-information-symbolic"), { size: 28, color: "fg" }),
        box({ direction: "column", gap: 2, grow: 1, shrink: 1, minWidth: 0 }, [
          box({ gap: 6 }, [
            box({ grow: 1, shrink: 1, minWidth: 0 }, [
              text(n.app || "Notification", { size: 12, color: "fg/0.6", lines: 1 }),
            ]),
            text(ago(n.at), { size: 12, color: "fg/0.6", tabular: true }),
          ]),
          text(plain(n.summary, 80) || n.app || "Notification", { weight: 700, lines: 1 }),
          body ? text(body, { size: 13, color: "fg/0.75" }) : null,
          actions.length
            ? box({ gap: 6, padding: [6, 0, 0, 0] }, actions.map(([key, label]) =>
                button({
                  label, fill: "fg/0.1", grow: 1, shrink: 1, minWidth: 0,
                  onClick: () => invoke(n.id, key),
                })))
            : null,
        ]),
        button({
          icon: "window-close-symbolic", size: 14, width: 26, height: 26, radius: 13,
          onClick: () => dismiss(n.id),
        }),
      ],
    );
  };
  const showNotes = () => {
    const all = kept().slice(0, MOST_NOTES);
    const shown = allNotes ? all : all.slice(0, NOTES);
    drawer(notesList, shown.length ? shown.map(noteCard) : [note("No Notifications")]);
    clearNotes.set({ display: all.length ? "flex" : "none" });
    const hidden = all.length - shown.length;
    moreNotes.set({ display: hidden > 0 ? "flex" : "none" });
    moreNotes.show({ label: `Show ${hidden} More` });
    panel?.fit();
  };
  const notesSection = box({ direction: "column", gap: GAP }, [
    box({ align: "center", gap: GAP, height: 26 }, [
      box({ grow: 1 }, [heading("Notifications")]),
      clearNotes,
    ]),
    notesList,
    moreNotes,
  ]);

  const main = box({ direction: "column", gap: 14, width: WIDTH }, [
    top,
    box({ direction: "column", gap: GAP }, [volumeRow, brightnessRow]),
    tilesBox,
    media,
    notesSection,
  ]);

  // Pages, which take the main view's place, each with its title and a way back.
  const pages = box({ direction: "column", width: WIDTH, display: "none" });
  let current = null;
  const page = (title, { onSwitch } = {}) => {
    const body = box({ direction: "column", gap: 2 });
    const flip = onSwitch ? toggle({ onChange: onSwitch }) : null;
    const node = box({ direction: "column", gap: 10, display: "none" }, [
      box({ align: "center", gap: GAP }, [
        button({
          icon: "go-previous-symbolic", width: 32, height: 32, radius: 16,
          onClick: () => closePage(),
        }),
        box({ grow: 1 }, [text(title, { weight: 700, size: 16 })]),
        flip,
      ]),
      body,
    ]);
    pages.add(node);
    return { node, body, flip, refresh: () => {}, opened: () => {}, closed: () => {} };
  };
  const openPage = (shown) => {
    if (current === shown) return;
    current?.node.set({ display: "none" });
    current?.closed();
    current = shown;
    shown.refresh(true);
    shown.opened();
    main.set({ display: "none" });
    shown.node.set({ display: "flex" });
    pages.set({ display: "flex", opacity: 1, blur: 0, jump: { opacity: 0, blur: 6 } });
    panel.fit();
  };
  const closePage = (atOnce = false) => {
    if (!current) return;
    current.node.set({ display: "none" });
    current.closed();
    current = null;
    pages.set({ display: "none" });
    main.set({
      display: "flex", opacity: 1, blur: 0, ...(atOnce ? {} : { jump: { opacity: 0, blur: 6 } }),
    });
    panel.fit();
  };

  // Sound: the outputs, pinned or not, the microphone's level, and the inputs.
  const soundPage = page("Sound");
  const inputLevel = level("inputs", "microphone-sensitivity-high-symbolic", WIDTH);
  const outputList = box({ direction: "column", gap: 2 });
  const inputList = box({ direction: "column", gap: 2 });
  for (const node of [
    heading("Output"), outputList, box({ height: 10 }), heading("Input"), box({ height: 4 }),
    inputLevel, box({ height: 4 }), inputList,
  ]) {
    soundPage.body.add(node);
  }
  const deviceRows = (kind, list) =>
    list.map((device) => {
      const isPinned = pinned.includes(device.name);
      return row({
        icon: device.icon, title: device.label, lit: device.active,
        onClick: () => data("device", { kind, name: device.name }),
        end: kind === "output"
          ? [button({
              icon: "view-pin-symbolic", active: isPinned, height: 26, radius: 13,
              onClick: () => setPinned(device.name, !isPinned),
            })]
          : [],
      });
    });
  let soundShown = "";
  soundPage.refresh = () => {
    const { outputs, inputs } = devices();
    const key = JSON.stringify([
      outputs.map((d) => [d.name, d.label, d.icon, d.active]),
      inputs.map((d) => [d.name, d.label, d.icon, d.active]),
      pinned,
    ]);
    if (key === soundShown) return;
    soundShown = key;
    refill(outputList, outputs.length ? deviceRows("output", outputs) : [note("No Outputs")]);
    refill(inputList, inputs.length ? deviceRows("input", inputs) : [note("No Inputs")]);
    panel?.fit();
  };

  // Wi-Fi: the networks around, joined with a click, and with a password when one's new and
  // has one.
  const wifiPage = page("Wi-Fi", { onSwitch: (on) => data("wifi-enable", { on }) });
  // The network a password is asked for, and the field it's typed in.
  let asking = null;
  let password = null;
  let wifiError = "";
  const join = (ssid, typed) => {
    if (!ssid) return;
    wifiError = "";
    asking = null;
    data("wifi-join", typed ? { ssid, password: typed } : { ssid });
    wifiPage.refresh(true);
  };
  wifiPage.refresh = (force = false) => {
    const wifi = last("wifi");
    wifiPage.flip.show(Boolean(wifi?.enabled));
    // Not while a password's being typed, which a new list would take away.
    if (asking && !force) return;
    const rows = [];
    if (wired()) rows.push(row({ icon: last("network").icon, title: "Ethernet", subtitle: "Connected" }));
    if (!wifi?.enabled) rows.push(note("Wi-Fi is off."));
    else if (!wifi.networks.length) rows.push(note("Looking for networks…"));
    for (const network of wifi?.enabled ? wifi.networks : []) {
      const joining = wifi.joining === network.ssid;
      rows.push(row({
        icon: signalIcon(network.strength),
        title: network.ssid,
        subtitle: joining ? "Connecting…" : network.active ? "Connected"
          : network.known ? "Saved" : "",
        lit: network.active,
        onClick: network.active || joining ? undefined : () => {
          if (network.secure && !network.known) {
            asking = network.ssid;
            wifiPage.refresh(true);
            focus(password);
          } else {
            join(network.ssid);
          }
        },
        end: [
          network.secure ? image("system-lock-screen-symbolic", { size: 14, color: "fg/0.5" }) : null,
          network.active
            ? button({
                label: "Disconnect", height: 26, fill: "fg/0.08",
                onClick: () => data("wifi-disconnect"),
              })
            : null,
        ],
      }));
      if (asking === network.ssid) {
        password = input({
          placeholder: "Password", size: 14, onSubmit: (typed) => join(network.ssid, typed),
        });
        rows.push(box({ align: "center", gap: GAP, padding: [4, 0, 6, 12] }, [
          box({ grow: 1, shrink: 1, minWidth: 0, padding: [8, 12], radius: 12, fill: "fg/0.08" }, [
            password,
          ]),
          button({
            label: "Join", active: true, height: 34,
            onClick: () => join(network.ssid, password.text),
          }),
        ]));
      }
    }
    if (wifiError) rows.push(note(wifiError));
    drawer(wifiPage.body, rows);
    panel?.fit();
  };
  wifiPage.opened = () => {
    wifiError = "";
    data("wifi-scan");
  };
  wifiPage.closed = () => {
    asking = null;
  };

  // Bluetooth: the devices known and nearby, connected and disconnected with a click.
  const bluetoothPage = page("Bluetooth", { onSwitch: (on) => data("bluetooth-power", { on }) });
  let bluetoothError = "";
  bluetoothPage.refresh = () => {
    const bluetooth = last("bluetooth");
    bluetoothPage.flip.show(Boolean(bluetooth?.powered));
    const rows = [];
    if (!bluetooth?.powered) {
      rows.push(note("Bluetooth is off."));
    } else {
      const deviceRow = (device) => row({
        icon: device.icon, title: device.name, lit: device.connected,
        subtitle: device.connecting ? "Connecting…" : device.connected ? "Connected" : "",
        onClick: device.connecting ? undefined : () => data(
          device.connected ? "bluetooth-disconnect" : "bluetooth-connect",
          { path: device.path },
        ),
      });
      const known = bluetooth.devices.filter((d) => d.paired);
      const nearby = bluetooth.devices.filter((d) => !d.paired);
      rows.push(...(known.length ? known.map(deviceRow) : [note("No devices yet.")]));
      rows.push(box({ padding: [10, 12, 2, 12] }, [
        heading(bluetooth.discovering ? "Nearby · Looking…" : "Nearby"),
      ]));
      rows.push(...nearby.map(deviceRow));
    }
    if (bluetoothError) rows.push(note(bluetoothError));
    drawer(bluetoothPage.body, rows);
    panel?.fit();
  };
  bluetoothPage.opened = () => {
    bluetoothError = "";
    if (last("bluetooth")?.powered) data("bluetooth-scan", { on: true });
  };
  bluetoothPage.closed = () => data("bluetooth-scan", { on: false });

  // The power mode.
  const profilePage = page("Power Mode");
  profilePage.refresh = () => {
    const profile = last("power-profile");
    refill(profilePage.body, (profile?.profiles ?? []).map((name) => row({
      icon: profileIcon(name),
      title: profileName(name),
      lit: profile.active === name,
      onClick: () => data("power-profile", { name }),
    })));
    panel?.fit();
  };

  // Power: suspending at once; restarting, turning off and logging out on a second click.
  const powerPage = page("Power");
  let armed = null;
  let disarm;
  const POWER = [
    ["Suspend", "weather-clear-night-symbolic", () => data("suspend"), false],
    ["Restart", "system-reboot-symbolic", () => data("reboot"), true],
    ["Power Off", "system-shutdown-symbolic", () => data("power-off"), true],
    [
      "Log Out", "system-log-out-symbolic",
      () => action("quit", [], { properties: { "skip-confirmation": true } }), true,
    ],
  ];
  powerPage.refresh = () => {
    refill(powerPage.body, POWER.map(([title, icon, act, confirm]) => row({
      icon, title,
      subtitle: armed === title ? "Click again to confirm" : "",
      lit: armed === title,
      onClick: () => {
        if (confirm && armed !== title) {
          armed = title;
          if (disarm !== undefined) cancel(disarm);
          disarm = after(3000, () => {
            armed = null;
            if (current === powerPage) powerPage.refresh();
          });
          powerPage.refresh();
          return;
        }
        armed = null;
        closeAll();
        act();
      },
    })));
    panel?.fit();
  };
  powerPage.closed = () => {
    armed = null;
  };

  // Every display's brightness.
  const displaysPage = page("Displays");
  const displaySliders = new Map();
  displaysPage.refresh = () => {
    const all = displays();
    const ids = all.map((display) => display.id).join("\n");
    if (ids !== [...displaySliders.keys()].join("\n")) {
      displaySliders.clear();
      refill(displaysPage.body, all.map((display) => {
        const levelOf = slider({
          width: WIDTH, icon: "display-brightness-symbolic", value: display.value,
          onChange: (value) => data("brightness", { id: display.id, value }),
        });
        displaySliders.set(display.id, levelOf);
        return box({ direction: "column", gap: 4, padding: [0, 0, 8, 0] }, [
          text(display.label, { size: 12, color: "fg/0.7" }),
          levelOf,
        ]);
      }));
      panel?.fit();
    }
    for (const display of all) displaySliders.get(display.id)?.show(display.value);
  };

  const content = box({ direction: "column", width: WIDTH }, [main, pages]);
  // The island's own row gives way to whatever it opens into.
  const busy = () => pill.panels.some((other) => other.open);
  const showDot = () => dot.set({ display: unseenOnes() && !busy() ? "flex" : "none" });
  const toggled = () => {
    const away = busy();
    for (const node of status) node.set({ opacity: away ? 0 : 1, blur: away ? 4 : 0 });
    showDot();
    onToggle?.();
  };
  const panel = expand(pill, content, {
    hold: "end",
    onToggle: (open) => {
      if (open) {
        // It opens on its main view, with the notifications looked at.
        closePage(true);
        allNotes = false;
        showNotes();
        looked();
        // A monitor's own buttons change its brightness too.
        data("brightness-refresh");
      }
      toggled();
    },
  });
  const clips = clipboardPanel(output, pill, toggled);

  // What the data program says.
  const showDevices = () => {
    const out = activeDevice("outputs");
    const mic = activeDevice("inputs");
    volume.set({ icon: volumeIcon(out ?? { muted: true }), display: out ? "flex" : "none" });
    outputLevel.show(Math.min(out?.volume ?? 0, 1), volumeIcon(out ?? { muted: true }));
    inputLevel.show(Math.min(mic?.volume ?? 0, 1), micIcon(mic ?? { muted: true }));
    // The switcher shows while outputs are pinned, as the one that plays.
    switcher.set({ display: pinned.length > 0 ? "flex" : "none" });
    switcher.show({ icon: out?.icon ?? "audio-speakers-symbolic" });
    soundPage.refresh();
    showTiles();
  };
  const showBrightness = () => {
    const display = shownDisplay();
    brightnessRow.set({ display: display ? "flex" : "none" });
    if (display) brightnessLevel.show(display.value);
    displaysPage.refresh();
    panel?.fit();
  };
  const showWifi = () => {
    if (current === wifiPage) wifiPage.refresh();
    showTiles();
  };
  const showBluetooth = () => {
    if (current === bluetoothPage) bluetoothPage.refresh();
    showTiles();
  };
  const showProfile = () => {
    if (current === profilePage) profilePage.refresh();
    showTiles();
  };
  const showNetwork = () => {
    const state = last("network");
    if (state) network.set({ icon: state.icon });
    showWifi();
  };
  const showBattery = () => {
    const state = last("battery");
    const shown = state?.present ? "flex" : "none";
    const percent = `${Math.round(state?.percentage ?? 0)}%`;
    battery.set({ icon: state?.icon || "battery-missing-symbolic", display: shown });
    charge.set({ text: percent, display: shown });
    batteryWords.set({
      text: state?.present ? `${percent}${state.charging ? " · Charging" : ""}` : "",
    });
  };
  const showNotifications = () => {
    quietIcon.set({ display: doNotDisturb() ? "flex" : "none" });
    showDot();
    showTiles();
    if (panel.open && !current) {
      // Looked at as it comes, since the list is open.
      looked();
      showNotes();
    }
  };

  showDevices();
  showBrightness();
  showNetwork();
  showBluetooth();
  showProfile();
  showBattery();
  showMedia();
  showNotifications();
  on("audio-devices", showDevices);
  on("brightness", showBrightness);
  on("wifi", showWifi);
  on("wifi-error", (event) => {
    wifiError = event.message;
    if (current === wifiPage) wifiPage.refresh(true);
  });
  on("bluetooth", showBluetooth);
  on("bluetooth-error", (event) => {
    bluetoothError = event.message;
    if (current === bluetoothPage) bluetoothPage.refresh();
  });
  on("power-profile", showProfile);
  on("theme", showTiles);
  on("network", showNetwork);
  on("battery", showBattery);
  on("media", showMedia);
  onNotifications(showNotifications);
  pinListeners.push(() => {
    soundShown = "";
    showDevices();
  });

  return { island: pill, panel, busy };
}
