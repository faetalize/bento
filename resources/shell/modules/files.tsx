// Files: finding them by their names, and opening them. It shows nothing itself: what searches with
// it shows what it finds its own way, and says when to search.

import { command, onData, type FoundFile } from "bento";
import { createSignal, getOwner, onCleanup } from "solid-js";

// The last search asked for. It starts from the time, so that what one asked for before the shell
// loaded again finds is never taken for another's.
let last = Date.now();

/** A search for files: what's looked for, and the best `most` that match it, 50 at most. One
 * search runs at a time, so only one of these looks for something at once. */
export function fileSearch(most = 8) {
  const [query, setQuery] = createSignal("");
  const [files, setFiles] = createSignal<FoundFile[]>([]);
  const [searching, setSearching] = createSignal(false);
  const [available, setAvailable] = createSignal(true);
  // The search that's on its way.
  let running: { id: number; query: string } | null = null;

  const ask = (text: string) => {
    running = { id: ++last, query: text };
    command("files-search", running);
  };
  const stop = () => {
    if (running) command("files-stop", { id: running.id });
    running = null;
  };
  const search = (text: string) => {
    const wanted = text.trim();
    setQuery(text);
    setSearching(wanted !== "");
    // What was found stays until there's something in its place, so that it doesn't blink, and
    // asking for another search ends the one on its way.
    if (wanted) ask(wanted);
    else {
      stop();
      setFiles([]);
    }
  };

  onData("files", (event) => {
    if (event.id !== running?.id) return;
    running = null;
    setSearching(false);
    setAvailable(event.available);
    setFiles(event.files.slice(0, most));
  });
  // The data program started again, and knows nothing of the search it was on.
  onData("ready", () => running && ask(running.query));
  if (getOwner()) onCleanup(stop);

  return {
    query,
    /** The files that match, the best first. While `searching()`, they're still those of the
     * search before. */
    files,
    /** Whether the query is still being looked for. */
    searching,
    /** Whether files can be found at all, which takes `fd`, as far as the last search knows. */
    available,
    /** Looks for `text` right away. */
    search,
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
