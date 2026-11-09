// Notifications, as toasts under the right island of the output that has the focus: each comes
// out of the island and goes back up into it.

import {
  box, text, image, on, data, after, cancel, focusedOutput, grow, shrink, ISLAND,
} from "bento:ui";

const WIDTH = 340;
const RADIUS = 18;
// How long a toast that doesn't say stays, in milliseconds.
const STAY = 5000;
// How much of a notification's text a toast shows.
const LENGTH = 180;

// Each output's stack of toasts, and what to tell when it gets or loses one.
const stacks = new Map();
// The toasts showing, by their notification's id.
const showing = new Map();

// The stack of toasts for output `output`, to put under its right island. `onChange` hears a
// toast come or go; `count()` says how many show.
export function toastStack(output, onChange) {
  const stack = box({ direction: "column", align: "end", gap: 8 });
  const entry = { stack, onChange, count: () => count(output) };
  stacks.set(output, entry);
  return entry;
}

function count(output) {
  return [...showing.values()].filter((toast) => toast.output === output).length;
}

// A notification's text without its markup, and no longer than a toast shows.
function plain(markup) {
  const flat = String(markup ?? "")
    .replace(/<[^>]*>/g, "")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&amp;/g, "&")
    .replace(/\s+/g, " ")
    .trim();
  return flat.length > LENGTH ? `${flat.slice(0, LENGTH - 1).trimEnd()}…` : flat;
}

function show(notification) {
  hide(notification.id, true);
  const output = stacks.has(focusedOutput()) ? focusedOutput() : stacks.keys().next().value;
  const entry = stacks.get(output);
  if (!entry) return;

  const picture = notification.image ?? notification.appIcon;
  const body = plain(notification.body);
  const words = box({ direction: "column", gap: 2, shrink: 1, width: WIDTH - 24 - 32 - 12 }, [
    text(plain(notification.summary) || notification.app || "Notification", { weight: 700 }),
    body ? text(body, { size: 13, color: "fg/0.7" }) : null,
  ]);
  const content = box({ gap: 12, align: "start" }, [
    image(picture || "dialog-information-symbolic", { size: 32, color: "fg" }),
    words,
  ]);
  const toast = box(
    {
      material: "glass", radius: RADIUS, width: WIDTH, padding: 12, clip: true, cursor: "pointer",
      // A click does what the notification is for, and sends it away.
      onClick: () => {
        data("invoke", { id: notification.id, key: "default" });
        data("dismiss", { id: notification.id });
      },
      onRightClick: () => data("dismiss", { id: notification.id }),
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
  const timer = stays ? undefined : after(stay, () => data("expire", { id: notification.id }));
  showing.set(notification.id, { toast, content, output, timer });
  entry.onChange?.();
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
