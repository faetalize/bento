// A start menu: the apps pinned to it, some in groups, and the files opened lately, or all the
// apps from A to Z; apps and files found by typing, as the launcher finds them; and along its
// bottom, the user, folders, bento's settings and the power menu. It comes out of the start
// button, or up from the bottom of the output, in its middle.

import {
  action, after, apps, command, data, focusedOutput, now, onAction, outputSize, outputs, useModule,
  type App, type FoundFile, type Meta, type Pin, type PointerEvent, type RecentFile, type SceneNode,
} from "bento";
import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import { Button, Flyout, Scroller, popup } from "bento:modules/ui";
import { appById, appIcon, appId, preloadIcons, start } from "bento:modules/apps";
import { FilesPill, SearchLine, everythingSearch, type Line } from "bento:modules/search";
import { PowerPage } from "bento:modules/power";
import { openFile, showFile } from "bento:modules/files";
import { openMenu, type MenuItem } from "bento:modules/menu";
import { isPinned as onTaskbar, pin as pinToTaskbar } from "bento:modules/taskbar";

export const meta = {
  settings: {
    pinned: {
      type: "pins",
      default: [] as Pin[],
      description: "The apps pinned to the start menu, in order, some of them in groups.",
    },
    recent: {
      type: "boolean",
      default: true,
      description: "Show the files opened lately under the pinned apps.",
      page: "shell",
      group: "Start Menu",
    },
    folders: {
      type: "folder",
      list: true,
      default: [
        "$XDG_DOCUMENTS_DIR", "$XDG_DOWNLOAD_DIR", "$XDG_PICTURES_DIR", "$XDG_MUSIC_DIR",
        "$XDG_VIDEOS_DIR",
      ],
      description: "Folders with a button along the bottom of the start menu. One that isn't "
        + "there has none.",
      page: "shell",
      group: "Start Menu",
    },
  },
  actions: {
    "toggle-start-menu": {
      description: "Show or hide the start menu on the current monitor.",
      bind: "Mod",
    },
  },
} satisfies Meta;

const { settings, setSetting } = useModule(import.meta, meta);

onAction("toggle-start-menu", ({ output }) => {
  popup.toggle(`start:${output ?? focusedOutput() ?? outputs()[0]}`);
});

const WIDTH = 640;
const PAD = 16;
const COLUMNS = 6;
const TILE = { width: WIDTH / COLUMNS, height: 88 };
const ICON = 32;
const HEIGHTS = { head: 28, row: 48, more: 40 };
// A section's title, with the room under it, and the room between sections.
const HEAD = 28;
const GAP = 14;
// The files opened lately the main page shows, in two columns.
const RECENT = 6;
// How tall what's between the field and the bottom bar is, at most: three rows of pinned apps and
// the files opened lately. On an output too short for the menu, it's what's left of the room
// above where the menu comes out, but for `EDGE` at the top, and never less than a row.
const LIST = HEAD + 3 * TILE.height + GAP + HEAD + (RECENT / 2) * HEIGHTS.row;
const LEAST = HEAD + TILE.height;
const EDGE = 24;
// The bottom bar: its line, its padding and its buttons.
const BAR = 1 + 10 + 36 + 10;
// How much of the menu isn't what's between the field and the bottom bar.
const AROUND = PAD + 42 + GAP + GAP + BAR;
// The all apps list: a letter's title and an app's row.
const LETTER = 34;
const ROW = 44;
// How far a press moves before it drags a pinned app.
const SLOP = 8;
// A group made by dropping an app on another is called this until it's renamed.
const NEW_GROUP = "New Group";

/** A pinned app, or a group of them, by its place among the pins. */
type Item =
  | { key: string; app: App }
  | { key: string; group: string; apps: App[] };

const isGroup = (item: Item): item is Extract<Item, { group: string }> => "group" in item;

/** The pins as they show: apps that aren't installed are left out, and groups with none. */
function pinnedItems(pins: readonly Pin[]): Item[] {
  const items: Item[] = [];
  pins.forEach((pin, at) => {
    if (typeof pin === "string") {
      const app = appById(pin);
      if (app) items.push({ key: `app:${app.id}`, app });
    } else {
      const list = pin.apps.map(appById).filter((app): app is App => app !== undefined);
      if (list.length > 0) items.push({ key: `group:${at}:${pin.group}`, group: pin.group, apps: list });
    }
  });
  return items;
}

/** Writes `items` as the pins, a group of one app as that app. */
function savePins(items: Item[]) {
  const pins: Pin[] = items.map((item) => {
    if (!isGroup(item)) return appId(item.app);
    if (item.apps.length === 1) return appId(item.apps[0]);
    return { group: item.group, apps: item.apps.map(appId) };
  });
  setSetting("pinned", pins);
}

const pinned = () => pinnedItems(settings.pinned);

/** Whether `app` is pinned to the start menu, on its own or in a group. */
export const isPinned = (app: App) =>
  pinned().some((item) => (isGroup(item) ? item.apps.includes(app) : item.app === app));

/** Pins `app` to the start menu at the end, or unpins it, from its group too. */
export function pin(app: App, pinnedNow: boolean) {
  const items = pinned()
    .map((item) =>
      isGroup(item) ? { ...item, apps: item.apps.filter((one) => one !== app) } : item)
    .filter((item) => (isGroup(item) ? item.apps.length > 0 : item.app !== app));
  if (pinnedNow) items.push({ key: `app:${app.id}`, app });
  savePins(items);
}

/** The start menu of `output`, which `popup` opens as `start:<output>`: `gap` over the start
 * button `from`, lined up with it as `align` says, or else `bottom` up from the bottom of the
 * output. */
export function StartMenu(props: {
  output: string;
  from?: SceneNode;
  align?: "start" | "center" | "end";
  gap?: number;
  bottom?: number;
}) {
  preloadIcons(ICON);
  const key = `start:${props.output}`;
  // How tall what's between the field and the bottom bar can be, from where it comes out: above
  // the start button, or `bottom` up from the output's bottom.
  const list = () => {
    const height = outputSize(props.output)?.height;
    if (height === undefined) return LIST;
    // Where it comes out, from the output's top.
    const from = props.from?.layout();
    const bottom = from ? from.y - (props.gap ?? 0) : height - (props.bottom ?? 0);
    return Math.max(LEAST, Math.min(LIST, bottom - EDGE - AROUND));
  };
  // What Escape does before it closes the menu, which the menu says as it changes: true when it
  // did something.
  let back: () => boolean = () => false;
  return (
    <Flyout
      output={props.output}
      popup={key}
      from={props.from}
      side="top"
      align={props.align ?? "center"}
      anchor={["bottom"]}
      gap={props.from ? props.gap : props.bottom}
      keyboard
      padding={0}
      radius={14}
      onKey={(event) => event.key === "Escape" && event.pressed && back()}
    >
      <Menu
        output={props.output}
        onClose={() => popup.close(key)}
        list={list()}
        back={(fn) => (back = fn)}
      />
    </Flyout>
  );
}

type Page = "main" | "all" | "recent" | "power";

/** The menu, whose part between the field and the bottom bar is `list` tall. */
function Menu(props: {
  output: string;
  onClose: () => void;
  list: number;
  back: (fn: () => boolean) => void;
}) {
  const search = everythingSearch({ headers: "all", heights: HEIGHTS, done: props.onClose });
  search.reset();
  let field: any;
  const [page, setPage] = createSignal<Page>("main");
  // The group that's open, by its key, and whether its name is being changed.
  const [open, setOpen] = createSignal<string | null>(null);
  const [renaming, setRenaming] = createSignal(false);
  props.back(() => {
    if (renaming()) setRenaming(false);
    else if (open()) setOpen(null);
    else if (page() !== "main" && !search.query().trim()) setPage("main");
    else return false;
    field?.focus();
    return true;
  });

  const close = props.onClose;
  const launch = (app: App) => {
    start(app);
    close();
  };
  // What a right click on an app offers, and on a group.
  const appMenu = (app: App, group?: string): MenuItem[] => {
    const items: MenuItem[] = [
      { label: app.name, icon: appIcon(app), run: () => launch(app) },
      "separator",
      isPinned(app)
        ? { label: "Unpin from Start", icon: "view-pin-symbolic", run: () => pin(app, false) }
        : { label: "Pin to Start", icon: "view-pin-symbolic", run: () => pin(app, true) },
      onTaskbar(app)
        ? { label: "Unpin from Taskbar", icon: "view-pin-symbolic", run: () => pinToTaskbar(app, false) }
        : { label: "Pin to Taskbar", icon: "view-pin-symbolic", run: () => pinToTaskbar(app, true) },
    ];
    if (group !== undefined) {
      items.push({ label: "Remove from Group", icon: "list-remove-symbolic", run: () => takeOut(group, app) });
    }
    return items;
  };
  const groupMenu = (item: Extract<Item, { group: string }>): MenuItem[] => [
    { label: "Open", icon: "folder-open-symbolic", run: () => setOpen(item.key) },
    {
      label: "Rename…",
      icon: "document-edit-symbolic",
      run: () => {
        setOpen(item.key);
        setRenaming(true);
      },
    },
    { label: "Ungroup", icon: "view-grid-symbolic", run: () => ungroup(item.key) },
    "separator",
    { label: "Unpin from Start", icon: "view-pin-symbolic", run: () => unpinItem(item.key) },
  ];
  const menuAt = (event: PointerEvent, items: MenuItem[]) =>
    openMenu({ output: event.output ?? props.output, x: event.x, y: event.y }, items);

  // Changes to the pins, by the items' keys.
  const edit = (fn: (items: Item[]) => Item[]) => savePins(fn(pinned()));
  const unpinItem = (key: string) => edit((items) => items.filter((item) => item.key !== key));
  const ungroup = (key: string) => {
    if (open() === key) setOpen(null);
    edit((items) =>
      items.flatMap((item) =>
        item.key === key && isGroup(item)
          ? item.apps.map((app) => ({ key: `app:${app.id}`, app }))
          : [item]));
  };
  /** Takes `app` out of the group `key`, to right after it. */
  const takeOut = (key: string, app: App) =>
    edit((items) =>
      items.flatMap((item) => {
        if (item.key !== key || !isGroup(item)) return [item];
        const rest = { ...item, apps: item.apps.filter((one) => one !== app) };
        return [rest, { key: `app:${app.id}`, app }].filter((one) => !isGroup(one) || one.apps.length > 0);
      }));
  const rename = (key: string, name: string) => {
    setRenaming(false);
    const group = name.trim();
    if (group) edit((items) => items.map((item) => (item.key === key && isGroup(item) ? { ...item, group } : item)));
    field?.focus();
  };

  const recent = () => (settings.recent ? data("recent")?.files ?? [] : []);
  // The folders along the bottom, asked for as it opens and as the setting changes.
  createEffect(
    () => [...settings.folders],
    (folders) => command("folders", { folders }),
  );

  return (
    <box style={{ flexDirection: "column", width: WIDTH + 2 * PAD }}>
      <box style={{ flexDirection: "column", gap: GAP, padding: [PAD, PAD, GAP, PAD] }}>
        <box
          style={{
            flexDirection: "row", alignItems: "center", gap: 10, height: 42,
            padding: [0, 6, 0, 14], borderRadius: 21, background: "fg/0.08", fontSize: 15,
          }}
        >
          <image icon="system-search-symbolic" style={{ color: "fg/0.6" }} />
          <input
            ref={field}
            autofocus
            placeholder={search.placeholder()}
            onInput={(text) => {
              setOpen(null);
              search.type(text);
            }}
            onSubmit={search.submit}
            onBeforeKey={search.beforeKey}
            onKey={(event) => search.key(event, field)}
          />
          <FilesPill search={search} />
        </box>
        <box style={{ height: props.list, flexDirection: "column" }}>
          <Show
            when={!search.query().trim()}
            fallback={
              <Found
                search={search}
                height={props.list}
                onMenu={(event, line) =>
                  menuAt(event, line.kind === "app" ? appMenu(line.app) : fileMenu(line.file, close))}
              />
            }
          >
            <Show
              when={!search.filesOnly()}
              fallback={<Middle height={props.list}>Type to find files and folders by name</Middle>}
            >
              <Show when={page() === "main"}>
                <MainPage
                  height={props.list}
                  recent={recent()}
                  open={open()}
                  onOpen={setOpen}
                  renaming={renaming()}
                  onRename={(key) => {
                    setOpen(key);
                    setRenaming(true);
                  }}
                  rename={rename}
                  onLaunch={launch}
                  onTakeOut={takeOut}
                  onAll={() => setPage("all")}
                  onMore={() => setPage("recent")}
                  onClose={close}
                  appMenu={(event, app, group) => menuAt(event, appMenu(app, group))}
                  groupMenu={(event, item) => menuAt(event, groupMenu(item))}
                  fileMenu={(event, file) => menuAt(event, fileMenu(file, close))}
                />
              </Show>
              <Show when={page() === "all"}>
                <AllApps
                  height={props.list}
                  onBack={() => setPage("main")}
                  onLaunch={launch}
                  appMenu={(event, app) => menuAt(event, appMenu(app))}
                />
              </Show>
              <Show when={page() === "recent"}>
                <RecentPage
                  height={props.list}
                  files={recent()}
                  onBack={() => setPage("main")}
                  onClose={close}
                  fileMenu={(event, file) => menuAt(event, fileMenu(file, close))}
                />
              </Show>
              <Show when={page() === "power"}>
                <PowerPage onDone={close} />
              </Show>
            </Show>
          </Show>
        </box>
      </box>
      <BottomBar
        power={page() === "power"}
        onPower={() => setPage(page() === "power" ? "main" : "power")}
        onClose={close}
      />
    </box>
  );
}

/** What a right click on a file offers. */
const fileMenu = (file: FoundFile, close: () => void): MenuItem[] => [
  { label: "Open", icon: file.icon, run: () => { openFile(file); close(); } },
  { label: "Show in Folder", icon: "folder-open-symbolic", run: () => { showFile(file); close(); } },
];

/** Words in the middle of what's `height` tall. */
function Middle(props: { height: number; children: string }) {
  return (
    <box style={{ height: props.height, padding: [22, 14], alignItems: "center", justifyContent: "center" }}>
      <text style={{ color: "fg/0.6", textAlign: "center" }}>{props.children}</text>
    </box>
  );
}

/** A section's title, with a button at its end. */
function Head(props: { title: string; button?: string; back?: boolean; onClick?: () => void }) {
  return (
    <box
      style={{
        height: HEAD, flexShrink: 0, flexDirection: "row", alignItems: "start",
        justifyContent: "space-between", padding: [0, 0, 0, 8],
      }}
    >
      <text style={{ fontWeight: 700 }}>{props.title}</text>
      <Show when={props.button}>
        <box
          style={{
            flexDirection: "row", alignItems: "center", gap: 4, height: 24, borderRadius: 12,
            padding: props.back ? [0, 12, 0, 8] : [0, 8, 0, 12], background: "fg/0.08",
            fontSize: 12, fontWeight: 600, ":hover": { background: "fg/0.14" },
          }}
          onClick={() => props.onClick?.()}
        >
          <Show when={props.back}>
            <image icon="go-previous-symbolic" size={12} />
          </Show>
          <text>{props.button}</text>
          <Show when={!props.back}>
            <image icon="go-next-symbolic" size={12} />
          </Show>
        </box>
      </Show>
    </box>
  );
}

/** What's found for what's typed, whose apps and files a right click opens a menu for. */
function Found(props: {
  search: ReturnType<typeof everythingSearch>;
  height: number;
  onMenu: (event: PointerEvent, line: Extract<Line, { kind: "app" | "file" }>) => void;
}) {
  const search = props.search;
  return (
    <Scroller style={{ height: props.height }} inner={{ flexDirection: "column" }} reveal={search.reveal()}>
      <For each={search.lines()}>
        {(line) => (
          <SearchLine
            line={line}
            search={search}
            heights={HEIGHTS}
            iconSize={ICON}
            fontSize={14}
            onMenu={props.onMenu}
          />
        )}
      </For>
      <Show when={search.nothing()}>
        <box style={{ padding: [22, 14], justifyContent: "center" }}>
          <text style={{ color: "fg/0.6" }}>No Results</text>
        </box>
      </Show>
    </Scroller>
  );
}

/** An app's icon over its name, or a group's: the first four of its apps' icons in a square.
 * `lit` while an app dragged over it would go onto it, and `mark` the side one would go beside it
 * on. */
function Tile(props: {
  item: Item;
  width: number;
  lit?: boolean;
  mark?: "before" | "after" | null;
  style?: any;
  onClick?: () => void;
  onRightClick?: (event: PointerEvent) => void;
  onPress?: (event: PointerEvent) => void;
  onDrag?: (event: PointerEvent) => void;
  onRelease?: () => void;
}) {
  const item = () => props.item;
  return (
    <box
      style={[
        {
          width: props.width, height: TILE.height, flexDirection: "column", alignItems: "center",
          justifyContent: "center", gap: 8, borderRadius: 10, padding: [0, 4],
          background: props.lit ? "accent/0.25" : undefined,
          ":hover": { background: props.lit ? "accent/0.25" : "fg/0.1" },
        },
        props.style,
      ]}
      onClick={() => props.onClick?.()}
      onRightClick={(event) => props.onRightClick?.(event)}
      onPress={(event) => props.onPress?.(event)}
      onDrag={(event) => props.onDrag?.(event)}
      onRelease={() => props.onRelease?.()}
    >
      <Show when={props.mark}>
        <box
          style={{
            position: "absolute", top: 12, bottom: 12, width: 3, borderRadius: 2,
            background: "accent", left: props.mark === "before" ? -1 : undefined,
            right: props.mark === "after" ? -1 : undefined,
          }}
        />
      </Show>
      <Show
        when={isGroup(item())}
        fallback={<image icon={appIcon((item() as { app: App }).app)} size={ICON} />}
      >
        <box
          style={{
            width: ICON, height: ICON, borderRadius: 9, background: "fg/0.14", padding: 3,
            flexDirection: "row", flexWrap: "wrap", gap: 2,
          }}
        >
          <For each={(item() as { apps: App[] }).apps.slice(0, 4)}>
            {(app) => <image icon={appIcon(app)} size={12} />}
          </For>
        </box>
      </Show>
      <text style={{ fontSize: 12, lineClamp: 1, textAlign: "center" }}>
        {isGroup(item()) ? (item() as { group: string }).group : (item() as { app: App }).app.name}
      </text>
    </box>
  );
}

/** Where a pinned app being dragged would go: onto another, to make a group with it or join
 * its group, or between them, at `before`, the place it'd have among the others. */
type Drop = { onto: number } | { before: number };

/** The pinned apps, then the files opened lately, if they're shown and there are any. */
function MainPage(props: {
  height: number;
  recent: RecentFile[];
  open: string | null;
  onOpen: (key: string | null) => void;
  renaming: boolean;
  onRename: (key: string) => void;
  rename: (key: string, name: string) => void;
  onLaunch: (app: App) => void;
  onTakeOut: (key: string, app: App) => void;
  onAll: () => void;
  onMore: () => void;
  onClose: () => void;
  appMenu: (event: PointerEvent, app: App, group?: string) => void;
  groupMenu: (event: PointerEvent, item: Extract<Item, { group: string }>) => void;
  fileMenu: (event: PointerEvent, file: RecentFile) => void;
}) {
  const items = createMemo(pinned);
  const files = () => props.recent.slice(0, RECENT);
  const recentHeight = () =>
    files().length > 0 ? GAP + HEAD + Math.ceil(files().length / 2) * HEIGHTS.row : 0;
  const gridHeight = () => Math.max(TILE.height, props.height - HEAD - recentHeight());
  const opened = () => items().find((item) => item.key === props.open && isGroup(item)) as
    | Extract<Item, { group: string }>
    | undefined;

  // A press on a tile that may become a drag, and the drag, by where the pointer is from where
  // it was pressed.
  let pressed: { at: number; x: number; y: number } | null = null;
  const [dragged, setDragged] = createSignal<{ at: number; dx: number; dy: number } | null>(null);
  // A press that went on to drag isn't a click.
  let dropped = false;
  const drop = (): Drop | null => {
    const drag = dragged();
    if (!drag) return null;
    const count = items().length;
    // Where the pointer is, in tiles, from the grid's top left.
    const px = (drag.at % COLUMNS) + 0.5 + drag.dx / TILE.width;
    const py = Math.floor(drag.at / COLUMNS) + 0.5 + drag.dy / TILE.height;
    const column = Math.max(0, Math.min(COLUMNS - 1, Math.floor(px)));
    const row = Math.max(0, Math.floor(py));
    const cell = Math.min(count, row * COLUMNS + column);
    const [fx, fy] = [px - Math.floor(px), py - row];
    // Over the middle of another, an app goes onto it; a group never goes into another.
    const middle = fx > 0.25 && fx < 0.75 && fy > 0.2 && fy < 0.8;
    if (middle && cell < count && cell !== drag.at && !isGroup(items()[drag.at])) {
      return { onto: cell };
    }
    return { before: Math.min(count, fx < 0.5 ? cell : cell + 1) };
  };
  const release = () => {
    pressed = null;
    const where = drop();
    const drag = dragged();
    setDragged(null);
    if (!where || !drag) return;
    dropped = true;
    after(0, () => (dropped = false));
    const list = [...items()];
    const moving = list[drag.at];
    if ("onto" in where) {
      const target = list[where.onto];
      const app = (moving as { app: App }).app;
      list[where.onto] = isGroup(target)
        ? { ...target, apps: [...target.apps, app] }
        : { key: `group:new`, group: NEW_GROUP, apps: [(target as { app: App }).app, app] };
      list.splice(drag.at, 1);
    } else {
      list.splice(drag.at, 1);
      list.splice(where.before > drag.at ? where.before - 1 : where.before, 0, moving);
    }
    savePins(list);
  };

  return (
    <box style={{ flexDirection: "column", height: props.height }}>
      <Head title="Pinned" button="All apps" onClick={props.onAll} />
      <Show
        when={items().length > 0}
        fallback={
          <Middle height={gridHeight()}>Pin apps here from All apps, with a right click</Middle>
        }
      >
        <Scroller
          style={{ height: gridHeight() }}
          inner={{ flexDirection: "row", flexWrap: "wrap", alignContent: "start" }}
        >
          <For each={items()}>
            {(item, at) => {
              const moving = () => dragged()?.at === at();
              const where = () => drop();
              const onto = () => {
                const w = where();
                return w !== null && "onto" in w && w.onto === at();
              };
              // Where it'd go, but for its own place, which changes nothing.
              const between = () => {
                const w = where();
                const from = dragged()?.at;
                if (w === null || !("before" in w) || from === undefined) return null;
                return w.before === from || w.before === from + 1 ? null : w.before;
              };
              const before = () => between() === at();
              const after = () => between() === items().length && at() === items().length - 1;
              return (
                <Tile
                  item={item}
                  width={TILE.width}
                  lit={onto()}
                  mark={before() ? "before" : after() ? "after" : null}
                  style={{
                    translate: moving() ? [dragged()!.dx, dragged()!.dy] : [0, 0],
                    opacity: moving() ? 0.8 : 1,
                    transition: moving() ? { frame: 0.04, translate: 0.04 } : undefined,
                  }}
                  onPress={(event) => {
                    if (event.button === 1) pressed = { at: at(), x: event.x, y: event.y };
                  }}
                  onDrag={(event) => {
                    if (!pressed) return;
                    const dx = event.x - pressed.x;
                    const dy = event.y - pressed.y;
                    if (!dragged() && Math.hypot(dx, dy) < SLOP) return;
                    setDragged({ at: pressed.at, dx, dy });
                  }}
                  onRelease={release}
                  onClick={() => {
                    if (dropped) return;
                    if (isGroup(item)) props.onOpen(item.key);
                    else props.onLaunch(item.app);
                  }}
                  onRightClick={(event) =>
                    isGroup(item) ? props.groupMenu(event, item) : props.appMenu(event, item.app)}
                />
              );
            }}
          </For>
        </Scroller>
      </Show>
      <Show when={files().length > 0}>
        <box style={{ height: GAP }} />
        <Head title="Recent" button="More" onClick={props.onMore} />
        <box style={{ flexDirection: "row", flexWrap: "wrap", columnGap: 8 }}>
          <For each={files()}>
            {(file) => (
              <FileRow
                file={file}
                width={(WIDTH - 8) / 2}
                onClose={props.onClose}
                onMenu={(event) => props.fileMenu(event, file)}
              />
            )}
          </For>
        </box>
      </Show>
      <Show when={opened()}>
        <GroupCard
          item={opened()!}
          height={props.height}
          renaming={props.renaming}
          onRename={() => props.onRename(opened()!.key)}
          rename={(name) => props.rename(opened()!.key, name)}
          onDismiss={() => props.onOpen(null)}
          onLaunch={props.onLaunch}
          onTakeOut={(app) => {
            props.onTakeOut(opened()!.key, app);
            props.onOpen(null);
          }}
          appMenu={(event, app) => props.appMenu(event, app, opened()!.key)}
        />
      </Show>
    </box>
  );
}

/** An open group: its name, which a click changes, and its apps, over the dimmed menu. An app
 * dragged out of it and let go is taken out of the group. */
function GroupCard(props: {
  item: Extract<Item, { group: string }>;
  height: number;
  renaming: boolean;
  onRename: () => void;
  rename: (name: string) => void;
  onDismiss: () => void;
  onLaunch: (app: App) => void;
  onTakeOut: (app: App) => void;
  appMenu: (event: PointerEvent, app: App) => void;
}) {
  const columns = 4;
  const width = 420;
  const tile = (width - 2 * PAD) / columns;
  let root: SceneNode | undefined;
  let card: SceneNode | undefined;
  // A press on an app that may become a drag, and the drag: the app, and where the pointer is on
  // the output. The app's grid scrolls and cuts what it holds, so what's dragged is a copy of
  // its tile over the whole menu.
  let pressed: { app: App; x: number; y: number } | null = null;
  const [dragged, setDragged] = createSignal<{ app: App; x: number; y: number } | null>(null);
  let dropped = false;
  const outside = () => {
    const drag = dragged();
    const rect = card?.layout();
    if (!drag || !rect) return false;
    return drag.x < rect.x || drag.x > rect.x + rect.width
      || drag.y < rect.y || drag.y > rect.y + rect.height;
  };
  const release = () => {
    pressed = null;
    const drag = dragged();
    const out = outside();
    setDragged(null);
    if (!drag) return;
    dropped = true;
    after(0, () => (dropped = false));
    if (out) props.onTakeOut(drag.app);
  };
  // Where the copy is, in the menu.
  const ghost = () => {
    const drag = dragged();
    const rect = root?.layout();
    if (!drag || !rect) return null;
    return { left: drag.x - rect.x - tile / 2, top: drag.y - rect.y - TILE.height / 2 };
  };
  return (
    <box
      ref={root}
      style={{
        position: "absolute", top: 0, left: 0, right: 0, height: props.height,
        alignItems: "center", justifyContent: "center", background: "window/0.6", borderRadius: 10,
      }}
      enter={{ opacity: 0 }}
      exit={{ opacity: 0 }}
      onClick={() => !dropped && props.onDismiss()}
    >
      <box
        ref={card}
        style={{
          width, maxHeight: props.height - 24, flexDirection: "column", gap: 8, padding: PAD,
          borderRadius: 14, background: "window/0.9", borderWidth: 1, borderColor: "fg/0.1",
          opacity: outside() ? 0.6 : 1,
        }}
        enter={{ scale: 0.96, opacity: 0 }}
        exit={{ scale: 0.96, opacity: 0 }}
        // A click in it stays in it.
        onClick={() => {}}
      >
        <Show
          when={props.renaming}
          fallback={
            <box
              style={{
                height: 32, borderRadius: 8, alignItems: "center", justifyContent: "center",
                ":hover": { background: "fg/0.08" },
              }}
              onClick={props.onRename}
            >
              <text style={{ fontSize: 15, fontWeight: 700 }}>{props.item.group}</text>
            </box>
          }
        >
          <box
            style={{
              height: 32, borderRadius: 8, padding: [0, 10], background: "fg/0.08",
              flexDirection: "row", alignItems: "center", fontSize: 15,
            }}
          >
            <input
              ref={(node) => node.setText(props.item.group)}
              autofocus
              placeholder="Group Name"
              onSubmit={props.rename}
            />
          </box>
        </Show>
        <Scroller
          style={{ height: Math.min(3, Math.ceil(props.item.apps.length / columns)) * TILE.height }}
          inner={{ flexDirection: "row", flexWrap: "wrap", alignContent: "start" }}
        >
          <For each={props.item.apps}>
            {(app) => (
              <Tile
                item={{ key: app.id, app }}
                width={tile}
                style={{ opacity: dragged()?.app === app ? 0.3 : 1 }}
                onPress={(event) => {
                  if (event.button === 1) pressed = { app, x: event.x, y: event.y };
                }}
                onDrag={(event) => {
                  if (!pressed) return;
                  const far = Math.hypot(event.x - pressed.x, event.y - pressed.y) >= SLOP;
                  if (dragged() || far) setDragged({ app, x: event.x, y: event.y });
                }}
                onRelease={release}
                onClick={() => !dropped && props.onLaunch(app)}
                onRightClick={(event) => props.appMenu(event, app)}
              />
            )}
          </For>
        </Scroller>
      </box>
      <Show when={ghost()}>
        <Tile
          item={{ key: dragged()!.app.id, app: dragged()!.app }}
          width={tile}
          style={{
            position: "absolute", left: ghost()!.left, top: ghost()!.top, opacity: 0.9,
            transition: { frame: 0.04, translate: 0.04 },
          }}
        />
      </Show>
    </box>
  );
}

/** A file opened lately: its icon, its name and when, which a click opens. */
function FileRow(props: {
  file: RecentFile;
  width: number;
  where?: boolean;
  onClose: () => void;
  onMenu: (event: PointerEvent) => void;
}) {
  const time = now(60_000);
  return (
    <box
      style={{
        width: props.width, height: HEIGHTS.row, flexDirection: "row", alignItems: "center",
        gap: 12, padding: [0, 10], borderRadius: 10, ":hover": { background: "fg/0.1" },
      }}
      onClick={() => {
        openFile(props.file);
        props.onClose();
      }}
      onRightClick={props.onMenu}
    >
      <image icon={props.file.icon} size={ICON - 8} style={{ color: "fg/0.85" }} />
      <box style={{ flexDirection: "column", flexShrink: 1, minWidth: 0 }}>
        <text style={{ fontSize: 13, lineClamp: 1 }}>{props.file.name}</text>
        <text style={{ fontSize: 12, color: "fg/0.6", lineClamp: 1 }}>
          {props.where
            ? `${ago(props.file.opened, time())} · ${props.file.location}`
            : ago(props.file.opened, time())}
        </text>
      </box>
    </box>
  );
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** When `iso` was, from `at`: "12 min ago", "3 h ago", "Yesterday", "Nov 18". */
export function ago(iso: string, at: Date) {
  const then = new Date(iso);
  if (Number.isNaN(then.getTime())) return "";
  const minutes = Math.floor((at.getTime() - then.getTime()) / 60_000);
  if (minutes < 1) return "Just now";
  if (minutes < 60) return `${minutes} min ago`;
  const today = new Date(at.getFullYear(), at.getMonth(), at.getDate());
  if (then >= today) return `${Math.floor(minutes / 60)} h ago`;
  const yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1);
  if (then >= yesterday) return "Yesterday";
  const day = `${MONTHS[then.getMonth()]} ${then.getDate()}`;
  return then.getFullYear() === at.getFullYear() ? day : `${day}, ${then.getFullYear()}`;
}

/** All the files opened lately that are still there, with where they are. */
function RecentPage(props: {
  height: number;
  files: RecentFile[];
  onBack: () => void;
  onClose: () => void;
  fileMenu: (event: PointerEvent, file: RecentFile) => void;
}) {
  return (
    <box style={{ flexDirection: "column", height: props.height }}>
      <Head title="Recent" button="Back" back onClick={props.onBack} />
      <Show
        when={props.files.length > 0}
        fallback={<Middle height={props.height - HEAD}>No files opened lately</Middle>}
      >
        <Scroller style={{ height: props.height - HEAD }} inner={{ flexDirection: "column" }}>
          <For each={props.files}>
            {(file) => (
              <FileRow
                file={file}
                width={WIDTH}
                where
                onClose={props.onClose}
                onMenu={(event) => props.fileMenu(event, file)}
              />
            )}
          </For>
        </Scroller>
      </Show>
    </box>
  );
}

/** The letter an app goes under: the first of its name, without accents, or `#`. */
function letterOf(app: App) {
  const first = app.name.normalize("NFD").charAt(0).toUpperCase();
  return first >= "A" && first <= "Z" ? first : "#";
}

const LETTERS = "#ABCDEFGHIJKLMNOPQRSTUVWXYZ".split("");

/** All the apps from A to Z, under their letters, with every letter down the side to jump to. */
function AllApps(props: {
  height: number;
  onBack: () => void;
  onLaunch: (app: App) => void;
  appMenu: (event: PointerEvent, app: App) => void;
}) {
  // Each letter with apps, its apps and how far down it is.
  const sections = (() => {
    const all = apps();
    let top = 0;
    return LETTERS.flatMap((letter) => {
      const list = all.filter((app) => letterOf(app) === letter);
      if (list.length === 0) return [];
      const section = { letter, apps: list, top };
      top += LETTER + list.length * ROW;
      return [section];
    });
  })();
  let scrollTo: (top: number) => void = () => {};
  const height = () => props.height - HEAD;
  // The letters down the side, which need room for each.
  const rail = () => height() >= LETTERS.length * 14;
  return (
    <box style={{ flexDirection: "column", height: props.height }}>
      <Head title="All apps" button="Back" back onClick={props.onBack} />
      <box style={{ flexDirection: "row", height: height() }}>
        <Scroller
          style={{ height: height(), flexGrow: 1 }}
          inner={{ flexDirection: "column" }}
          scroller={(fn) => (scrollTo = fn)}
        >
          <For each={sections}>
            {(section) => (
              <box style={{ flexDirection: "column" }}>
                <box
                  style={{
                    height: LETTER, flexDirection: "column", justifyContent: "end",
                    alignItems: "start", padding: [0, 0, 4, 6],
                  }}
                >
                  <box
                    style={{
                      height: 24, minWidth: 24, borderRadius: 8, background: "fg/0.08",
                      alignItems: "center", justifyContent: "center", padding: [0, 8],
                    }}
                  >
                    <text style={{ fontSize: 12, fontWeight: 700 }}>{section.letter}</text>
                  </box>
                </box>
                <For each={section.apps}>
                  {(app) => (
                    <box
                      style={{
                        height: ROW, flexDirection: "row", alignItems: "center", gap: 12,
                        padding: [0, 10], borderRadius: 10, ":hover": { background: "fg/0.1" },
                      }}
                      onClick={() => props.onLaunch(app)}
                      onRightClick={(event) => props.appMenu(event, app)}
                    >
                      <image icon={appIcon(app)} size={ICON - 4} />
                      <text style={{ fontSize: 13, lineClamp: 1 }}>{app.name}</text>
                    </box>
                  )}
                </For>
              </box>
            )}
          </For>
        </Scroller>
        <Show when={rail()}>
          <box style={{ width: 20, flexDirection: "column", justifyContent: "center", alignItems: "center" }}>
            <For each={LETTERS}>
              {(letter) => {
                const section = sections.find((one) => one.letter === letter);
                return (
                  <box
                    style={{
                      height: 14, width: 20, alignItems: "center", justifyContent: "center",
                      borderRadius: 4, ":hover": section ? { background: "fg/0.1" } : undefined,
                    }}
                    onClick={() => section && scrollTo(section.top)}
                  >
                    <text
                      style={{
                        fontSize: 10, fontWeight: 700, color: section ? "fg" : "fg/0.3",
                      }}
                    >
                      {letter}
                    </text>
                  </box>
                );
              }}
            </For>
          </box>
        </Show>
      </box>
    </box>
  );
}

/** Along the bottom: the user, then the folders that are there, bento's settings and power. */
function BottomBar(props: { power: boolean; onPower: () => void; onClose: () => void }) {
  const user = () => data("user");
  const folders = () => data("folders")?.folders ?? [];
  const round = { width: 36, height: 36, borderRadius: 18 };
  return (
    <box style={{ flexDirection: "column", background: "window/0.25" }}>
      <box style={{ height: 1, background: "fg/0.08" }} />
      <box style={{ flexDirection: "row", alignItems: "center", gap: 6, padding: [10, PAD] }}>
        <Show when={user()}>
          <box style={{ flexDirection: "row", alignItems: "center", gap: 10, flexShrink: 1, minWidth: 0 }}>
            <box
              style={{
                width: 32, height: 32, borderRadius: 16, overflow: "hidden", flexShrink: 0,
                background: "accent/0.6", alignItems: "center", justifyContent: "center",
              }}
            >
              <Show
                when={user()!.picture}
                fallback={
                  <text style={{ fontWeight: 700 }}>
                    {(user()!.name.trim().charAt(0) || "?").toUpperCase()}
                  </text>
                }
              >
                <image icon={user()!.picture!} size={32} />
              </Show>
            </box>
            <text style={{ fontSize: 13, fontWeight: 600, lineClamp: 1 }}>{user()!.name}</text>
          </box>
        </Show>
        <box style={{ flexGrow: 1 }} />
        <For each={folders()}>
          {(folder) => (
            <Button
              icon={folder.icon}
              style={round}
              onClick={() => {
                openFile({ path: folder.path, name: folder.name, location: "", folder: true, icon: folder.icon });
                props.onClose();
              }}
            />
          )}
        </For>
        <Show when={folders().length > 0}>
          <box style={{ width: 1, height: 18, background: "fg/0.15", margin: [0, 4] }} />
        </Show>
        <Button
          icon="emblem-system-symbolic"
          style={round}
          onClick={() => {
            action("spawn", ["bento-settings"]);
            props.onClose();
          }}
        />
        <Button
          icon={props.power ? "go-previous-symbolic" : "system-shutdown-symbolic"}
          style={round}
          onClick={props.onPower}
        />
      </box>
    </box>
  );
}
