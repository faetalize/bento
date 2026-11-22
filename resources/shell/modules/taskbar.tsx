// A taskbar's apps: a button for each app pinned to it and each app with open windows, lit while
// one of them has the focus, with live previews of its windows over it on hover. A right click
// opens the app's menu, which pins and unpins it, and dragging a button moves it among the others;
// the pinned keep their order among themselves in the settings.

import { action, after, useModule, windows, type App, type Meta, type Window } from "bento";
import { createMemo, createSignal, For, Show } from "solid-js";
import { Flyout } from "bento:modules/ui";
import { menuOpen, openMenu, type MenuItem } from "bento:modules/menu";
import { appById, appIcon, appId, appOf, start } from "bento:modules/apps";

export const meta = {
  settings: {
    pinned: {
      type: "string",
      list: true,
      default: [] as string[],
      description: "The apps pinned to the taskbar, by their desktop entry's id, in order.",
    },
  },
} satisfies Meta;

export const { settings, setSetting } = useModule(import.meta, meta);

/** Whether `app` is pinned. */
export const isPinned = (app: App) => settings.pinned.some((id) => appById(id) === app);

/** Pins `app` at the end, or unpins it. */
export function pin(app: App, pinned: boolean) {
  const list = settings.pinned.filter((id) => appById(id) !== app);
  if (pinned) list.push(appId(app));
  setSetting("pinned", list);
}

/** Pins the apps of `groups` that are pinned, in their order. */
function pinInOrder(groups: AppGroup[]) {
  const list = groups.filter((group) => group.pinned && group.app).map((group) => appId(group.app!));
  if (list.join("\n") !== settings.pinned.join("\n")) setSetting("pinned", list);
}

export interface AppGroup {
  key: string;
  app: App | undefined;
  windows: Window[];
  pinned: boolean;
}

/** The apps pinned, in order, then the windows on `output` by app, in the order their apps first
 * came, each with its windows in the order they opened. */
export function appGroups(output?: string) {
  const groups = new Map<string, AppGroup>();
  for (const id of settings.pinned) {
    const app = appById(id);
    if (app && !groups.has(app.id)) {
      groups.set(app.id, { key: app.id, app, windows: [], pinned: true });
    }
  }
  // By id, which is the order they opened in: bento lists a floating window first once it's raised,
  // and a button's windows don't change places as they're gone through.
  for (const win of [...windows()].sort((a, b) => a.id - b.id)) {
    if (output && win.output !== output) continue;
    const app = appOf(win);
    const key = app?.id ?? win.appId ?? `window-${win.id}`;
    if (!groups.has(key)) groups.set(key, { key, app, windows: [], pinned: false });
    groups.get(key)!.windows.push(win);
  }
  return [...groups.values()];
}

/** Brings up the windows of a group: the next of them when one has the focus, so that clicking
 * goes around them, and minimizes the one with the focus when it's alone, as Windows does. A
 * pinned app without windows starts. */
function activate(group: AppGroup) {
  const list = group.windows;
  if (list.length === 0) {
    if (group.app) start(group.app);
    return;
  }
  const focused = list.findIndex((win) => win.focused);
  if (focused >= 0 && list.length === 1) {
    action("layout-action", ["minimize"]);
    return;
  }
  const next = list[(focused + 1) % list.length];
  action("focus-window", [], { window: next.id });
}

// How long the pointer rests on a button before its previews show, and how long they stay
// after it leaves them, in milliseconds.
const REST = 400;
const LINGER = 300;
const PREVIEW = { width: 220, height: 130 };
// How far a press moves before it drags its button, and the room between buttons.
const SLOP = 8;
const GAP = 4;
// How many marks a button, 40 wide, has room for.
const MARKS = 3;

/** The apps pinned and the apps with windows on `output`, as buttons `size` big. */
export function TaskbarApps(props: { output: string; size?: number }) {
  const groups = createMemo(() => appGroups(props.output));
  // The group whose previews show, and the button they come out of.
  const [shown, setShown] = createSignal<{ group: string; node: any } | null>(null);
  const [open, setOpen] = createSignal(false);
  let timer: (() => void) | null = null;
  const later = (ms: number, fn: () => void) => {
    timer?.();
    timer = after(ms, fn);
  };
  // Previews don't come up over a menu.
  const show = (group: string, node: any) =>
    later(open() ? 0 : REST, () => {
      if (menuOpen()) return;
      setShown({ group, node });
      setOpen(true);
    });
  const hide = () => later(LINGER, () => setOpen(false));
  const previewed = () => groups().find((group) => group.key === shown()?.group);
  const size = () => props.size ?? 40;

  // The buttons' order as they've been dragged, by key. The pinned take the places of the pinned
  // in the setting's order, so that the settings file says it, but while one is dragged.
  const [order, setOrder] = createSignal<string[]>([]);
  const [dragged, setDragged] = createSignal<{ key: string; from: number; x: number; keys: string[] } | null>(null);
  const ordered = createMemo(() => {
    const list = groups();
    const byKey = new Map(list.map((group) => [group.key, group]));
    const kept = order().filter((key) => byKey.has(key));
    const keys = [...kept, ...list.map((group) => group.key).filter((key) => !kept.includes(key))];
    if (dragged()) return keys.map((key) => byKey.get(key)!);
    const pinned = list.filter((group) => group.pinned);
    let next = 0;
    return keys.map((key) => (byKey.get(key)!.pinned ? pinned[next++] : byKey.get(key)!));
  });
  // A press that went on to drag isn't a click.
  let dropped = false;
  // A press that may become a drag: what it pressed, where, and the order then.
  let pressed: { key: string; x: number; keys: string[] } | null = null;
  const press = (key: string, x: number) => {
    setDragged(null);
    pressed = { key, x, keys: ordered().map((group) => group.key) };
  };
  const drag = (x: number) => {
    if (!pressed) return;
    // Solid shows what's set once the event is done, so the drag starts from what's at hand.
    let drag = dragged();
    if (!drag) {
      if (Math.abs(x - pressed.x) < SLOP) return;
      timer?.();
      setOpen(false);
      drag = { ...pressed, from: pressed.keys.indexOf(pressed.key) };
      setDragged(drag);
    }
    const step = size() + GAP;
    const to = Math.max(0, Math.min(drag.keys.length - 1, drag.from + Math.round((x - drag.x) / step)));
    const keys = drag.keys.filter((key) => key !== drag.key);
    keys.splice(to, 0, drag.key);
    setOrder(keys);
  };
  const release = () => {
    pressed = null;
    if (!dragged()) return;
    setDragged(null);
    dropped = true;
    after(0, () => (dropped = false));
    pinInOrder(ordered());
  };
  /** Pins or unpins `group` where its button is, among the others pinned. */
  const pinHere = (group: AppGroup, pinned: boolean) =>
    pinInOrder(ordered().map((one) => (one.key === group.key ? { ...one, pinned } : one)));
  // How far the dragged button is from its place, to follow the pointer.
  const [pointerX, setPointerX] = createSignal(0);

  return (
    <box style={{ flexDirection: "row", alignItems: "center", gap: GAP }}>
      <For each={ordered()} keyed={(group: any) => group.key}>
        {(item: () => AppGroup) => {
          // Kept by its app as its windows come and go, so the button stays.
          const group: AppGroup = {
            get key() { return item().key; },
            get app() { return item().app; },
            get windows() { return item().windows; },
            get pinned() { return item().pinned; },
          };
          const moving = () => dragged()?.key === group.key;
          // Where it'd be without the drag, and where the pointer has it.
          const offset = () => {
            const drag = dragged();
            if (!drag || !moving()) return 0;
            const at = ordered().findIndex((one) => one.key === group.key);
            return pointerX() - drag.x - (at - drag.from) * (size() + GAP);
          };
          let node: any;
          const lit = () => group.windows.some((win) => win.focused);
          const running = () => group.windows.length > 0;
          return (
            <box
              ref={node}
              style={{
                width: size(), height: size(), borderRadius: 8, alignItems: "center",
                justifyContent: "center", flexDirection: "column",
                background: lit() ? "fg/0.1" : undefined, ":hover": { background: "fg/0.14" },
                translate: [offset(), 0], opacity: moving() ? 0.85 : 1,
                // The dragged button keeps up with the pointer.
                transition: moving() ? { frame: 0.04, translate: 0.04 } : undefined,
              }}
              enter={{ scale: 0.6, opacity: 0 }}
              exit={{ scale: 0.6, opacity: 0 }}
              onPress={(event) => event.button === 1 && press(group.key, event.x)}
              onDrag={(event) => {
                setPointerX(event.x);
                drag(event.x);
              }}
              onRelease={release}
              onClick={() => !dropped && activate(group)}
              onMiddleClick={() => group.app && start(group.app)}
              onRightClick={() => {
                timer?.();
                setOpen(false);
                openMenu(
                  { output: props.output, node, side: "top" },
                  appMenu(group, (pinned) => pinHere(group, pinned)),
                );
              }}
              onNear={({ inside }) =>
                inside && running() && !dragged() ? show(group.key, node) : hide()}
            >
              <image icon={appIcon(group.app)} size={Math.round(size() * 0.6)} />
              <Marks windows={group.windows} width={size()} />
            </box>
          );
        }}
      </For>
      <Show when={shown()}>
        <Flyout
          output={props.output}
          open={open()}
          from={shown()!.node}
          side="top"
          padding={8}
          light={false}
        >
          <box
            style={{ flexDirection: "row", gap: 8 }}
            onNear={({ inside }) => (inside ? timer?.() : hide())}
          >
            <For each={previewed()?.windows ?? []}>
              {(win) => (
                <box
                  style={{
                    flexDirection: "column", gap: 6, padding: 6, borderRadius: 12,
                    width: PREVIEW.width + 12, ":hover": { background: "fg/0.1" },
                  }}
                  onClick={() => {
                    action("focus-window", [], { window: win.id });
                    setOpen(false);
                  }}
                >
                  <box style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
                    <image icon={appIcon(appOf(win))} size={16} />
                    <text style={{ fontSize: 12, lineClamp: 1, flexShrink: 1, minWidth: 0 }}>
                      {win.title ?? ""}
                    </text>
                  </box>
                  <copy window={win.id} style={{ width: PREVIEW.width, height: PREVIEW.height, borderRadius: 8 }} />
                </box>
              )}
            </For>
          </box>
        </Flyout>
      </Show>
    </box>
  );
}

/** The marks under an app's button, `width` wide: one for each of its windows, long and lit for the
 * one with the focus. The last stands for every window there's no room for. */
function Marks(props: { windows: Window[]; width: number }) {
  const marks = createMemo(() => {
    const shown = props.windows.slice(0, MARKS).map((win) => [win]);
    shown[MARKS - 1]?.push(...props.windows.slice(MARKS));
    return shown.map((list) => ({
      focused: list.some((win) => win.focused),
      urgent: list.some((win) => win.urgent),
    }));
  });
  return (
    <box
      style={{
        position: "absolute", bottom: 2, left: 0, width: props.width, flexDirection: "row",
        justifyContent: "center", gap: 3,
      }}
    >
      {/* By place, so that marks stay and change length as the focus moves. */}
      <For each={marks()} keyed={false}>
        {(mark) => (
          <box
            style={{
              height: 3, borderRadius: 2, width: mark().focused ? 16 : 6,
              background: mark().urgent ? "#e5a50a" : mark().focused ? "accent" : "fg/0.5",
            }}
            enter={{ opacity: 0 }}
            exit={{ opacity: 0 }}
          />
        )}
      </For>
    </box>
  );
}

/** What a right click on an app's button offers: a new window, pinning it, and closing its
 * windows. */
function appMenu(group: AppGroup, pin: (pinned: boolean) => void): MenuItem[] {
  const app = group.app;
  const windows = group.windows;
  const items: MenuItem[] = [];
  if (app) {
    items.push({ label: app.name, icon: appIcon(app), run: () => start(app) });
    items.push(
      group.pinned
        ? { label: "Unpin from Taskbar", icon: "view-pin-symbolic", run: () => pin(false) }
        : { label: "Pin to Taskbar", icon: "view-pin-symbolic", run: () => pin(true) },
    );
  }
  if (windows.length > 0) {
    if (items.length > 0) items.push("separator");
    items.push({
      label: windows.length === 1 ? "Close Window" : "Close All Windows",
      icon: "window-close-symbolic",
      run: () => {
        for (const win of windows) action("close-window", [], { window: win.id });
      },
    });
  }
  return items;
}
