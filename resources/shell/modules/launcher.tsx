// The launcher: a glass search field in the middle of the focused output that finds apps as
// they're typed, its results opening below it. It grows out of a point, and shrinks back into
// one. Its shortcut opens it, and Escape or a press elsewhere closes it.

import { Surface, apps, focusedOutput, launch, onAction, outputs, preload } from "bento";
import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import { grows, growsWith, popup } from "bento:modules/ui";

// How big an app's icon is in a row.
const ICON = 40;
const RESULTS = 8;
// How tall the launcher is with all its results: it's held at the top of that room, in the
// middle of the output, so its search stays put as the results come and go.
const HEIGHT = 544;
const RADIUS = 26;

/** How well an app matches a query, lower being better, or null if it doesn't. */
function score(app: any, query: string) {
  if (!query) return 0;
  const name = app.name.toLowerCase();
  if (name.startsWith(query)) return 0;
  if (name.split(/\s+/).some((word: string) => word.startsWith(query))) return 1;
  if (name.includes(query)) return 2;
  const extra = [app.generic, app.comment, ...app.keywords].filter(Boolean);
  if (extra.some((word: string) => word.toLowerCase().includes(query))) return 3;
  return null;
}

export function Launcher() {
  // The apps are read and their icons loaded ahead of the first search, which then has them in
  // its first frame.
  for (const app of apps()) preload(app.icon ?? "application-x-executable", ICON);
  const [where, setWhere] = createSignal<string | null>(null);
  onAction("toggle-launcher", ({ output }) => {
    if (popup.is("launcher")) {
      popup.close("launcher");
      return;
    }
    setWhere(output ?? focusedOutput() ?? outputs()[0] ?? null);
    popup.open("launcher");
  });
  // It's kept while closed, so that opening it again as it shrinks away turns it around from
  // where it is.
  return (
    <Show when={where()}>
      <Palette output={where()!} open={popup.is("launcher")} />
    </Show>
  );
}

function Palette(props: { output: string; open: boolean }) {
  let all = apps();
  let field: any;
  const [query, setQuery] = createSignal("");
  // Opened, it starts empty, with the apps as they are now, and the keys.
  createEffect(
    () => props.open,
    (open) => {
      if (!open) return;
      all = apps();
      setQuery("");
      setSelected(0);
      field?.setText("");
      field?.focus();
    },
  );
  const [selected, setSelected] = createSignal(0);
  // Only the field, until there's something to find.
  const matches = createMemo(() => {
    const q = query().trim().toLowerCase();
    if (!q) return [];
    return all
      .map((app: any) => [score(app, q), app] as const)
      .filter(([s]) => s !== null)
      .sort((a, b) => (a[0] as number) - (b[0] as number))
      .slice(0, RESULTS)
      .map(([, app]) => app);
  });
  const select = (index: number) => {
    const count = matches().length;
    if (count > 0) setSelected((index + count) % count);
  };
  const close = () => popup.close("launcher");
  const start = (app: any) => {
    launch(app.id);
    close();
  };
  return (
    <Surface
      output={props.output}
      layer="overlay"
      z={-1}
      keyboard={props.open ? "exclusive" : "none"}
      onEscape={close}
      onOutside={() => props.open && close()}
      style={{ height: HEIGHT, flexDirection: "column", alignItems: "center" }}
    >
      <box
        style={{
          material: "glass", borderRadius: RADIUS, width: 720, padding: 12,
          flexDirection: "column", overflow: "hidden", display: props.open ? "flex" : "none",
        }}
        {...grows(RADIUS)}
      >
        {/* One body, so that it grows around the launcher's middle as one. */}
        <box style={{ flexDirection: "column", gap: matches().length ? 8 : 0 }} {...growsWith}>
          <box style={{ padding: [10, 14], fontSize: 22 }}>
            <input
              ref={field}
              placeholder="Search apps"
              onInput={(typed) => {
                setQuery(typed);
                setSelected(0);
              }}
              onSubmit={() => {
                const app = matches()[selected()];
                if (app) start(app);
              }}
              onKey={(event) => {
                if (!event.pressed) return false;
                if (event.key === "Down" || (event.key === "Tab" && !event.shift)) select(selected() + 1);
                else if (event.key === "Up" || event.key === "ISO_Left_Tab") select(selected() - 1);
                else return false;
                return true;
              }}
            />
          </box>
          <box style={{ flexDirection: "column", gap: 2 }}>
            <For each={matches()}>
              {(app: any, index) => (
                <box
                  style={{
                    flexDirection: "row", alignItems: "center", gap: 14, padding: [8, 12],
                    borderRadius: 16, background: index() === selected() ? "fg/0.1" : undefined,
                  }}
                  onClick={() => start(app)}
                  onHover={({ inside }) => inside && select(index())}
                >
                  <image icon={app.icon ?? "application-x-executable"} size={ICON} />
                  <box style={{ flexDirection: "column", flexGrow: 1, flexShrink: 1, minWidth: 0 }}>
                    <text style={{ fontSize: 16, fontWeight: 600, lineClamp: 1 }}>{app.name}</text>
                    <Show when={app.comment}>
                      <text style={{ fontSize: 13, color: "fg/0.6", lineClamp: 1 }}>{app.comment}</text>
                    </Show>
                  </box>
                </box>
              )}
            </For>
          </box>
        </box>
      </box>
    </Surface>
  );
}
