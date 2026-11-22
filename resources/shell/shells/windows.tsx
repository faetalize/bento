// A shell laid out like Windows 11's: a taskbar along the bottom of each output, its apps in the
// middle after the start button and the overview's, and at its end, in three areas, the tray, the
// sound output switcher and the clipboard, and quick settings with the clock and the
// notifications, then a sliver that shows the desktop.
// Notifications come up over the taskbar's end. The taskbar is in the top layer, so fullscreen
// windows cover it.

import {
  Surface, action, focusedOutput, outputs, useModule, type Meta, type Style,
} from "bento";
import { createSignal, For, Show } from "solid-js";
import { Divider, Flyout, popup } from "bento:modules/ui";
import { ClipboardHistory } from "bento:modules/clipboard";
import { Clock, Month } from "bento:modules/clock";
import { Toasts, notifications } from "bento:modules/notifications";
import { NotificationCenter, QuickSettings } from "bento:modules/control-center";
import { networkIcon } from "bento:modules/network";
import { activeDevice, OutputSwitcher, VolumeIndicator, volumeIcon } from "bento:modules/sound";
import { battery } from "bento:modules/power";
import { OverviewButton } from "bento:modules/overview";
import { TaskbarApps } from "bento:modules/taskbar";
import { StartMenu } from "bento:modules/start-menu";
import { Tray } from "bento:modules/tray";
import { WorkspaceEditor } from "bento:modules/workspace-editor";

export const meta = {
  settings: {
    height: {
      type: "number",
      default: 48,
      page: "shell",
      group: "Taskbar",
    },
    alignment: {
      type: "choice",
      choices: ["center", "left"],
      default: "center",
      description: "Where the start button and the apps are along the taskbar.",
      page: "shell",
      group: "Taskbar",
    },
  },
} satisfies Meta;

const { settings } = useModule(import.meta, meta);

/** The buttons of quick things, like the sound output switcher: the size of the tray's icons. */
const SMALL: Style = {
  width: 28, height: 28, paddingLeft: 0, paddingRight: 0, borderRadius: 6,
  ":hover": { background: "fg/0.08" },
};

/** How tall the taskbar is, with room for its buttons. */
const height = () => Math.max(40, settings.height);

/** How far a panel is from a button `size` tall on the taskbar, for it to come out the same
 * distance from the taskbar's edge whichever button it's from: close enough to melt into it as
 * one piece, never so far that it half melts. */
const over = (size: number) => (height() - size) / 2 + 2;

export default function Windows() {
  return (
    <>
      <For each={outputs()}>{(output) => <Taskbar output={output} />}</For>
      <VolumeIndicator />
      <WorkspaceEditor />
    </>
  );
}

/** A button on the taskbar: lit while what it opens is open. */
function BarButton(props: {
  ref?: (node: any) => void;
  onClick: () => void;
  open?: boolean;
  children: any;
  style?: any;
}) {
  return (
    <box
      ref={(node) => props.ref?.(node)}
      style={[
        {
          flexDirection: "row", alignItems: "center", justifyContent: "center", gap: 8,
          height: 40, minWidth: 40, padding: [0, 8], borderRadius: 8,
          background: props.open ? "fg/0.1" : undefined, ":hover": { background: "fg/0.14" },
        },
        props.style,
      ]}
      onClick={props.onClick}
    >
      {props.children}
    </box>
  );
}

function Taskbar(props: { output: string }) {
  const output = props.output;
  const start = `start:${output}`;
  const quick = `quick:${output}`;
  const calendar = `calendar:${output}`;
  // The clipboard's bind opens this too, by its name.
  const clipboard = `clipboard:${output}`;
  const [bar, setBar] = createSignal<any>(null);
  const [startButton, setStartButton] = createSignal<any>(null);
  const [quickButton, setQuickButton] = createSignal<any>(null);
  const [clockButton, setClockButton] = createSignal<any>(null);
  const [clipboardButton, setClipboardButton] = createSignal<any>(null);
  const out = () => activeDevice("outputs");
  return (
    <>
      <Surface
        output={output}
        anchor={["bottom", "left", "right"]}
        layer="top"
        reserve={height()}
        style={{ height: height(), flexDirection: "column" }}
      >
        <box
          ref={setBar}
          style={{
            material: "mica", height: height(), flexDirection: "row", alignItems: "center",
            padding: [0, 0, 0, 12],
          }}
        >
          {/* Room on the left, as much as on the right, so the apps are in the middle. */}
          <Show when={settings.alignment === "center"}>
            <box style={{ flexGrow: 1, flexBasis: 0 }} />
          </Show>
          <box style={{ flexDirection: "row", alignItems: "center", gap: 4 }}>
            <BarButton
              ref={setStartButton}
              open={popup.is(start)}
              onClick={() => popup.toggle(start)}
            >
              <image icon="ph-squares-four-symbolic" size={20} />
            </BarButton>
            {/* The overview, of every workspace. */}
            <OverviewButton
              output={output}
              gap={over(40)}
              align={settings.alignment === "left" ? "start" : "center"}
            />
            <TaskbarApps output={output} size={40} />
          </box>
          <box
            style={{
              flexGrow: 1, flexBasis: 0, flexDirection: "row", alignItems: "center",
              justifyContent: "end", gap: 4,
            }}
          >
            {/* Three areas, a line between each: the tray, quick things like the sound output
                switcher and the clipboard, and quick settings with the clock. */}
            <Tray output={output} side="top" gap={over(28)} />
            <Divider length={24} style={{ margin: [0, 4] }} />
            {/* While sound outputs are pinned, a click goes to the next, and a right click opens
                quick settings, where they're pinned. */}
            <OutputSwitcher style={SMALL} onMore={() => popup.open(quick)} />
            <box
              ref={setClipboardButton}
              style={[SMALL, { alignItems: "center", justifyContent: "center" }, popup.is(clipboard) && { background: "fg/0.1" }]}
              onClick={() => popup.toggle(clipboard)}
            >
              <image icon="edit-paste-symbolic" />
            </box>
            <Divider length={24} style={{ margin: [0, 4] }} />
            <BarButton
              ref={setQuickButton}
              open={popup.is(quick)}
              onClick={() => popup.toggle(quick)}
            >
              <image icon={networkIcon()} />
              <Show when={out()}>
                <image icon={volumeIcon(out())} />
              </Show>
              <Show when={battery()}>
                <image icon={battery()!.icon} />
              </Show>
            </BarButton>
            <BarButton
              ref={setClockButton}
              open={popup.is(calendar)}
              onClick={() => popup.toggle(calendar)}
            >
              <Clock lines={2} />
              <Show
                when={notifications.doNotDisturb()}
                fallback={
                  <Show when={notifications.unseen()}>
                    <box style={{ width: 8, height: 8, borderRadius: 4, background: "accent" }} />
                  </Show>
                }
              >
                <image icon="notifications-disabled-symbolic" />
              </Show>
            </BarButton>
            {/* The sliver at the end shows the desktop. */}
            <box
              style={{
                width: 12, height: height(), marginLeft: 4,
                ":hover": { background: "fg/0.1" },
              }}
              onClick={() => action("layout-action", ["show-desktop"])}
            >
              <box style={{ position: "absolute", left: 0, top: (height() - 24) / 2, width: 1, height: 24, background: "fg/0.2" }} />
            </box>
          </box>
        </box>
      </Surface>
      {/* Over the start button: from its left edge on a taskbar to the left, and around it on
          one in the middle. */}
      <StartMenu
        output={output}
        from={startButton()}
        align={settings.alignment === "left" ? "start" : "center"}
        gap={over(40)}
      />
      <Flyout output={output} popup={quick} from={quickButton()} side="top" align="end" gap={over(40)} keyboard>
        <QuickSettings output={output} onClose={() => popup.close(quick)} />
      </Flyout>
      <Flyout output={output} popup={clipboard} from={clipboardButton()} side="top" align="end" gap={over(28)} keyboard>
        <ClipboardHistory onPicked={() => popup.close(clipboard)} />
      </Flyout>
      <Flyout output={output} popup={calendar} from={clockButton()} side="top" align="end" gap={over(40)} keyboard>
        <box style={{ flexDirection: "column", gap: 16, width: 360 }}>
          <NotificationCenter onUsed={() => popup.close(calendar)} />
          <box style={{ height: 1, background: "fg/0.1" }} />
          <Month />
        </box>
      </Flyout>
      <Toasts
        output={output}
        from={bar()}
        side="top"
        held={popup.is(calendar) || focusedOutput() !== output}
      />
    </>
  );
}
