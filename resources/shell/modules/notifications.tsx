// Notifications: the ones that came, newest first, kept until they're dismissed or their app
// closes them, and Do Not Disturb; cards to list them, and toasts that show them as they come.
// A transient one, like a change of sound output, is only a toast.

import { Surface, after, command, now, onData, stored } from "bento";
import { createEffect, createSignal, For, onCleanup, Show } from "solid-js";
import { Button, ISLAND, grows, growsWith, plain } from "bento:modules/ui";

export interface Notification {
  id: number;
  app: string;
  appIcon: string | null;
  summary: string;
  body: string;
  actions: [string, string][];
  urgency: 0 | 1 | 2;
  timeout: number;
  image: string | null;
  transient: boolean;
  /** When it came, in milliseconds. */
  at: number;
}

const [all, setAll] = createSignal<Notification[]>([]);
const [quiet, setQuiet] = stored("doNotDisturb", false);
// Whether one came since the list was last looked at.
const [unseen, setUnseen] = createSignal(false);
// What hears each notification come, as it comes, for toasts.
const comers = new Set<(notification: Notification) => void>();
const goers = new Set<(id: number) => void>();

onData("notification", (event: any) => {
  if (event.event === "closed") {
    setAll((list) => list.filter((one) => one.id !== event.id));
    for (const fn of goers) fn(event.id);
    return;
  }
  // One that replaces another takes its place at the top, as it's news again.
  const { event: _event, ...rest } = event;
  const notification = { ...rest, at: Date.now() } as Notification;
  setAll((list) => [notification, ...list.filter((one) => one.id !== event.id)]);
  if (!event.transient) setUnseen(true);
  for (const fn of comers) fn(notification);
});

export const notifications = {
  /** The notifications kept, newest first. */
  kept: () => all().filter((notification) => !notification.transient),
  doNotDisturb: quiet,
  setDoNotDisturb: (on: boolean) => setQuiet(Boolean(on)),
  unseen,
  /** Says the list has been looked at. */
  looked: () => setUnseen(false),
  dismiss: (id: number) => command("dismiss", { id }),
  dismissAll: () => {
    for (const notification of notifications.kept()) command("dismiss", { id: notification.id });
  },
  /** Does a notification's action `key`, with an activation token, so that the app may bring
   * its window forward. */
  invoke: (id: number, key: string) => command("invoke", { id, key, token: true }),
  hasDefault: (notification: Notification) =>
    (notification.actions ?? []).some(([key]) => key === "default"),
};

/** How long ago `at` was, shortly. */
export function ago(at: number, time = Date.now()) {
  const minutes = Math.floor((time - at) / 60000);
  if (minutes < 1) return "Now";
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} h`;
  return `${Math.floor(hours / 24)} d`;
}

const picture = (n: Notification) => n.image ?? (n.appIcon || "dialog-information-symbolic");
const buttons = (n: Notification) => (n.actions ?? []).filter(([key]) => key !== "default");

/** A notification in a list: its app, when it came, its words and buttons, and a way to send it
 * away. A click does what it's for. */
export function NotificationCard(props: { notification: Notification; onUsed?: () => void }) {
  const n = () => props.notification;
  const minute = now(60_000);
  return (
    <box
      style={{
        flexDirection: "row", gap: 10, alignItems: "start", padding: [10, 6, 10, 10],
        borderRadius: 16, background: "fg/0.08",
        cursor: notifications.hasDefault(n()) ? "pointer" : undefined,
      }}
      enter={{ opacity: 0 }}
      exit={{ opacity: 0 }}
      onClick={() => {
        if (!notifications.hasDefault(n())) return;
        notifications.invoke(n().id, "default");
        props.onUsed?.();
      }}
    >
      <image icon={picture(n())} size={28} />
      <box style={{ flexDirection: "column", gap: 2, flexGrow: 1, flexShrink: 1, minWidth: 0 }}>
        <box style={{ flexDirection: "row", gap: 6 }}>
          <box style={{ flexGrow: 1, flexShrink: 1, minWidth: 0 }}>
            <text style={{ fontSize: 12, color: "fg/0.6", lineClamp: 1 }}>
              {n().app || "Notification"}
            </text>
          </box>
          <text style={{ fontSize: 12, color: "fg/0.6", fontVariantNumeric: "tabular-nums" }}>
            {ago(n().at, minute().getTime())}
          </text>
        </box>
        <text style={{ fontWeight: 700, lineClamp: 1 }}>
          {plain(n().summary, 80) || n().app || "Notification"}
        </text>
        <Show when={plain(n().body, 140)}>
          <text style={{ fontSize: 13, color: "fg/0.75" }}>{plain(n().body, 140)}</text>
        </Show>
        <Show when={buttons(n()).length > 0}>
          <box style={{ flexDirection: "row", gap: 6, paddingTop: 6 }}>
            <For each={buttons(n())}>
              {([key, label]) => (
                <Button
                  label={label}
                  style={{ background: "fg/0.1", flexGrow: 1, flexShrink: 1, minWidth: 0 }}
                  onClick={() => notifications.invoke(n().id, key)}
                />
              )}
            </For>
          </box>
        </Show>
      </box>
      <Button
        icon="window-close-symbolic"
        size={14}
        style={{ width: 26, height: 26, borderRadius: 13 }}
        onClick={() => notifications.dismiss(n().id)}
      />
    </box>
  );
}

// ------------------------------------------------------------------------------------------------
// Toasts.
// ------------------------------------------------------------------------------------------------

const TOAST_WIDTH = 340;
const TOAST_RADIUS = 18;
// How long a toast that doesn't say stays, in milliseconds.
const STAY = 5000;

export interface ToastsProps {
  /** The output they show on. */
  output: string;
  /** What they come out of, like an island: they hang under it, and come down out of it. */
  from?: any;
  /** While true, none come, and those showing go, as while a list shows them itself. */
  held?: boolean;
  /** Hears how many show, as it changes. */
  onCount?: (count: number) => void;
}

/** Notifications as they come, as toasts under what they come out of. A click does what the
 * notification is for, its buttons do its other actions, and the pointer on it keeps it. Once
 * its time is up the toast goes, and the notification stays listed, unless it's transient. With
 * Do Not Disturb on, only critical ones show. */
export function Toasts(props: ToastsProps) {
  const [showing, setShowing] = createSignal<Notification[]>([]);
  const timers = new Map<number, () => void>();
  const report = (list: Notification[]) => props.onCount?.(list.length);

  const hide = (id: number) => {
    timers.get(id)?.();
    timers.delete(id);
    setShowing((list) => {
      const next = list.filter((one) => one.id !== id);
      report(next);
      return next;
    });
  };
  const timeUp = (n: Notification, ms: number) =>
    after(ms, () => {
      if (n.transient) command("expire", { id: n.id });
      else hide(n.id);
    });
  const stay = (n: Notification) => (n.timeout > 0 ? n.timeout : STAY);
  // One that's critical stays until it's dealt with, and so does one that says to.
  const stays = (n: Notification) => n.urgency === 2 || n.timeout === 0;

  const come = (n: Notification) => {
    if (props.held) return;
    if (notifications.doNotDisturb() && n.urgency !== 2) return;
    timers.get(n.id)?.();
    timers.delete(n.id);
    if (!stays(n)) timers.set(n.id, timeUp(n, stay(n)));
    setShowing((list) => {
      const next = [n, ...list.filter((one) => one.id !== n.id)];
      report(next);
      return next;
    });
  };
  comers.add(come);
  goers.add(hide);
  onCleanup(() => {
    comers.delete(come);
    goers.delete(hide);
  });

  // Held, the toasts showing go.
  createEffect(
    () => Boolean(props.held),
    (held) => {
      if (held) for (const n of showing()) hide(n.id);
    },
  );

  return (
    <Surface
      output={props.output}
      layer="overlay"
      z={-1}
      attach={props.from ? { to: props.from, side: "bottom", align: "end", gap: 8 } : undefined}
      anchor={props.from ? undefined : ["top", "right"]}
      style={{ flexDirection: "column", alignItems: "end", gap: 8 }}
    >
      <Show when={!props.held}>
        <For each={showing()}>
          {(n) => (
            <box
              style={{
                material: "glass", borderRadius: TOAST_RADIUS, width: TOAST_WIDTH, padding: 12,
                overflow: "hidden", cursor: "pointer", flexDirection: "column",
              }}
              // It comes down out of what it hangs from, whose glass it melts with on the way.
              enter={{ ...grows(TOAST_RADIUS).enter, translate: [0, -(ISLAND + 8)] }}
              exit={{ ...grows(TOAST_RADIUS).exit, translate: [0, -(ISLAND + 8)] }}
              // A click does what the notification is for, or sends it away if it's for nothing.
              onClick={() => {
                if (notifications.hasDefault(n)) notifications.invoke(n.id, "default");
                else notifications.dismiss(n.id);
              }}
              onRightClick={() => notifications.dismiss(n.id)}
              // Under the pointer it stays; once the pointer leaves, its time starts again.
              onNear={({ inside }) => {
                timers.get(n.id)?.();
                timers.delete(n.id);
                if (!inside && !stays(n)) timers.set(n.id, timeUp(n, stay(n)));
              }}
            >
              <box style={{ flexDirection: "column" }} {...growsWith}>
                <box style={{ flexDirection: "row", gap: 12, alignItems: "start" }}>
                  <image icon={picture(n)} size={32} />
                  <box
                    style={{
                      flexDirection: "column", gap: 2, flexShrink: 1, width: TOAST_WIDTH - 24 - 32 - 12,
                    }}
                  >
                    <text style={{ fontWeight: 700, lineClamp: 1 }}>
                      {plain(n.summary) || n.app || "Notification"}
                    </text>
                    <Show when={plain(n.body)}>
                      <text style={{ fontSize: 13, color: "fg/0.7" }}>{plain(n.body)}</text>
                    </Show>
                  </box>
                </box>
                <Show when={buttons(n).length > 0}>
                  <box style={{ flexDirection: "row", gap: 6, justifyContent: "end", paddingTop: 10 }}>
                    <For each={buttons(n)}>
                      {([key, label]) => (
                        <Button
                          label={label}
                          style={{ background: "fg/0.1", flexGrow: 1, flexShrink: 1, minWidth: 0 }}
                          onClick={() => notifications.invoke(n.id, key)}
                        />
                      )}
                    </For>
                  </box>
                </Show>
              </box>
            </box>
          )}
        </For>
      </Show>
    </Surface>
  );
}
