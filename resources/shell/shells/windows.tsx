// A shell laid out like Windows 11's: a taskbar along the bottom of each output, its apps in the
// middle after the start button and the overview's, and at its end, in three areas, the tray, the
// sound output switcher and the clipboard, and quick settings with the clock and the
// notifications, then a sliver that shows the desktop.
// Notifications come up over the taskbar's end. The taskbar is in the top layer, so fullscreen
// windows cover it.

import { Surface, action, focusedOutput, onAction, outputs, useModule, type Meta } from "bento";
import { createSignal, For, Show } from "solid-js";
import { Divider, Flyout, popup } from "bento:modules/ui";
import { ClipboardHistory } from "bento:modules/clipboard";
import { Clock, Month } from "bento:modules/clock";
import { Toasts, notifications } from "bento:modules/notifications";
import { NotificationCenter, QuickSettings } from "bento:modules/control-center";
import { networkIcon } from "bento:modules/network";
import { activeDevice, OutputSwitcher, VolumeIndicator, volumeIcon } from "bento:modules/sound";
import { battery } from "bento:modules/power";
import { TaskbarApps } from "bento:modules/taskbar";
import { StartMenu } from "bento:modules/start-menu";
import { Tray } from "bento:modules/tray";
import { WorkspaceEditor } from "bento:modules/workspace-editor";

export const meta = {
  settings: {
    height: {
      type: "number",
      default: 48,
      description: "How tall the taskbar is, in pixels.",
      page: "shell",
      group: "Taskbar",
    },
  },
} satisfies Meta;

const { settings } = useModule(import.meta, meta);

/** How tall the taskbar is, with room for its buttons. */
const height = () => Math.max(40, settings.height);

// The launcher's shortcut opens the focused output's start menu.
onAction("toggle-launcher", ({ output }) => {
  popup.toggle(`start:${output ?? focusedOutput() ?? outputs()[0]}`);
});

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
  // The clipboard's shortcut opens this too, by its name.
  const clipboard = `clipboard:${output}`;
  const [bar, setBar] = createSignal<any>(null);
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
          <box style={{ flexGrow: 1, flexBasis: 0 }} />
          <box style={{ flexDirection: "row", alignItems: "center", gap: 4 }}>
            <BarButton open={popup.is(start)} onClick={() => popup.toggle(start)}>
              <image icon="start-here-symbolic" size={22} style={{ color: "accent" }} />
            </BarButton>
            <BarButton onClick={() => action("toggle-overview")}>
              <image icon="view-grid-symbolic" size={20} />
            </BarButton>
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
            <Tray output={output} side="top" />
            <Divider length={24} style={{ margin: [0, 4] }} />
            {/* While sound outputs are pinned, a click goes to the next, and a right click opens
                quick settings, where they're pinned. */}
            <OutputSwitcher
              style={{ height: 40, minWidth: 40, borderRadius: 8, ":hover": { background: "fg/0.14" } }}
              onMore={() => popup.open(quick)}
            />
            <BarButton
              ref={setClipboardButton}
              open={popup.is(clipboard)}
              onClick={() => popup.toggle(clipboard)}
            >
              <image icon="edit-paste-symbolic" />
            </BarButton>
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
      <StartMenu output={output} bottom={height() + 12} />
      <Flyout output={output} popup={quick} from={quickButton()} side="top" align="end" keyboard>
        <QuickSettings output={output} onClose={() => popup.close(quick)} />
      </Flyout>
      <Flyout output={output} popup={clipboard} from={clipboardButton()} side="top" align="end" keyboard>
        <ClipboardHistory onPicked={() => popup.close(clipboard)} />
      </Flyout>
      <Flyout output={output} popup={calendar} from={clockButton()} side="top" align="end" keyboard>
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
