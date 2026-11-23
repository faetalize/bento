// Searching apps and files together, as the launcher and the start menu do: one list, apps then
// files, which of it is selected, and the keys that go through it. Apps show at once and files
// come a moment later, so files only ever add rows under the apps, and the row that's selected
// stays put as they come. When no app's name matches, only what it's for, files go first, and
// nothing shows until they're there. A Files pill searches files and folders alone, and `/`,
// typed anywhere, switches it, since no name has one. Its settings say whether apps are searched
// with files, and which folders files are searched in.

import {
  apps, useModule, type App, type FoundFile, type InputNode, type KeyEvent, type Meta, type Style,
} from "bento";
import { createMemo, createSignal, Show } from "solid-js";
import { ACTIVE } from "bento:modules/ui";
import { appIcon, score, start } from "bento:modules/apps";
import { fileSearch, filesAvailable, openFile, showFile } from "bento:modules/files";

/** A line of the list: a kind's header, an app, a file, or the row that shows the rest of a kind. */
export type Line =
  | { kind: "head"; key: string; title: string }
  | { kind: "app"; key: string; app: App }
  | { kind: "file"; key: string; file: FoundFile }
  | { kind: "more"; key: string; of: "apps" | "files"; title: string; next: string };

/** How tall each kind of line is, for the list to know where each is. */
export interface Heights {
  head: number;
  row: number;
  more: number;
}

const TITLES = { apps: "Apps", files: "Files" } as const;

export const meta = {
  settings: {
    "files-with-apps": {
      type: "boolean",
      default: true,
      description:
        "Find files and folders along with apps. Without it, the Files pill and / still search "
        + "them alone.",
      page: "shell",
      group: "Search",
    },
    folders: {
      type: "folder",
      list: true,
      default: ["~"],
      description:
        "Where files and folders are found, with the folders in them. With none, they aren't found.",
      page: "shell",
      group: "Search",
    },
    "excluded-folders": {
      type: "folder",
      list: true,
      default: [],
      description:
        "Folders whose files are never found, unless a folder in them is one to search.",
      page: "shell",
      group: "Search",
    },
  },
} satisfies Meta;

const { settings } = useModule(import.meta, meta);

/** Whether files can be found: with `fd`, and in some folder. */
const findable = () => filesAvailable() && settings.folders.length > 0;

/** A search through apps and files. `each` is how many of a kind show before "Show All", and a
 * kind that's alone shows `alone`. `headers` says which kinds have a header: `"all"`, or only the
 * second, which says where the first ends. `done` is called once something's opened. */
export function everythingSearch(options: {
  each?: number;
  alone?: number;
  headers?: "all" | "second";
  heights: Heights;
  done: () => void;
}) {
  const each = options.each ?? 4;
  const alone = options.alone ?? 8;
  const files = fileSearch(50);
  let all = apps();
  const [query, setQuery] = createSignal("");
  // Files and folders alone.
  const [only, setOnly] = createSignal(false);
  // The kinds shown whole.
  const [whole, setWhole] = createSignal<string[]>([]);
  // The line picked, by its key: the first one when it's none or gone.
  const [picked, setPicked] = createSignal<string | null>(null);
  // The line picked with the keys, which the list scrolls to.
  const [moved, setMoved] = createSignal<string | null>(null);

  // A line is made once for each key, so that the list keeps the same row for it.
  let made = new Map<string, Line>();
  const lines = createMemo(() => {
    const q = query().trim().toLowerCase();
    const was = made;
    made = new Map();
    const line = <L extends Line>(next: L): L => {
      const same = (was.get(next.key) as L | undefined) ?? next;
      made.set(next.key, same);
      return same;
    };
    if (!q) return [] as Line[];
    const found = files.files();
    const sections: { of: "apps" | "files"; items: Line[]; most: number }[] = [];
    const fileLines = found.map((file) => line({ kind: "file", key: `file:${file.path}`, file }));
    if (only()) {
      sections.push({ of: "files", items: fileLines, most: Infinity });
    } else {
      const scored: [number, App][] = [];
      for (const app of all) {
        const fit = score(app, q);
        if (fit !== null) scored.push([fit, app]);
      }
      scored.sort((a, b) => a[0] - b[0]);
      const appLines = scored.map(([, app]) => line({ kind: "app", key: `app:${app.id}`, app }));
      const named = scored.some(([fit]) => fit < 3);
      // Apps take the room of both only once there are surely no files: rows only ever come under
      // them.
      const withFiles = findable() && settings["files-with-apps"];
      const noFiles = !withFiles || (!files.searching() && found.length === 0);
      const apps = { of: "apps" as const, items: appLines, most: noFiles ? alone : each };
      const fileSection = {
        of: "files" as const,
        items: fileLines,
        most: appLines.length ? each : alone,
      };
      if (!withFiles) sections.push(apps);
      else if (named || !appLines.length) sections.push(apps, fileSection);
      // Files go first, and nothing shows until they're here, as rows come only under others.
      else if (!files.searching() || found.length) sections.push(fileSection, apps);
    }
    const out: Line[] = [];
    for (const section of sections.filter((s) => s.items.length)) {
      const headed = options.headers === "all" ? !only() : out.length > 0;
      if (headed) out.push(line({ kind: "head", key: `head:${section.of}`, title: TITLES[section.of] }));
      // One more than fits shows itself rather than a row that shows it.
      const cut = !whole().includes(section.of) && section.items.length > section.most + 1;
      out.push(...(cut ? section.items.slice(0, section.most) : section.items));
      if (cut) {
        const title = `Show All ${section.items.length} ${TITLES[section.of]}`;
        // What it shows first, which is picked as it does.
        const next = section.items[section.most].key;
        const key = `more:${section.of}:${section.items.length}`;
        out.push(line({ kind: "more", key, of: section.of, title, next }));
      }
    }
    return out;
  });
  const choices = createMemo(() => lines().filter((line) => line.kind !== "head"));
  const current = () => choices().find((line) => line.key === picked()) ?? choices()[0];

  // Files are looked for along with apps only as the settings say, and alone with the pill.
  const lookFor = (text: string, alone: boolean) => {
    const wanted = findable() && (alone || settings["files-with-apps"]);
    files.search(wanted ? text : "", {
      folders: [...settings.folders],
      excluded: [...settings["excluded-folders"]],
    });
  };
  const type = (text: string) => {
    setQuery(text);
    setWhole([]);
    setPicked(null);
    setMoved(null);
    lookFor(text, only());
  };
  const setFilesOnly = (on: boolean) => {
    const alone = on && findable();
    // With files found along with apps, they're already looked for.
    const before = only() || settings["files-with-apps"];
    setOnly(alone);
    setPicked(null);
    setMoved(null);
    if (before !== (alone || settings["files-with-apps"])) lookFor(query(), alone);
  };
  const move = (key: string | undefined) => {
    if (key === undefined) return;
    setPicked(key);
    setMoved(key);
  };
  const step = (by: number) => {
    const list = choices();
    if (!list.length) return;
    const at = list.indexOf(current()!);
    move(list[(at + by + list.length) % list.length].key);
  };
  // The first row of the next kind, or the one before.
  const jump = (by: number) => {
    const list = choices();
    const kindOf = (line: Line) => (line.kind === "more" ? line.of : line.kind === "app" ? "apps" : "files");
    const kinds = [...new Set(list.map(kindOf))];
    if (!kinds.length) return;
    const now = current() ? kinds.indexOf(kindOf(current()!)) : 0;
    const next = kinds[(now + by + kinds.length) % kinds.length];
    move(list.find((line) => kindOf(line) === next)?.key);
  };

  /** Does what `line` is for: opens it, shows a file in its folder with `show`, or shows the rest
   * of a kind. */
  const activate = (line: Line | undefined, show = false) => {
    if (!line || line.kind === "head") return;
    if (line.kind === "more") {
      setWhole([...whole(), line.of]);
      move(line.next);
      return;
    }
    if (line.kind === "app") start(line.app);
    else if (show) showFile(line.file);
    else openFile(line.file);
    options.done();
  };

  const reveal = createMemo((): [number, number] | null => {
    const key = moved();
    if (!key) return null;
    let top = 0;
    for (const line of lines()) {
      const height = options.heights[line.kind === "app" || line.kind === "file" ? "row" : line.kind];
      if (line.key === key) return [top, top + height];
      top += height;
    }
    return null;
  });

  return {
    query,
    lines,
    /** The key of the line that's selected. */
    selected: () => current()?.key ?? null,
    select: (key: string) => setPicked(key),
    /** Where the line picked with the keys is, from the list's top, to scroll to. */
    reveal,
    /** How tall the list is with all its lines. */
    height: () =>
      lines().reduce(
        (sum, line) =>
          sum + options.heights[line.kind === "app" || line.kind === "file" ? "row" : line.kind],
        0,
      ),
    filesOnly: only,
    setFilesOnly,
    /** Whether files can be searched, which takes `fd` and a folder to search. */
    canFindFiles: findable,
    /** Whether nothing matched what's typed. */
    nothing: () => query().trim() !== "" && !files.searching() && lines().length === 0,
    placeholder: () =>
      !findable() ? "Search apps"
        : only() ? "Search files and folders" : "Search apps, or / for files and folders",
    type,
    activate,
    /** Opens the selected line. */
    submit: () => activate(current()),
    /** Starts again, empty, with the apps as they are now, as when what searches opens. */
    reset() {
      all = apps();
      setOnly(false);
      type("");
    },
    /** For the field's `onBeforeKey`: `/` switches the Files pill, and Ctrl+Enter shows the
     * selected file in its folder. */
    beforeKey(event: KeyEvent) {
      if (event.text === "/" && findable()) {
        setFilesOnly(!only());
        return true;
      }
      if ((event.key === "Return" || event.key === "KP_Enter") && event.ctrl) {
        activate(current(), true);
        return true;
      }
      return false;
    },
    /** For the `field`'s `onKey`: Up, Down and Tab go through the list, Backspace in an empty
     * field turns the pill off, and Escape empties the field, then turns the pill off, and is left
     * to what holds it, to close, once there's nothing left to undo. */
    key(event: KeyEvent, field: InputNode | undefined) {
      const text = field?.text ?? query();
      if (event.key === "Down") step(1);
      else if (event.key === "Up") step(-1);
      else if (event.key === "Tab" && !event.shift) jump(1);
      else if (event.key === "ISO_Left_Tab" || event.key === "Tab") jump(-1);
      else if (event.key === "BackSpace" && only() && text === "") setFilesOnly(false);
      else if (event.key === "Escape" && text !== "") {
        field?.setText("");
        type("");
      } else if (event.key === "Escape" && only()) setFilesOnly(false);
      else return false;
      return true;
    },
  };
}

export type EverythingSearch = ReturnType<typeof everythingSearch>;

/** The Files pill, at the end of a search field: lit while only files and folders are searched. */
export function FilesPill(props: { search: EverythingSearch }) {
  return (
    <Show when={props.search.canFindFiles()}>
      <box
        style={{
          padding: [6, 14], borderRadius: 16, fontSize: 13, fontWeight: 600, flexShrink: 0,
          background: props.search.filesOnly() ? ACTIVE : "fg/0.08",
          color: props.search.filesOnly() ? "fg" : "fg/0.6",
          cursor: "pointer",
          ":hover": props.search.filesOnly() ? undefined : { background: "fg/0.14" },
        }}
        onClick={() => props.search.setFilesOnly(!props.search.filesOnly())}
      >
        <text>Files</text>
      </box>
    </Show>
  );
}

/** A line of the list, as both the launcher and the start menu draw it: an app or a file with its
 * icon, its name and what it's for or where it is, a kind's header, or the row that shows the rest
 * of one. The selected file has a button that shows it in its folder. */
export function SearchLine(props: {
  line: Line;
  search: EverythingSearch;
  heights: Heights;
  iconSize: number;
  fontSize: number;
}) {
  const line = props.line;
  const selected = () => props.search.selected() === line.key;
  const pick = () => props.search.select(line.key);
  if (line.kind === "head") {
    return (
      <box
        style={{
          height: props.heights.head, flexDirection: "column", justifyContent: "end",
          padding: [0, 12, 4], flexShrink: 0,
        }}
      >
        <text style={{ fontSize: 12, fontWeight: 700, color: "fg/0.6" }}>{line.title}</text>
      </box>
    );
  }
  const row: Style = {
    flexDirection: "row", alignItems: "center", gap: 14, padding: [0, 12], borderRadius: 14,
    flexShrink: 0,
  };
  if (line.kind === "more") {
    return (
      <box
        style={[row, { height: props.heights.more, background: selected() ? "fg/0.1" : undefined }]}
        onClick={() => props.search.activate(line)}
        onHover={({ inside }) => inside && pick()}
      >
        <box style={{ width: props.iconSize, flexShrink: 0 }} />
        <text style={{ fontSize: props.fontSize - 2, color: "fg/0.75" }}>{line.title}</text>
      </box>
    );
  }
  const [icon, name, detail] = line.kind === "app"
    ? [appIcon(line.app), line.app.name, line.app.comment ?? ""]
    : [line.file.icon, line.file.name, line.file.location];
  return (
    <box
      style={[row, { height: props.heights.row, background: selected() ? "fg/0.1" : undefined }]}
      onClick={() => props.search.activate(line)}
      onHover={({ inside }) => inside && pick()}
    >
      <image icon={icon} size={props.iconSize} />
      <box style={{ flexDirection: "column", flexGrow: 1, flexShrink: 1, minWidth: 0 }}>
        <text style={{ fontSize: props.fontSize, fontWeight: 600, lineClamp: 1 }}>{name}</text>
        <Show when={detail}>
          <text style={{ fontSize: props.fontSize - 3, color: "fg/0.6", lineClamp: 1 }}>
            {detail}
          </text>
        </Show>
      </box>
      <Show when={line.kind === "file" && selected()}>
        <box
          style={{
            flexDirection: "row", gap: 7, padding: [6, 10], borderRadius: 9, fontSize: 12,
            background: "fg/0.12", flexShrink: 0, cursor: "pointer",
            ":hover": { background: "fg/0.18" },
          }}
          onClick={() => props.search.activate(line, true)}
        >
          <text style={{ color: "fg/0.6" }}>Ctrl+Enter</text>
          <text>Show in Folder</text>
        </box>
      </Show>
    </box>
  );
}
