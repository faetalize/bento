// The launcher: a glass search field in the middle of the current output that finds apps and
// files as they're typed, its results opening below it. It grows out of a point, and shrinks back
// into one. Its bind opens it, and Escape or a press elsewhere closes it.

import { Surface, focusedOutput, onAction, outputs, useModule, type Meta } from "bento";
import { createEffect, createSignal, For, Show } from "solid-js";
import { Scroller, grows, growsWith, popup } from "bento:modules/ui";
import { preloadIcons } from "bento:modules/apps";
import { FilesPill, SearchLine, everythingSearch } from "bento:modules/search";

export const meta = {
  actions: {
    "toggle-launcher": {
      description: "Show or hide the launcher on the current monitor.",
      bind: "Mod+D",
    },
  },
} satisfies Meta;

useModule(import.meta, meta);

// How big an icon is in a row.
const ICON = 40;
const HEIGHTS = { head: 30, row: 56, more: 40 };
// How tall the launcher is with all its results: it's held at the top of that room, in the
// middle of the output, so its search stays put as the results come and go.
const HEIGHT = 640;
const RADIUS = 26;
// How tall its list gets, the rest of that room, past which it scrolls.
const LIST = HEIGHT - 24 - 48 - 8;

export function Launcher() {
  preloadIcons(ICON);
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
  const close = () => popup.close("launcher");
  const search = everythingSearch({ headers: "second", heights: HEIGHTS, done: close });
  let field: any;
  // Opened, it starts empty, with the apps as they are now, and the keys.
  createEffect(
    () => props.open,
    (open) => {
      if (!open) return;
      search.reset();
      field?.setText("");
      field?.focus();
    },
  );
  const showing = () => search.lines().length > 0 || search.nothing();
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
        <box style={{ flexDirection: "column", gap: showing() ? 8 : 0 }} {...growsWith}>
          <box
            style={{
              flexDirection: "row", alignItems: "center", gap: 10, height: 48,
              padding: [0, 8, 0, 14], fontSize: 22,
            }}
          >
            <input
              ref={field}
              placeholder={search.placeholder()}
              onInput={search.type}
              onSubmit={search.submit}
              onBeforeKey={search.beforeKey}
              onKey={(event) => search.key(event, field)}
            />
            <FilesPill search={search} />
          </box>
          <Show when={search.lines().length > 0}>
            <Scroller
              style={{ height: Math.min(LIST, search.height()) }}
              inner={{ flexDirection: "column" }}
              reveal={search.reveal()}
            >
              <For each={search.lines()}>
                {(line) => (
                  <SearchLine
                    line={line}
                    search={search}
                    heights={HEIGHTS}
                    iconSize={ICON}
                    fontSize={16}
                  />
                )}
              </For>
            </Scroller>
          </Show>
          <Show when={search.nothing()}>
            <box style={{ padding: [14, 14], justifyContent: "center" }}>
              <text style={{ color: "fg/0.6" }}>No Results</text>
            </box>
          </Show>
        </box>
      </box>
    </Surface>
  );
}
