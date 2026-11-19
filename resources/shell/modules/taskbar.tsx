// A taskbar's apps: a button for each app with open windows, lit while one of them has the
// focus, with live previews of its windows over it on hover.

import { action, after, windows, type Window } from "bento";
import { createMemo, createSignal, For, Show } from "solid-js";
import { Flyout } from "bento:modules/ui";
import { appIcon, appOf, start } from "bento:modules/apps";

/** The windows on `output`, by app, in the order their apps first came. */
export function appGroups(output?: string) {
  const groups = new Map<string, { key: string; app: any; windows: Window[] }>();
  for (const win of windows()) {
    if (output && win.output !== output) continue;
    const app = appOf(win);
    const key = app?.id ?? win.appId ?? `window-${win.id}`;
    if (!groups.has(key)) groups.set(key, { key, app, windows: [] });
    groups.get(key)!.windows.push(win);
  }
  return [...groups.values()];
}

/** Brings up the windows of a group: the next of them when one has the focus, so that clicking
 * goes around them, and minimizes the one with the focus when it's alone, as Windows does. */
function activate(list: Window[]) {
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

/** The apps with windows on `output`, as buttons `size` big. */
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
  const show = (group: string, node: any) =>
    later(open() ? 0 : REST, () => {
      setShown({ group, node });
      setOpen(true);
    });
  const hide = () => later(LINGER, () => setOpen(false));
  const previewed = () => groups().find((group) => group.key === shown()?.group);
  const size = () => props.size ?? 40;
  return (
    <box style={{ flexDirection: "row", alignItems: "center", gap: 4 }}>
      <For each={groups()} keyed={(group: any) => group.key}>
        {(item: any) => {
          // Kept by its app as its windows come and go, so the button stays.
          const group = { get key() { return item().key; }, get app() { return item().app; }, get windows() { return item().windows; } };
          let node: any;
          const lit = () => group.windows.some((win) => win.focused);
          const urgent = () => group.windows.some((win) => win.urgent);
          return (
            <box
              ref={node}
              style={{
                width: size(), height: size(), borderRadius: 8, alignItems: "center",
                justifyContent: "center", flexDirection: "column",
                background: lit() ? "fg/0.1" : undefined, ":hover": { background: "fg/0.14" },
              }}
              enter={{ scale: 0.6, opacity: 0 }}
              exit={{ scale: 0.6, opacity: 0 }}
              onClick={() => activate(group.windows)}
              onMiddleClick={() => group.app && start(group.app)}
              onNear={({ inside }) => (inside ? show(group.key, node) : hide())}
            >
              <image icon={appIcon(group.app)} size={Math.round(size() * 0.6)} />
              {/* Under it, a line: wide and lit while it has the focus, short while it's open. */}
              <box
                style={{
                  position: "absolute", bottom: 2, height: 3, borderRadius: 2,
                  width: lit() ? 16 : 6, left: (size() - (lit() ? 16 : 6)) / 2,
                  background: urgent() ? "#e5a50a" : lit() ? "accent" : "fg/0.5",
                }}
              />
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
