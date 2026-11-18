// The launcher, opened by its shortcut.

import {
  box, text, image, input, root, outputs, on, apps, launch, focus, focusedOutput, grow, shrink,
  opened, closed, preload,
} from "bento:ui";

// How big an app's icon is in a row.
const ICON = 40;
// The apps are read and their icons loaded ahead of the first search, which then has them in
// its first frame.
for (const app of apps()) preload(app.icon ?? "application-x-executable", ICON);

// The launcher: a glass search bar in the middle of the focused output, above the windows, that
// finds apps as you type, its results opening below it. It grows out of a point, and
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
      direction: "column", align: "center", justify: "center",
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

// What closes it, for whatever else opens or a press elsewhere.
const self = { close: () => closeLauncher() };

function openLauncher(output) {
  if (!output || !root(output)) return;
  opened(self);
  const kept = launchers.get(output);
  if (kept) {
    launcher = { ...kept, rows: kept.rows, matches: [], selected: 0, all: apps() };
    kept.field.setText("");
    focus(kept.field);
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
  fillLauncher("");
  grow(panel, { radius: LAUNCHER_RADIUS, content });
}

function closeLauncher() {
  if (!launcher) return;
  shrink(launcher.panel, { content: launcher.content });
  launchers.set(launcher.output, launcher);
  launcher = null;
  closed(self);
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
        image(app.icon ?? "application-x-executable", { size: ICON }),
        box({ direction: "column", grow: 1, shrink: 1, minWidth: 0 }, [
          text(app.name, { size: 16, weight: 600, lines: 1 }),
          app.comment && text(app.comment, { size: 13, color: "fg/0.6", lines: 1 }),
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
