// Apps: finding them by what's typed, a row to show one, and the app a window is of. The
// launcher and a start menu both search with it.

import { apps, launch, preload, type App, type Window } from "bento";
import { createMemo, createSignal, Show } from "solid-js";

/** How well an app matches a query, lower being better, or null if it doesn't. */
export function score(app: App, query: string) {
  if (!query) return 0;
  const name = app.name.toLowerCase();
  if (name.startsWith(query)) return 0;
  if (name.split(/\s+/).some((word) => word.startsWith(query))) return 1;
  if (name.includes(query)) return 2;
  const extra = [app.generic, app.comment, ...app.keywords].filter(Boolean) as string[];
  if (extra.some((word) => word.toLowerCase().includes(query))) return 3;
  return null;
}

/** A search through the apps: what's typed, the best `most` that match it, which is selected,
 * and starting one. `refresh()` reads the apps again, as when a search opens. */
export function appSearch(most = 8) {
  let all = apps();
  const [query, setQuery] = createSignal("");
  const [selected, setSelected] = createSignal(0);
  const matches = createMemo(() => {
    const q = query().trim().toLowerCase();
    if (!q) return [];
    return all
      .map((app) => [score(app, q), app] as const)
      .filter(([s]) => s !== null)
      .sort((a, b) => (a[0] as number) - (b[0] as number))
      .slice(0, most)
      .map(([, app]) => app);
  });
  const select = (index: number) => {
    const count = matches().length;
    if (count > 0) setSelected((index + count) % count);
  };
  return {
    query,
    matches,
    selected,
    select,
    type(text: string) {
      setQuery(text);
      setSelected(0);
    },
    refresh() {
      all = apps();
      setQuery("");
      setSelected(0);
    },
    /** The selected app, if any matches. */
    current: () => matches()[selected()],
    /** Up, Down and Tab move the selection: true when the key was one of them. */
    key(event: { key: string; shift: boolean; pressed: boolean }) {
      if (!event.pressed) return false;
      if (event.key === "Down" || (event.key === "Tab" && !event.shift)) select(selected() + 1);
      else if (event.key === "Up" || event.key === "ISO_Left_Tab") select(selected() - 1);
      else return false;
      return true;
    },
  };
}

export type AppSearch = ReturnType<typeof appSearch>;

/** Loads the apps' icons at `size` ahead of the first search, which then has them in its first
 * frame. */
export function preloadIcons(size: number) {
  for (const app of apps()) preload(app.icon ?? "application-x-executable", size);
}

export const appIcon = (app?: App | null) => app?.icon ?? "application-x-executable";

/** An app as a row: its icon, its name and what it's for. */
export function AppRow(props: {
  app: App;
  selected?: boolean;
  iconSize?: number;
  onClick: () => void;
  onHover?: () => void;
}) {
  return (
    <box
      style={{
        flexDirection: "row", alignItems: "center", gap: 14, padding: [8, 12], borderRadius: 16,
        background: props.selected ? "fg/0.1" : undefined,
      }}
      onClick={props.onClick}
      onHover={({ inside }) => inside && props.onHover?.()}
    >
      <image icon={appIcon(props.app)} size={props.iconSize ?? 40} />
      <box style={{ flexDirection: "column", flexGrow: 1, flexShrink: 1, minWidth: 0 }}>
        <text style={{ fontSize: 16, fontWeight: 600, lineClamp: 1 }}>{props.app.name}</text>
        <Show when={props.app.comment}>
          <text style={{ fontSize: 13, color: "fg/0.6", lineClamp: 1 }}>{props.app.comment}</text>
        </Show>
      </box>
    </box>
  );
}

export const start = (app: App) => launch(app.id);

/** An app's id without `.desktop`, like `org.gnome.Nautilus`, as settings name apps. */
export const appId = (app: App) => app.id.replace(/\.desktop$/, "");

const bare = (id: string) => id.replace(/\.desktop$/, "").toLowerCase();

/** The app with id `id`, written with `.desktop` or without. */
export function appById(id: string): App | undefined {
  return known().find((app) => bare(app.id) === bare(id));
}

// The apps, read again at most every few seconds, for what looks one up often.
let cached: App[] = [];
let readAt = -Infinity;
function known() {
  if (Date.now() - readAt > 5000) {
    cached = apps();
    readAt = Date.now();
  }
  return cached;
}

/** The app a window is of, by its app id: a desktop entry's id, the last part of one, or its
 * `StartupWMClass`. */
export function appOf(window: Window): App | undefined {
  const id = window.appId?.toLowerCase();
  if (!id) return undefined;
  const all = known();
  return all.find((app) => bare(app.id) === id)
    ?? all.find((app) => app.wmClass?.toLowerCase() === id)
    ?? all.find((app) => bare(app.id).split(".").pop() === id);
}
