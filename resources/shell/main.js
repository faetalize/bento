// bento's own shell: three islands of glass along the top of each output. The clock opens into
// the calendar.

import { box, text, root, outputs, every, on } from "bento:ui";

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

function clockLabel(now) {
  const time = `${pad(now.getHours())}:${pad(now.getMinutes())}`;
  return `${DAYS[now.getDay()]} ${now.getDate()} ${MONTHS[now.getMonth()]}   ${time}`;
}

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

  const now = new Date();
  const clock = text(clockLabel(now), { weight: 600, tabular: true, anchor: [0.5, 0] });
  const day = text(LONG_DAYS[now.getDay()], { size: 22, weight: 700 });
  const date = text(`${now.getDate()} ${LONG_MONTHS[now.getMonth()]} ${now.getFullYear()}`, {
    color: "fg/0.6",
  });
  const calendar = box(
    { direction: "column", gap: 4, position: "absolute", top: 18, left: 20, opacity: 0 },
    [day, date],
  );
  let open = false;
  const middle = island(
    {
      anchor: [0.5, 0],
      onClick: () => {
        open = !open;
        middle.set(
          open
            ? { width: 320, height: 110, radius: 24, align: "start" }
            : { width: "auto", height: HEIGHT, radius: HEIGHT / 2, align: "center" },
        );
        clock.set({ opacity: open ? 0 : 1 });
        calendar.set({ opacity: open ? 1 : 0 });
      },
    },
    [clock, calendar],
  );

  const right = island({ anchor: [1, 0] }, [text("Status", { weight: 600 })]);

  const side = (justify, child) => box({ grow: 1, width: 0, justify }, [child]);
  const bar = box(
    { position: "absolute", top: GAP, left: GAP, right: GAP, align: "start" },
    [side("start", left), middle, side("end", right)],
  );
  top.add(bar);
  shells.set(output, { left, buttons, clock, day, date });
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
    shell.clock.set({ text: clockLabel(now) });
  }
});
