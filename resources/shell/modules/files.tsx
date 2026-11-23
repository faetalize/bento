// Files: finding them by what's typed, and opening them. It shows nothing itself: what searches
// with it shows what it finds its own way.

import { after, command, onData, type FoundFile } from "bento";
import { createSignal, getOwner, onCleanup } from "solid-js";

// Fewer letters than this match far too much to be worth looking for.
const SHORTEST = 2;
// How long typing pauses before what's typed is looked for, in milliseconds.
const PAUSE = 80;

// The last search asked for. It starts from the time, so that what one asked for before the shell
// loaded again finds is never taken for another's.
let last = Date.now();

/** A search for files, as it's typed: what's typed, and the best `most` that match it, 50 at most.
 * One search runs at a time, so only one of these looks for something at once. */
export function fileSearch(most = 8) {
  const [query, setQuery] = createSignal("");
  const [files, setFiles] = createSignal<FoundFile[]>([]);
  const [searching, setSearching] = createSignal(false);
  const [available, setAvailable] = createSignal(true);
  // The search that's on its way, and what it looks for.
  let running: { id: number; query: string } | null = null;
  let waiting = () => {};

  const ask = (text: string) => {
    running = { id: ++last, query: text };
    command("files-search", running);
  };
  const stop = () => {
    waiting();
    if (running) command("files-stop", { id: running.id });
    running = null;
  };
  const type = (text: string) => {
    stop();
    setQuery(text);
    const wanted = text.trim();
    const enough = [...wanted].length >= SHORTEST;
    setSearching(enough);
    // What was found stays until there's something in its place, so that it doesn't blink.
    if (enough) waiting = after(PAUSE, () => ask(wanted));
    else setFiles([]);
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
    /** The files that match, the best first. While `searching()`, they're still those of what was
     * typed before. */
    files,
    /** Whether what's typed is still being looked for. */
    searching,
    /** Whether files can be found at all, which takes `fd`, as far as the last search knows. */
    available,
    type,
    /** Forgets what's typed and found, as when a search opens or closes. */
    clear: () => type(""),
  };
}

export type FileSearch = ReturnType<typeof fileSearch>;

/** Opens a file with its app, or a folder in the file manager. */
export const openFile = (file: FoundFile) =>
  command("files-open", { path: file.path, token: true });

/** Shows a file in its folder, in the file manager. */
export const showFile = (file: FoundFile) =>
  command("files-show", { path: file.path, token: true });
