// Files: finding them by their names, and opening them. It shows nothing itself: what searches with
// it shows what it finds its own way, and says when to search.

import { command, data, onData, type FoundFile } from "bento";
import { createMemo, createSignal, getOwner, onCleanup } from "solid-js";

// The last search asked for. It starts from the time, so that what one asked for before the shell
// loaded again finds is never taken for another's.
let last = Date.now();

/** Whether files can be found, which takes `fd`: for a search field to say so, for one. It's false
 * until the data program says, and follows `fd` being installed or removed from the next search. */
export const filesAvailable = () => data("file-search")?.available ?? false;

/** Where files are looked for: folders written as settings write them, with `~` for the home
 * folder. */
export interface Where {
  folders?: string[];
  excluded?: string[];
}

/** A search for files: what's looked for, and the best `most` that match it, 50 at most. One
 * search runs at a time, so only one of these looks for something at once. */
export function fileSearch(most = 8) {
  const [query, setQuery] = createSignal("");
  const [found, setFound] = createSignal<FoundFile[]>([]);
  // What was found before, while something else is looked for, is only what that matches too: it
  // has the same words in its name.
  const files = createMemo(() => {
    const words = query().toLowerCase().split(/\s+/).filter(Boolean);
    return found().filter((file) => words.every((word) => file.name.toLowerCase().includes(word)));
  });
  const [searching, setSearching] = createSignal(false);
  // The search that's on its way.
  let running: { id: number; query: string; folders?: string[]; excluded?: string[] } | null =
    null;

  const ask = (text: string, where: Where) => {
    running = { id: ++last, query: text, ...where };
    command("files-search", running);
  };
  const stop = () => {
    if (running) command("files-stop", { id: running.id });
    running = null;
  };
  const search = (text: string, where: Where = {}) => {
    const wanted = text.trim();
    setQuery(text);
    setSearching(wanted !== "");
    // What was found stays until there's something in its place, so that it doesn't blink, and
    // asking for another search ends the one on its way.
    if (wanted) ask(wanted, where);
    else {
      stop();
      setFound([]);
    }
  };

  onData("files", (event) => {
    if (event.id !== running?.id) return;
    running = null;
    setSearching(false);
    setFound(event.files.slice(0, most));
  });
  // The data program started again, and knows nothing of the search it was on.
  onData("ready", () => {
    if (running) ask(running.query, { folders: running.folders, excluded: running.excluded });
  });
  if (getOwner()) onCleanup(stop);

  return {
    query,
    /** The files that match, the best first. While `searching()`, they're those of the search
     * before that match what's looked for now. */
    files,
    /** Whether the query is still being looked for. */
    searching,
    /** Looks for `text` right away, in `where`'s folders, the home folder unless it says, but
     * for those it leaves out. */
    search,
    /** Ends the search on its way, keeping what was found, as when what searches closes. */
    stop,
    /** Forgets what's looked for and found, as when what searches opens or closes. */
    clear: () => search(""),
  };
}

export type FileSearch = ReturnType<typeof fileSearch>;

/** Opens a file with its app, or a folder in the file manager. */
export const openFile = (file: FoundFile) =>
  command("files-open", { path: file.path, token: true });

/** Shows a file in its folder, in the file manager. */
export const showFile = (file: FoundFile) =>
  command("files-show", { path: file.path, token: true });
