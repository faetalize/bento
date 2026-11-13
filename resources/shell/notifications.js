// The notifications that came, newest first, kept until they're dismissed or their app closes
// them, and Do Not Disturb. The toasts show them as they come; the control center lists them.
// A transient one, like a change of sound output, is only a toast.

import { on, data, stored, store } from "bento:ui";

const all = [];
const listeners = [];
let quiet = Boolean(stored("doNotDisturb", false));
// Whether one came since the list was last looked at.
let unseen = false;

const changed = () => listeners.forEach((listener) => listener());

// Calls `listener` whenever the list, Do Not Disturb, or whether there's something unseen
// changes.
export function onNotifications(listener) {
  listeners.push(listener);
}

// The notifications kept, newest first: `{ id, app, appIcon, summary, body, actions, urgency,
// image, at }`, `at` being when it came, in milliseconds.
export function kept() {
  return all.filter((notification) => !notification.transient);
}

export function find(id) {
  return all.find((notification) => notification.id === id);
}

export function doNotDisturb() {
  return quiet;
}

export function setDoNotDisturb(on) {
  quiet = Boolean(on);
  store("doNotDisturb", quiet);
  changed();
}

export function unseenOnes() {
  return unseen;
}

// Says the list has been looked at.
export function looked() {
  if (!unseen) return;
  unseen = false;
  changed();
}

export function dismiss(id) {
  data("dismiss", { id });
}

export function dismissAll() {
  for (const notification of kept()) data("dismiss", { id: notification.id });
}

// Does a notification's action `key`. It goes with an activation token, so the app may bring
// its window forward.
export function invoke(id, key) {
  data("invoke", { id, key, token: true });
}

export function hasDefault(notification) {
  return (notification.actions ?? []).some(([key]) => key === "default");
}

// A notification's text without its markup, at most `length` long.
export function plain(markup, length = 180) {
  const flat = String(markup ?? "")
    .replace(/<[^>]*>/g, "")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&amp;/g, "&")
    .replace(/\s+/g, " ")
    .trim();
  return flat.length > length ? `${flat.slice(0, length - 1).trimEnd()}…` : flat;
}

// How long ago `at` was, shortly.
export function ago(at, now = Date.now()) {
  const minutes = Math.floor((now - at) / 60000);
  if (minutes < 1) return "Now";
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} h`;
  return `${Math.floor(hours / 24)} d`;
}

on("notification", (event) => {
  const at = all.findIndex((notification) => notification.id === event.id);
  if (event.event === "closed") {
    if (at >= 0) all.splice(at, 1);
    changed();
    return;
  }
  // One that replaces another takes its place at the top, as it's news again.
  if (at >= 0) all.splice(at, 1);
  all.unshift({ ...event, at: Date.now() });
  if (!event.transient) unseen = true;
  changed();
});
