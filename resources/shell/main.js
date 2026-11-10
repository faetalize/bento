// bento's own shell: three islands of glass along the top of each output. The left one shows the
// workspaces and the minimap, the middle one the clock, which opens into the calendar, and the
// right one the status, which opens into the control center, with notifications under it. While
// a window is fullscreen on their output the islands leave the screen, and the pointer at the
// top edge brings them back.

import {
  box, text, island, refill, root, outputs, every, on, style, reserve, action, after, cancel,
  last, stored, ISLAND, ACTIVE,
} from "bento:ui";
import { calendar } from "bento:calendar";
import { controlCenter } from "bento:control";
import { toastStack } from "bento:toasts";
import { minimap } from "bento:workspace";
import "bento:launcher";

// How near the top edge the pointer brings the islands back over a fullscreen window, and how
// long after it leaves them they go again, in milliseconds.
const EDGE = 2;
const LINGER = 400;
// bento's gaps: the islands sit that far from the screen's edges, and windows that far below.
let gaps = style().gaps ?? 8;

// What scripts keep between runs has to be there: a bento without it keeps the shell it has.
stored("pinnedOutputs");

const shells = new Map();
let workspaces = last("workspaces")?.outputs ?? {};
// Which outputs show a fullscreen window.
let fullscreen = last("fullscreen")?.outputs ?? {};

// Sends an output's islands off the top of the screen while a window is fullscreen there, unless
// the pointer is at them or something of theirs shows, a panel or a notification, and brings
// them back otherwise. Gone, they're left out, so nothing is drawn over the window.
function placeIslands(output) {
  const shell = shells.get(output);
  if (!shell) return;
  const busy = shell.calendar.panel.open || shell.control.busy() || shell.toasts.count() > 0;
  const away = Boolean(fullscreen[output]) && !shell.near && !busy;
  if (away === shell.away) return;
  shell.away = away;
  // The pointer brings them back at the screen's edge, and keeps them while it's on them.
  shell.zone.set({ height: away ? EDGE : gaps + ISLAND + gaps });
  // They slide up off the screen and are left out there, and slide back from there.
  if (away) {
    shell.bar.set({
      offset: [0, -(ISLAND + gaps)],
      onSettle: () => shell.away && shell.bar.set({ hidden: true }),
    });
  } else {
    shell.bar.set({ hidden: false, offset: [0, 0] });
  }
}

function workspaceButtons(output) {
  return (workspaces[output] ?? []).map((ws) => {
    const hovered = box(
      {
        minWidth: 26,
        height: 26,
        radius: 13,
        padding: [0, ws.name ? 9 : 0],
        align: "center",
        justify: "center",
        fill: ws.active ? ACTIVE : undefined,
        cursor: "pointer",
        onClick: () => action("focus-workspace", [], { workspace: ws.id }),
        onHover: ws.active ? undefined : ({ inside }) => hovered.set({ fill: inside ? "fg/0.12" : null }),
      },
      [text(ws.name ?? ws.index + 1, { weight: 700, tabular: true, color: ws.active ? "#ffffff" : "fg" })],
    );
    return hovered;
  });
}

function build(output) {
  const top = root(output);
  if (!top || shells.has(output)) return;
  const shell = { away: false, near: false };
  const place = () => placeIslands(output);

  // The workspaces' buttons, filled again as the workspaces change, and the minimap.
  shell.buttons = box({ gap: 2 });
  refill(shell.buttons, workspaceButtons(output));
  const left = island({ padding: [0, 10, 0, 5], gap: 8 }, [shell.buttons, minimap(output)]);
  shell.calendar = calendar(place);
  shell.control = controlCenter(output, place);
  shell.toasts = toastStack(output, place);

  const start = box({ grow: 1, width: 0, justify: "start" }, [left]);
  // Notifications come out under the right island, below its panel when it's open.
  const end = box({ grow: 1, width: 0, direction: "column", align: "end", gap: 8 }, [
    shell.control.island,
    shell.toasts.stack,
  ]);
  const bar = box(
    { position: "absolute", top: gaps, left: gaps, right: gaps, align: "start" },
    [start, shell.calendar.island, end],
  );
  top.add(bar);
  // Where the pointer keeps the islands on screen over a fullscreen window: they go a moment
  // after it leaves, so that brushing past the edge doesn't send them back and forth.
  const zone = box({
    position: "absolute", top: 0, left: 0, right: 0, height: gaps + ISLAND + gaps, animate: false,
    onNear: ({ inside }) => {
      if (shell.leaving !== undefined) cancel(shell.leaving);
      shell.leaving = undefined;
      if (inside) {
        shell.near = true;
        place();
      } else {
        shell.leaving = after(LINGER, () => {
          shell.near = false;
          place();
        });
      }
    },
  });
  top.add(zone);
  reserve(output, { top: gaps + ISLAND });
  Object.assign(shell, { bar, zone, left });
  shells.set(output, shell);
  place();
}

on("style", (event) => {
  gaps = event.style.gaps ?? gaps;
  for (const [output, shell] of shells) {
    shell.bar.set({ top: gaps, left: gaps, right: gaps });
    if (shell.away) shell.bar.set({ offset: [0, -(ISLAND + gaps)] });
    shell.zone.set({ height: shell.away ? EDGE : gaps + ISLAND + gaps });
    reserve(output, { top: gaps + ISLAND });
  }
});

on("fullscreen", (event) => {
  fullscreen = event.outputs;
  for (const output of shells.keys()) placeIslands(output);
});

for (const output of outputs()) build(output);
on("outputs", (event) => event.outputs.forEach(build));

on("workspaces", (event) => {
  workspaces = event.outputs;
  for (const [output, shell] of shells) refill(shell.buttons, workspaceButtons(output));
});

every(1000, () => {
  const now = new Date();
  for (const shell of shells.values()) shell.calendar.tick(now);
});
