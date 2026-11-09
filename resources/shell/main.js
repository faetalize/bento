// bento's own shell: three islands of glass along the top of each output. The clock opens into
// the calendar.

import { box, text, root, outputs, every, on, morph } from "bento:ui";

const HEIGHT = 36;
const GAP = 8;
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
  return (workspaces[output] ?? []).map((ws) =>
    box(
      {
        width: 26,
        height: 26,
        radius: 13,
        align: "center",
        justify: "center",
        fill: ws.active ? "accent" : undefined,
      },
      [text(ws.name ?? ws.index + 1, { weight: 700, tabular: true, color: ws.active ? "#ffffff" : "fg" })],
    ),
  );
}

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

  let open = false;
  const swap = (from, to) => {
    from.forEach((node, i) => morph(node, to[i]));
    for (const node of from) node.set({ opacity: 0, blur: 4 });
    for (const node of to) node.set({ opacity: 1, blur: 0, jump: { blur: 4 } });
  };
  const middle = island(
    {
      gap: 10,
      onClick: () => {
        open = !open;
        middle.set(
          open
            ? { width: 320, height: 104, radius: 24 }
            : { width: "auto", height: HEIGHT, radius: HEIGHT / 2 },
        );
        if (open) swap([time, shortDate], [bigTime, longDate]);
        else swap([bigTime, longDate], [time, shortDate]);
      },
    },
    [shortDate, time, calendar],
  );

  const right = island({}, [text("Status", { weight: 600 })]);

  const side = (justify, child) => box({ grow: 1, width: 0, justify }, [child]);
  const bar = box(
    { position: "absolute", top: GAP, left: GAP, right: GAP, align: "start" },
    [side("start", left), middle, side("end", right)],
  );
  top.add(bar);
  shells.set(output, { left, buttons, time, shortDate, bigTime, longDate });
}

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
