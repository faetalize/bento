// The launcher: a glass search field in the middle of the current output that finds apps as
// they're typed, its results opening below it. It grows out of a point, and shrinks back into
// one. Its bind opens it, and Escape or a press elsewhere closes it.

import { Surface, focusedOutput, onAction, outputs, useModule, type Meta } from "bento";
import { createEffect, createSignal, For, Show } from "solid-js";
import { grows, growsWith, popup } from "bento:modules/ui";
import { AppRow, appSearch, preloadIcons, start } from "bento:modules/apps";

export const meta = {
  actions: {
    "toggle-launcher": {
      description: "Show or hide the launcher on the current monitor.",
      bind: "Mod+D",
    },
  },
} satisfies Meta;

useModule(import.meta, meta);

// How big an app's icon is in a row.
const ICON = 40;
const RESULTS = 8;
// How tall the launcher is with all its results: it's held at the top of that room, in the
// middle of the output, so its search stays put as the results come and go.
const HEIGHT = 544;
const RADIUS = 26;

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
  const search = appSearch(RESULTS);
  let field: any;
  // Opened, it starts empty, with the apps as they are now, and the keys.
  createEffect(
    () => props.open,
    (open) => {
      if (!open) return;
      search.refresh();
      field?.setText("");
      field?.focus();
    },
  );
  const close = () => popup.close("launcher");
  const pick = (app: any) => {
    start(app);
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
        <box
          style={{ flexDirection: "column", gap: search.matches().length ? 8 : 0 }}
          {...growsWith}
        >
          <box style={{ padding: [10, 14], fontSize: 22 }}>
            <input
              ref={field}
              placeholder="Search apps"
              onInput={search.type}
              onSubmit={() => search.current() && pick(search.current())}
              onKey={search.key}
            />
          </box>
          <box style={{ flexDirection: "column", gap: 2 }}>
            <For each={search.matches()}>
              {(app, index) => (
                <AppRow
                  app={app}
                  iconSize={ICON}
                  selected={index() === search.selected()}
                  onClick={() => pick(app)}
                  onHover={() => search.select(index())}
                />
              )}
            </For>
          </box>
        </box>
      </box>
    </Surface>
  );
}
