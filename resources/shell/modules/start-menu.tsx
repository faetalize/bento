// A start menu: the apps, found by typing as the launcher finds them, or all of them in a grid,
// and the power menu. It comes out of the start button, or up from the bottom of the output, in its
// middle.

import { apps, type App, type SceneNode } from "bento";
import { createSignal, For, Show } from "solid-js";
import { Button, Flyout, Scroller, popup } from "bento:modules/ui";
import { AppRow, appIcon, appSearch, preloadIcons, start } from "bento:modules/apps";
import { PowerPage } from "bento:modules/power";

const WIDTH = 640;
const COLUMNS = 6;
// How tall the list of apps is, which scrolls.
const LIST = 420;
const ICON = 32;

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

/** The start menu of `output`, which `popup` opens as `start:<output>`: over the start button
 * `from`, lined up with it as `align` says, or else `bottom` up from the bottom of the output. */
export function StartMenu(props: {
  output: string;
  from?: SceneNode;
  align?: "start" | "center" | "end";
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
      gap={props.from ? undefined : props.bottom}
      keyboard
      padding={16}
      radius={14}
    >
      <Menu onClose={() => popup.close(key)} />
    </Flyout>
  );
}

function Menu(props: { onClose: () => void }) {
  const search = appSearch(8);
  const all = apps();
  const [power, setPower] = createSignal(false);
  const pick = (app: App) => {
    start(app);
    props.onClose();
  };
  return (
    <box style={{ flexDirection: "column", gap: 14, width: WIDTH }}>
      <box
        style={{
          flexDirection: "row", alignItems: "center", gap: 10, padding: [9, 14], borderRadius: 20,
          background: "fg/0.08", fontSize: 15,
        }}
      >
        <image icon="system-search-symbolic" style={{ color: "fg/0.6" }} />
        <input
          autofocus
          placeholder="Search for apps"
          onInput={search.type}
          onSubmit={() => search.current() && pick(search.current()!)}
          onKey={search.key}
        />
      </box>
      <Show
        when={search.query().trim()}
        fallback={
          <Show
            when={!power()}
            fallback={<PowerPage onDone={props.onClose} />}
          >
            <text style={{ fontWeight: 700, paddingLeft: 8 }}>All apps</text>
            <Scroller
              style={{ height: LIST }}
              inner={{ flexDirection: "row", flexWrap: "wrap", alignContent: "start" }}
            >
              <For each={all}>{(app) => <AppTile app={app} onClick={() => pick(app)} />}</For>
            </Scroller>
          </Show>
        }
      >
        <box style={{ flexDirection: "column", gap: 2, minHeight: LIST + 30 }}>
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
