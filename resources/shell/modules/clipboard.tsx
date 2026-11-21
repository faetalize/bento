// The clipboard's history: what was copied, newest first, found by typing, and copied again
// with a click or Enter. The data program keeps it, for the session only.

import { command, data } from "bento";
import { createSignal, For, Show } from "solid-js";
import { Button } from "bento:modules/ui";

const WIDTH = 380;
// How many things show at once; typing finds the others.
const ROWS = 8;
// How tall a picture's row is.
const PICTURE = 96;

const history = () => data("clipboard") ?? { available: false, entries: [] };

// A text on one line.
const flat = (value: string) => value.replace(/\s+/g, " ").trim();

/** The clipboard's history, to copy from. `onPicked` hears something copied from it. */
export function ClipboardHistory(props: { onPicked?: () => void }) {
  const [query, setQuery] = createSignal("");
  const [selected, setSelected] = createSignal(0);
  const shown = () => {
    const wanted = query().trim().toLowerCase();
    return history()
      .entries.filter((entry: any) => !wanted || (entry.text ?? "").toLowerCase().includes(wanted))
      .slice(0, ROWS);
  };
  const select = (index: number) => {
    const count = shown().length;
    if (count > 0) setSelected((index + count) % count);
  };
  const pick = (index: number) => {
    const entry = shown()[index];
    if (!entry) return;
    command("clipboard-copy", { id: entry.id });
    props.onPicked?.();
  };
  const why = () =>
    !history().available ? "The clipboard can't be watched here."
    : history().entries.length === 0 ? "Nothing's been copied yet."
    : shown().length === 0 ? "Nothing copied matches."
    : "";
  return (
    <box style={{ flexDirection: "column", gap: 10, width: WIDTH }}>
      <box style={{ flexDirection: "row", gap: 8, alignItems: "center" }}>
        <box
          style={{
            flexGrow: 1, flexShrink: 1, minWidth: 0, padding: [9, 12], borderRadius: 12,
            background: "fg/0.08", fontSize: 15,
          }}
        >
          <input
            placeholder="Search the clipboard"
            autofocus
            onInput={(typed) => {
              setQuery(typed);
              setSelected(0);
            }}
            onSubmit={() => pick(selected())}
            onKey={(event) => {
              if (!event.pressed) return false;
              if (event.key === "Down" || (event.key === "Tab" && !event.shift)) {
                select(selected() + 1);
              } else if (event.key === "Up" || event.key === "ISO_Left_Tab") {
                select(selected() - 1);
              } else if (event.key === "Delete" && event.shift && shown()[selected()]) {
                command("clipboard-delete", { id: shown()[selected()].id });
              } else {
                return false;
              }
              return true;
            }}
          />
        </box>
        <Show when={history().entries.length > 0}>
          <Button
            icon="edit-clear-all-symbolic"
            style={{ background: "fg/0.08", height: 34, borderRadius: 12 }}
            onClick={() => command("clipboard-clear", {})}
          />
        </Show>
      </box>
      <Show when={why()}>
        <box style={{ padding: [6, 12] }}>
          <text style={{ color: "fg/0.6" }}>{why()}</text>
        </box>
      </Show>
      <Show when={shown().length > 0}>
        <box style={{ flexDirection: "column", gap: 2 }}>
          <For each={shown()}>
            {(entry: any, index) => (
              <box
                style={{
                  flexDirection: "row", alignItems: "center", gap: 8, padding: [5, 5, 5, 12],
                  borderRadius: 12, cursor: "pointer",
                  background: index() === Math.min(selected(), shown().length - 1) ? "fg/0.1" : undefined,
                }}
                onClick={() => pick(index())}
                onHover={({ inside }) => inside && select(index())}
              >
                <Show
                  when={entry.kind === "image"}
                  fallback={
                    <box style={{ flexGrow: 1, flexShrink: 1, minWidth: 0 }}>
                      <text style={{ lineClamp: 1 }}>{flat(entry.text ?? "")}</text>
                    </box>
                  }
                >
                  <box
                    style={{
                      flexGrow: 1, flexShrink: 1, minWidth: 0, height: PICTURE, borderRadius: 10,
                      overflow: "hidden",
                    }}
                  >
                    <image src={entry.image} style={{ width: WIDTH - 60, height: PICTURE }} />
                  </box>
                </Show>
                <Button
                  icon="window-close-symbolic"
                  size={14}
                  style={{ height: 26, borderRadius: 13 }}
                  onClick={() => command("clipboard-delete", { id: entry.id })}
                />
              </box>
            )}
          </For>
        </box>
      </Show>
    </box>
  );
}
