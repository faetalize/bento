// A start menu: apps and files, found by typing as the launcher finds them, or all the apps in a
// grid, and the power menu. It comes out of the start button, or up from the bottom of the output, in its
// middle.

import {
  apps, focusedOutput, onAction, outputs, useModule, type App, type Meta, type SceneNode,
} from "bento";
import { createSignal, For, Show } from "solid-js";
import { Button, Flyout, Scroller, popup } from "bento:modules/ui";
import { appIcon, preloadIcons, start } from "bento:modules/apps";
import { FilesPill, SearchLine, everythingSearch } from "bento:modules/search";
import { PowerPage } from "bento:modules/power";

export const meta = {
  actions: {
    "toggle-start-menu": {
      description: "Show or hide the start menu on the current monitor.",
      bind: "Mod",
    },
  },
} satisfies Meta;

useModule(import.meta, meta);

onAction("toggle-start-menu", ({ output }) => {
  popup.toggle(`start:${output ?? focusedOutput() ?? outputs()[0]}`);
});

const WIDTH = 640;
const COLUMNS = 6;
// How tall the list of apps is, which scrolls.
const LIST = 420;
const ICON = 32;
const HEIGHTS = { head: 28, row: 48, more: 40 };

/** One app in the grid: its icon over its name. */
function AppTile(props: { app: App; onClick: () => void }) {
  return (
    <box
      style={{
        width: (WIDTH - 32) / COLUMNS, height: 88, flexDirection: "column", alignItems: "center",
        justifyContent: "center", gap: 8, borderRadius: 10, padding: [0, 4],
        ":hover": { background: "fg/0.1" },
      }}
      onClick={props.onClick}
    >
      <image icon={appIcon(props.app)} size={ICON} />
      <text style={{ fontSize: 12, lineClamp: 1, textAlign: "center" }}>{props.app.name}</text>
    </box>
  );
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
      padding={16}
      radius={14}
    >
      <Menu onClose={() => popup.close(key)} />
    </Flyout>
  );
}

function Menu(props: { onClose: () => void }) {
  const search = everythingSearch({ headers: "all", heights: HEIGHTS, done: props.onClose });
  search.reset();
  const all = apps();
  let field: any;
  const [power, setPower] = createSignal(false);
  const pick = (app: App) => {
    start(app);
    props.onClose();
  };
  return (
    <box style={{ flexDirection: "column", gap: 14, width: WIDTH }}>
      <box
        style={{
          flexDirection: "row", alignItems: "center", gap: 10, height: 42, padding: [0, 6, 0, 14],
          borderRadius: 21, background: "fg/0.08", fontSize: 15,
        }}
      >
        <image icon="system-search-symbolic" style={{ color: "fg/0.6" }} />
        <input
          ref={field}
          autofocus
          placeholder={search.placeholder()}
          onInput={search.type}
          onSubmit={search.submit}
          onBeforeKey={search.beforeKey}
          onKey={(event) => search.key(event, field)}
        />
        <FilesPill search={search} />
      </box>
      <Show
        when={search.query().trim()}
        fallback={
          <Show
            when={!power()}
            fallback={<PowerPage onDone={props.onClose} />}
          >
            <Show
              when={!search.filesOnly()}
              fallback={
                <box style={{ height: LIST + 30, padding: [22, 14], alignItems: "center" }}>
                  <text style={{ color: "fg/0.6" }}>Type to find files and folders by name</text>
                </box>
              }
            >
            <text style={{ fontWeight: 700, paddingLeft: 8 }}>All apps</text>
            <Scroller
              style={{ height: LIST }}
              inner={{ flexDirection: "row", flexWrap: "wrap", alignContent: "start" }}
            >
              <For each={all}>{(app) => <AppTile app={app} onClick={() => pick(app)} />}</For>
            </Scroller>
            </Show>
          </Show>
        }
      >
        <Scroller
          style={{ height: LIST + 30 }}
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
                fontSize={14}
              />
            )}
          </For>
          <Show when={search.nothing()}>
            <box style={{ padding: [22, 14], alignItems: "center" }}>
              <text style={{ color: "fg/0.6" }}>No Results</text>
            </box>
          </Show>
        </Scroller>
      </Show>
      <box style={{ flexDirection: "row", justifyContent: "end" }}>
        <Button
          icon={power() ? "go-previous-symbolic" : "system-shutdown-symbolic"}
          style={{ width: 36, height: 36, borderRadius: 18 }}
          onClick={() => setPower(!power())}
        />
      </box>
    </box>
  );
}
