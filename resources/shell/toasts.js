// Notifications, as toasts under the right island of the output that has the focus: each comes
// out of the island and goes back up into it. A click does what the notification is for, its
// buttons do its other actions, and the pointer on it keeps it. Once its time is up the toast
// goes and the notification stays in the control center, unless it's transient. With Do Not
// Disturb on, only critical ones show.

import {
  box, text, image, button, on, data, after, cancel, focusedOutput, grow, shrink, ISLAND,
} from "bento:ui";
import { doNotDisturb, dismiss, invoke, hasDefault, plain } from "bento:notifications";

const WIDTH = 340;
const RADIUS = 18;
// How long a toast that doesn't say stays, in milliseconds.
const STAY = 5000;

// Each output's stack of toasts, and what to tell when it gets or loses one.
const stacks = new Map();
// The toasts showing, by their notification's id.
const showing = new Map();

// The stack of toasts for output `output`, to put under its right island. `onChange` hears a
// toast come or go; `count()` says how many show. While `held()`, as while the control center
// lists the notifications itself, none come.
export function toastStack(output, onChange, held = () => false) {
  const stack = box({ direction: "column", align: "end", gap: 8 });
  const entry = { stack, onChange, held, count: () => count(output) };
  stacks.set(output, entry);
  return entry;
}

function count(output) {
  return [...showing.values()].filter((toast) => toast.output === output).length;
}

function show(notification) {
  hide(notification.id, true);
  const output = stacks.has(focusedOutput()) ? focusedOutput() : stacks.keys().next().value;
  const entry = stacks.get(output);
  if (!entry || entry.held()) return;
  if (doNotDisturb() && notification.urgency !== 2) return;

  const picture = notification.image ?? notification.appIcon;
  const body = plain(notification.body);
  const words = box({ direction: "column", gap: 2, shrink: 1, width: WIDTH - 24 - 32 - 12 }, [
    text(plain(notification.summary) || notification.app || "Notification", {
      weight: 700, lines: 1,
    }),
    body ? text(body, { size: 13, color: "fg/0.7" }) : null,
  ]);
  // The app's actions, but its default one, which is the toast's own click.
  const actions = (notification.actions ?? []).filter(([key]) => key !== "default");
  const buttons = actions.length
    ? box({ gap: 6, justify: "end", padding: [10, 0, 0, 0] }, actions.map(([key, label]) =>
        button({
          label, fill: "fg/0.1", grow: 1, shrink: 1, minWidth: 0, onClick: () => invoke(notification.id, key),
        })))
    : null;
  const content = box({ direction: "column" }, [
    box({ gap: 12, align: "start" }, [
      image(picture || "dialog-information-symbolic", { size: 32, color: "fg" }),
      words,
    ]),
    buttons,
  ]);
  const toast = box(
    {
      material: "glass", radius: RADIUS, width: WIDTH, padding: 12, clip: true, cursor: "pointer",
      // A click does what the notification is for, or sends it away if it isn't for anything.
      onClick: () => {
        if (hasDefault(notification)) invoke(notification.id, "default");
        else dismiss(notification.id);
      },
      onRightClick: () => dismiss(notification.id),
      // Under the pointer, its buttons too, it stays; once the pointer leaves, its time starts
      // again.
      onNear: ({ inside }) => {
        const shown = showing.get(notification.id);
        if (!shown) return;
        if (shown.timer !== undefined) cancel(shown.timer);
        shown.timer = inside || shown.stays ? undefined : timeUp(notification, shown.stay);
      },
    },
    [content],
  );
  entry.stack.add(toast);
  // It comes down out of the island, whose glass it melts with on the way.
  toast.set({ offset: [0, 0], jump: { offset: [0, -(ISLAND + 8)] } });
  grow(toast, { radius: RADIUS, content: [content] });

  // One that's critical stays until it's dealt with, and so does one that says to.
  const stay = notification.timeout > 0 ? notification.timeout : STAY;
  const stays = notification.urgency === 2 || notification.timeout === 0;
  const timer = stays ? undefined : timeUp(notification, stay);
  showing.set(notification.id, { toast, content, output, timer, stay, stays });
  entry.onChange?.();
}

// Takes away the toasts on `output`, as when the control center opens and lists them itself.
export function hideToasts(output) {
  for (const [id, shown] of [...showing]) if (shown.output === output) hide(id);
}

// Ends the toast of `notification` in `ms` milliseconds: a transient notification ends with it,
// and another stays in the control center.
function timeUp(notification, ms) {
  return after(ms, () => {
    if (notification.transient) data("expire", { id: notification.id });
    else hide(notification.id);
  });
}

// Takes the toast of notification `id` away, at once when another takes its place.
function hide(id, atOnce = false) {
  const shown = showing.get(id);
  if (!shown) return;
  showing.delete(id);
  if (shown.timer !== undefined) cancel(shown.timer);
  const gone = () => {
    shown.toast.remove();
    stacks.get(shown.output)?.onChange?.();
  };
  if (atOnce) {
    gone();
  } else {
    shown.toast.set({ offset: [0, -(ISLAND + 8)] });
    shrink(shown.toast, { content: [shown.content], done: gone });
  }
}

on("notification", (event) => {
  if (event.event === "closed") hide(event.id);
  else show(event);
});
