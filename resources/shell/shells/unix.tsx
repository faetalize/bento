// bento's own shell: three islands of glass along the top of each output. The left one shows the
// workspaces and the minimap, the middle one the clock, which opens into the calendar, and the
// right one the status, which opens into the control center, with notifications under it. While
// a window is fullscreen on their output the islands leave the screen, and the pointer at the
// top edge brings them back. The launcher, the volume indicator and the workspace editor come
// and go on the focused output.

import {
  Surface, after, bentoStyle, focusedOutput, fullscreen, outputs, useOutput,
} from "bento";
import { createSignal, For } from "solid-js";
import { ISLAND, Island, popup } from "bento:modules/ui";
import { Minimap, Workspaces } from "bento:modules/workspaces";
import { ClockIsland } from "bento:modules/clock";
import { StatusIsland } from "bento:modules/control-center";
import { Toasts } from "bento:modules/notifications";
import { Launcher } from "bento:modules/launcher";
import { VolumeIndicator } from "bento:modules/sound";
import { WorkspaceEditor } from "bento:modules/workspace-editor";

// How near the top edge the pointer brings the islands back over a fullscreen window, and how
// long after it leaves them they go again, in milliseconds.
const EDGE = 2;
const LINGER = 400;

export default function Unix() {
  return (
    <>
      {/* Each output's own, so that what's open in its islands is its own to close: a press in
          one output's islands is outside the other's. */}
      <For each={outputs()}>
        {(output) => (
          <Surface
            output={output}
            anchor={["top", "left", "right"]}
            layer="overlay"
            z={-1}
            reserve={bentoStyle().gaps + ISLAND}
            keyboard={busy(output) ? "exclusive" : "none"}
            onEscape={() => popup.close()}
            onOutside={() => busy(output) && popup.close()}
            style={{ flexDirection: "column" }}
          >
            <Bar />
          </Surface>
        )}
      </For>
      <Launcher />
      <VolumeIndicator />
      <WorkspaceEditor />
    </>
  );
}

/** Whether an island of `output` is open into something. */
function busy(output: string) {
  const open = popup.current();
  if (!open) return false;
  const [kind, on] = open.split(":");
  return ["calendar", "control", "clipboard"].includes(kind) && on === output;
}

function Bar() {
  const output = useOutput();
  const gaps = () => bentoStyle().gaps;
  const [statusNode, setStatusNode] = createSignal<any>(null);
  const [toasts, setToasts] = createSignal(0);
  // Whether the pointer is at the islands, which keeps them over a fullscreen window.
  const [near, setNear] = createSignal(false);
  let leaving: (() => void) | null = null;
  // Gone, they're left out, so nothing is drawn over the window and it goes straight to the
  // display.
  const [gone, setGone] = createSignal(false);
  // The control center lists the notifications itself, so their toasts make way for it.
  const listing = () => popup.is(`control:${output}`);
  const away = () => fullscreen(output) && !near() && !busy(output) && toasts() === 0;
  return (
    <box style={{ flexDirection: "column" }}>
      {/* Where the pointer keeps the islands on screen over a fullscreen window: they go a
          moment after it leaves, so that brushing past the edge doesn't send them back and
          forth. */}
      <box
        style={{
          position: "absolute", top: 0, left: 0, right: 0, animate: false,
          height: away() ? EDGE : gaps() + ISLAND + gaps(),
        }}
        onNear={({ inside }) => {
          leaving?.();
          leaving = null;
          if (inside) setNear(true);
          else leaving = after(LINGER, () => setNear(false));
        }}
      />
      <box
        style={{
          flexDirection: "row", alignItems: "start", padding: [gaps(), gaps(), 0, gaps()],
          // They slide up off the screen and are left out there, and slide back from there.
          translate: [0, away() ? -(ISLAND + gaps()) : 0],
          visibility: away() && gone() ? "hidden" : "visible",
        }}
        onRest={() => setGone(away())}
      >
        <box style={{ flexDirection: "row", flexGrow: 1, width: 0, justifyContent: "start" }}>
          <Island padding={[0, 10, 0, 5]} gap={8}>
            <Workspaces />
            <Minimap />
          </Island>
        </box>
        <ClockIsland />
        <box style={{ flexDirection: "row", flexGrow: 1, width: 0, justifyContent: "end" }}>
          <StatusIsland
            ref={setStatusNode}
          />
        </box>
      </box>
      {/* Notifications come out under the status island of the focused output. */}
      <Toasts
        output={output}
        from={statusNode()}
        held={listing() || focusedOutput() !== output}
        onCount={setToasts}
      />
    </box>
  );
}
