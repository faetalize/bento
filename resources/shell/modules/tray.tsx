// The tray: apps' status icons, as they give them over StatusNotifierItem. A click activates
// one, a right click opens its menu, and a middle click does its other thing.

import { command, data } from "bento";
import { For } from "solid-js";

export const trayItems = () => data("tray")?.items ?? [];

/** The tray's icons in a row. */
export function Tray(props: { size?: number; style?: any }) {
  return (
    <box style={[{ flexDirection: "row", alignItems: "center", gap: 2 }, props.style]}>
      <For each={trayItems()}>
        {(item: any) => (
          <box
            style={{
              width: 28, height: 28, borderRadius: 6, alignItems: "center", justifyContent: "center",
              ":hover": { background: "fg/0.08" },
            }}
            onClick={() => command("activate", { id: item.id })}
            onRightClick={() => command("context", { id: item.id })}
            onMiddleClick={() => command("secondary", { id: item.id })}
          >
            <image icon={item.icon ?? "application-x-executable-symbolic"} size={props.size ?? 16} />
          </box>
        )}
      </For>
    </box>
  );
}
