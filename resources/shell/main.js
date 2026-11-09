// bento's own shell: three islands of glass along the top of each output. The clock opens into
// the calendar.

import {
  box, text, image, input, root, outputs, every, on, morph, style, reserve, keyboard, action,
  apps, launch, focus, focusedOutput, grow, shrink,
} from "bento:ui";

const HEIGHT = 36;
// bento's gaps: the islands sit that far from the screen's edges, and windows that far below.
let gaps = style().gaps ?? 8;
const DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const LONG_DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const LONG_MONTHS = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
];

const pad = (n) => String(n).padStart(2, "0");

const clockTime = (now) => `${pad(now.getHours())}:${pad(now.getMinutes())}`;
const clockDate = (now) => `${DAYS[now.getDay()]} ${now.getDate()} ${MONTHS[now.getMonth()]}`;
const longDateLabel = (now) =>
  `${LONG_DAYS[now.getDay()]}, ${now.getDate()} ${LONG_MONTHS[now.getMonth()]} ${now.getFullYear()}`;

function island(props, children) {
  return box(
    {
      material: "glass",
      radius: HEIGHT / 2,
      height: HEIGHT,
      padding: [0, 14],
      align: "center",
      gap: 6,
      clip: true,
      ...props,
    },
    children,
  );
}

const shells = new Map();
let workspaces = {};

function workspaceButtons(output) {
  return (workspaces[output] ?? []).map((ws) => {
    const hovered = box(
      {
        width: 26,
        height: 26,
        radius: 13,
        align: "center",
        justify: "center",
        fill: ws.active ? "accent" : undefined,
        onClick: () => action("focus-workspace", [], { workspace: ws.id }),
        onHover: ws.active ? undefined : ({ inside }) => hovered.set({ fill: inside ? "fg/0.12" : null }),
      },
      [text(ws.name ?? ws.index + 1, { weight: 700, tabular: true, color: ws.active ? "#ffffff" : "fg" })],
    );
    return hovered;
  });
}

// Open panels close on Escape, and on a press anywhere off the shell.
const closeAll = () => {
  shells.forEach((shell) => shell.setOpen(false));
  closeLauncher();
};
on("outside", closeAll);
on("key", (event) => {
  if (event.pressed && event.key === "Escape") closeAll();
});

function build(output) {
  const top = root(output);
  if (!top || shells.has(output)) return;

  const buttons = workspaceButtons(output);
  const left = island({ padding: [0, 5], gap: 2 }, buttons);

  // The clock opens into the calendar: the time and the date travel to their places in it,
  // blurring out as their larger selves sharpen in.
  const now = new Date();
  const time = text(clockTime(now), { weight: 600, tabular: true });
  const shortDate = text(clockDate(now), { weight: 600 });
  const bigTime = text(clockTime(now), { size: 40, weight: 700, tabular: true });
  const longDate = text(longDateLabel(now), { color: "fg/0.6" });
  const calendar = box(
    { direction: "column", gap: 2, position: "absolute", top: 18, left: 22 },
    [bigTime, longDate],
  );
  for (const node of [bigTime, longDate]) node.set({ opacity: 0 });

  const swap = (from, to) => {
    from.forEach((node, i) => morph(node, to[i]));
    for (const node of from) node.set({ opacity: 0, blur: 4 });
    for (const node of to) node.set({ opacity: 1, blur: 0, jump: { blur: 4 } });
  };
  const shell = { open: false };
  // Opens the clock into the calendar, or closes it. While it's open, the shell has the keyboard,
  // for Escape.
  shell.setOpen = (open) => {
    if (shell.open === open) return;
    shell.open = open;
    middle.set(
      open
        ? { width: 320, height: 104, radius: 24 }
        : { width: "auto", height: HEIGHT, radius: HEIGHT / 2 },
    );
    if (open) swap([time, shortDate], [bigTime, longDate]);
    else swap([bigTime, longDate], [time, shortDate]);
    updateKeyboard();
  };
  const middle = island({ gap: 10, onClick: () => shell.setOpen(!shell.open) }, [
    shortDate,
    time,
    calendar,
  ]);

  // The status: network, sound and the battery, as the data program tells them.
  const network = image("network-offline-symbolic", { color: "fg" });
  const volume = image("audio-volume-muted-symbolic", { color: "fg" });
  const battery = image("battery-missing-symbolic", { color: "fg", display: "none" });
  const charge = text("", { weight: 600, tabular: true, display: "none" });
  const right = island({ gap: 8 }, [network, volume, battery, charge]);

  const side = (justify, child) => box({ grow: 1, width: 0, justify }, [child]);
  const bar = box(
    { position: "absolute", top: gaps, left: gaps, right: gaps, align: "start" },
    [side("start", left), middle, side("end", right)],
  );
  top.add(bar);
  reserve(output, { top: gaps + HEIGHT });
  Object.assign(shell, {
    bar, left, buttons, time, shortDate, bigTime, longDate, network, volume, battery, charge,
  });
  shells.set(output, shell);
}

on("style", (event) => {
  gaps = event.style.gaps ?? gaps;
  for (const [output, shell] of shells) {
    shell.bar.set({ top: gaps, left: gaps, right: gaps });
    reserve(output, { top: gaps + HEIGHT });
  }
});

for (const output of outputs()) build(output);
on("outputs", (event) => event.outputs.forEach(build));

on("workspaces", (event) => {
  workspaces = event.outputs;
  for (const [output, shell] of shells) {
    shell.buttons.forEach((button) => button.remove());
    shell.buttons = workspaceButtons(output);
    shell.buttons.forEach((button) => shell.left.add(button));
  }
});

every(1000, () => {
  const now = new Date();
  for (const shell of shells.values()) {
    shell.time.set({ text: clockTime(now) });
    shell.shortDate.set({ text: clockDate(now) });
    shell.bigTime.set({ text: clockTime(now) });
    shell.longDate.set({ text: longDateLabel(now) });
  }
});

const volumeIcon = ({ volume, muted }) =>
  muted || volume <= 0 ? "audio-volume-muted-symbolic"
  : volume < 0.34 ? "audio-volume-low-symbolic"
  : volume < 0.67 ? "audio-volume-medium-symbolic"
  : "audio-volume-high-symbolic";

on("network", (event) => shells.forEach((s) => s.network.set({ icon: event.icon })));
on("audio", (event) =>
  shells.forEach((s) =>
    s.volume.set({ icon: volumeIcon(event), display: event.available ? "flex" : "none" }),
  ),
);
on("battery", (event) =>
  shells.forEach((s) => {
    const shown = event.present ? "flex" : "none";
    s.battery.set({ icon: event.icon || "battery-missing-symbolic", display: shown });
    s.charge.set({ text: `${Math.round(event.percentage)}%`, display: shown });
  }),
);

// The shell has the keyboard while anything that takes keys is open.
function updateKeyboard() {
  keyboard(launcher !== null || [...shells.values()].some((s) => s.open));
}

// The launcher: a glass search bar in the middle of the focused output, above everything,
// that finds apps as you type, its results opening below it. It grows out of a point, and
// shrinks back into one.
const RESULTS = 8;
// How tall the launcher is with all its results.
const LAUNCHER_HEIGHT = 544;
const LAUNCHER_RADIUS = 26;
let launcher = null;
// Each output's frame for the launcher, which keeps it in the middle.
const launcherFrames = new Map();

// How well an app matches a query, lower is better, or null if it doesn't.
function score(app, query) {
  if (!query) return 0;
  const name = app.name.toLowerCase();
  if (name.startsWith(query)) return 0;
  if (name.split(/\s+/).some((word) => word.startsWith(query))) return 1;
  if (name.includes(query)) return 2;
  const extra = [app.generic, app.comment, ...app.keywords].filter(Boolean);
  if (extra.some((word) => word.toLowerCase().includes(query))) return 3;
  return null;
}

function launcherFrame(output) {
  if (!launcherFrames.has(output)) {
    // In the middle of the output at its tallest, with all its results, so its top, with the
    // search, stays where it is as the results come and go.
    const frame = box({
      position: "absolute", top: 0, left: 0, right: 0, bottom: 0,
      layer: "overlay", direction: "column", align: "center", justify: "center",
    });
    frame.room = box({ height: LAUNCHER_HEIGHT, direction: "column", align: "center" });
    frame.add(frame.room);
    root(output)?.add(frame);
    launcherFrames.set(output, frame);
  }
  return launcherFrames.get(output);
}

// Each output's launcher, kept while hidden, so that opening it again while it shrinks away
// turns it around from where it is.
const launchers = new Map();

function openLauncher(output) {
  if (!output || !root(output)) return;
  const kept = launchers.get(output);
  if (kept) {
    launcher = { ...kept, rows: kept.rows, matches: [], selected: 0, all: apps() };
    kept.field.setText("");
    focus(kept.field);
    updateKeyboard();
    fillLauncher("");
    grow(kept.panel, { radius: LAUNCHER_RADIUS, content: kept.content });
    return;
  }
  const field = input({
    placeholder: "Search apps",
    size: 22,
    onChange: (query) => fillLauncher(query),
    onSubmit: () => launchSelected(),
  });
  const list = box({ direction: "column", gap: 2 });
  const search = box({ padding: [10, 14] }, [field]);
  // One body, so that it grows around the panel's middle as one.
  const body = box({ direction: "column" }, [search, list]);
  const panel = box(
    {
      material: "glass", radius: LAUNCHER_RADIUS, width: 720, padding: 12, direction: "column",
      clip: true,
    },
    [body],
  );
  launcherFrame(output).room.add(panel);
  const content = [body];
  launcher = {
    output, panel, body, field, list, content, rows: [], matches: [], selected: 0, all: apps(),
  };
  launchers.set(output, launcher);
  focus(field);
  updateKeyboard();
  fillLauncher("");
  grow(panel, { radius: LAUNCHER_RADIUS, content });
}

function closeLauncher() {
  if (!launcher) return;
  shrink(launcher.panel, { content: launcher.content });
  launchers.set(launcher.output, launcher);
  launcher = null;
  focus(null);
  updateKeyboard();
}

function fillLauncher(query) {
  if (!launcher) return;
  const q = query.trim().toLowerCase();
  // Only the bar, until there's something to find.
  launcher.matches = !q ? [] : launcher.all
    .map((app) => [score(app, q), app])
    .filter(([s]) => s !== null)
    .sort((a, b) => a[0] - b[0])
    .slice(0, RESULTS)
    .map(([, app]) => app);
  launcher.selected = 0;
  launcher.rows.forEach((row) => row.remove());
  launcher.rows = launcher.matches.map((app, i) =>
    box(
      {
        align: "center", gap: 14, padding: [8, 12], radius: 16,
        onClick: () => launchApp(app),
        onHover: ({ inside }) => inside && selectRow(i),
      },
      [
        image(app.icon ?? "application-x-executable", { size: 40 }),
        box({ direction: "column", shrink: 1 }, [
          text(app.name, { size: 16, weight: 600 }),
          app.comment && text(app.comment, { size: 13, color: "fg/0.6" }),
        ]),
      ],
    ),
  );
  launcher.rows.forEach((row) => launcher.list.add(row));
  launcher.body.set({ gap: launcher.rows.length ? 8 : 0 });
  selectRow(0);
}

function selectRow(index) {
  if (!launcher || launcher.rows.length === 0) return;
  launcher.selected = (index + launcher.rows.length) % launcher.rows.length;
  launcher.rows.forEach((row, i) =>
    row.set({ fill: i === launcher.selected ? "fg/0.1" : null }),
  );
}

function launchApp(app) {
  launch(app.id);
  closeLauncher();
}

function launchSelected() {
  const app = launcher?.matches[launcher.selected];
  if (app) launchApp(app);
}

on("launcher", (event) => {
  if (launcher) closeLauncher();
  else openLauncher(event.output ?? focusedOutput() ?? outputs()[0]);
});

on("key", (event) => {
  if (!launcher || !event.pressed) return;
  if (event.key === "Down" || (event.key === "Tab" && !event.shift)) {
    selectRow(launcher.selected + 1);
  } else if (event.key === "Up" || event.key === "ISO_Left_Tab") {
    selectRow(launcher.selected - 1);
  }
});
