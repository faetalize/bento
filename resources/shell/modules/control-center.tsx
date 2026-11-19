// The control center, laid out like a phone's quick settings:
//
// - along the top, the battery, and buttons for a screenshot, a screen recording, the settings
//   and the power menu;
// - the volume, and the brightness of the output's own display;
// - tiles, two to a row, that switch Wi-Fi, Bluetooth, Do Not Disturb, the dark style, the power
//   mode and the microphone, each only when there's something behind it;
// - what's playing;
// - the notifications.
//
// What has more to it than a switch opens a page of its own in it, with a way back. The status
// island holds the status icons and opens into it, and into the clipboard's history.

import {
  action, after, command, config, data, focusedOutput, notify, onAction, outputs, run, useModule,
  useOutput, type JSX, type Meta,
} from "bento";
import { createEffect, createSignal, For, Match, Show, Switch } from "solid-js";
import {
  ACTIVE, Button, Divider, Heading, ICON_ROOM, Island, Note, Toggle, popup,
} from "bento:modules/ui";
import { Tray } from "bento:modules/tray";
import { ClipboardHistory } from "bento:modules/clipboard";
import { NotificationCard, notifications } from "bento:modules/notifications";
import {
  OutputSwitcher, SoundPage, VolumeSlider, activeDevice, devices, micIcon, toggleMute, volumeIcon,
} from "bento:modules/sound";
import { WifiPage, networkIcon, setWifi, wifiState } from "bento:modules/network";
import { BluetoothPage, bluetoothState, setBluetooth } from "bento:modules/bluetooth";
import { PowerPage, ProfilePage, battery, profileState } from "bento:modules/power";
import { BrightnessSlider, DisplaysPage, displayOf } from "bento:modules/brightness";
import { MediaCard } from "bento:modules/media";

const WIDTH = 360;
const GAP = 8;
// A tile is half a row.
const TILE = (WIDTH - GAP) / 2;
// A slider beside its arrow.
const SLIDER = WIDTH - 32 - GAP;
// How many notifications show before "Show More", and the most listed.
const NOTES = 3;
const MOST_NOTES = 20;

/** Runs what the settings set for the screenshot or the screen record button, once the control
 * center is out of the picture, and shows its notification when it's done, if it has one. */
function capture(which: "screenshot" | "record") {
  const line = settings[`${which}-command`].trim();
  const title = settings[`${which}-notification`].trim();
  const body = settings[`${which}-notification-body`];
  popup.close();
  if (!line && which === "record") {
    // Nothing records the screen until a command is set.
    action("spawn", ["bento-settings", "shell"]);
    return;
  }
  after(350, async () => {
    if (!line) {
      action("screenshot");
      return;
    }
    const icon = which === "screenshot" ? "camera-photo-symbolic" : "media-record-symbolic";
    const code = await run(line);
    if (title && code === 0) notify(title, body, icon);
  });
}

/** A round button with an icon. */
const Round = (props: { icon: string; onClick: () => void }) => (
  <Button
    icon={props.icon}
    onClick={props.onClick}
    style={{ width: 38, height: 38, borderRadius: 19, background: "fg/0.08" }}
  />
);

/** The arrow that opens a page. */
const More = (props: { onClick: () => void; size?: number }) => (
  <Button
    icon="go-next-symbolic"
    onClick={props.onClick}
    style={{ width: props.size ?? 32, height: props.size ?? 28, borderRadius: (props.size ?? 28) / 2 }}
  />
);

interface TileState {
  there: boolean;
  active: boolean;
  icon?: string;
  subtitle: string;
}

interface TileSpec {
  icon: string;
  title: string;
  onClick?: () => void;
  /** Opens what's behind it: its arrow, or without `onClick` the whole tile. */
  onMore?: () => void;
  state: () => TileState;
}

/** A tile: an icon, a title and a word on how it is. */
function Tile(props: { spec: TileSpec }) {
  const state = () => props.spec.state();
  const ink = () => (state().active ? "#ffffff" : "fg");
  return (
    <box
      style={{
        flexDirection: "row", width: TILE, height: 60, borderRadius: 18, padding: [0, 8, 0, 14],
        gap: 12, alignItems: "center", flexShrink: 0, cursor: "pointer",
        background: state().active ? ACTIVE : "fg/0.1",
        ":hover": state().active ? undefined : { background: "fg/0.14" },
        color: ink(),
      }}
      onClick={() => (props.spec.onClick ?? props.spec.onMore)?.()}
    >
      <image icon={state().icon ?? props.spec.icon} size={20} />
      <box style={{ flexDirection: "column", flexGrow: 1, flexShrink: 1, minWidth: 0 }}>
        <text style={{ fontWeight: 700, fontSize: 14, lineClamp: 1 }}>{props.spec.title}</text>
        <text
          style={{ fontSize: 12, lineClamp: 1, color: state().active ? "#ffffff" : "fg/0.7" }}
        >
          {state().subtitle}
        </text>
      </box>
      <Show when={props.spec.onClick && props.spec.onMore}>
        <More onClick={props.spec.onMore!} size={30} />
      </Show>
    </box>
  );
}

type Page = "sound" | "wifi" | "bluetooth" | "profile" | "power" | "displays";

const PAGES: Record<Page, string> = {
  sound: "Sound",
  wifi: "Wi-Fi",
  bluetooth: "Bluetooth",
  profile: "Power Mode",
  power: "Power",
  displays: "Displays",
};

/** The notifications, the newest few, and the rest behind "Show More", with a way to clear them
 * all. While it shows, they're looked at as they come. `onUsed` hears one's action done. */
export function NotificationCenter(props: { onUsed?: () => void }) {
  const [allNotes, setAllNotes] = createSignal(false);
  notifications.looked();
  createEffect(notifications.unseen, (unseen) => {
    if (unseen) notifications.looked();
  });
  const kept = () => notifications.kept().slice(0, MOST_NOTES);
  const shownNotes = () => (allNotes() ? kept() : kept().slice(0, NOTES));
  return (
    <box style={{ flexDirection: "column", gap: GAP }}>
      <box style={{ flexDirection: "row", alignItems: "center", gap: GAP, height: 26 }}>
        <box style={{ flexGrow: 1 }}>
          <Heading>Notifications</Heading>
        </box>
        <Show when={kept().length > 0}>
          <Button
            label="Clear All"
            style={{ height: 26, background: "fg/0.08" }}
            onClick={() => notifications.dismissAll()}
          />
        </Show>
      </box>
      <box style={{ flexDirection: "column", gap: 6, overflow: "hidden" }}>
        <For each={shownNotes()} fallback={<Note>No Notifications</Note>}>
          {(n) => <NotificationCard notification={n} onUsed={props.onUsed} />}
        </For>
      </box>
      <Show when={kept().length > shownNotes().length}>
        <Button
          label={`Show ${kept().length - shownNotes().length} More`}
          style={{ height: 30, background: "fg/0.08" }}
          onClick={() => setAllNotes(true)}
        />
      </Show>
    </box>
  );
}

/** Quick settings for output `output`: along the top, the battery, and buttons for a
 * screenshot, a screen recording, the settings and the power menu; the volume and the output's
 * brightness; the tiles; and what's playing, then what it holds. What has more to it opens a
 * page in its place. `onClose` closes what it's in. */
export function QuickSettings(props: { output: string; onClose: () => void; children?: JSX.Element }) {
  const [page, setPage] = createSignal<Page | null>(null);
  // A monitor's own buttons change its brightness too.
  command("brightness-refresh", {});

  const TILES: TileSpec[] = [
    {
      icon: "network-wireless-symbolic", title: "Wi-Fi",
      onClick: () => setWifi(!data("wifi")?.enabled),
      onMore: () => setPage("wifi"),
      state: wifiState,
    },
    {
      icon: "bluetooth-symbolic", title: "Bluetooth",
      onClick: () => setBluetooth(!data("bluetooth")?.powered),
      onMore: () => setPage("bluetooth"),
      state: bluetoothState,
    },
    {
      icon: "notifications-disabled-symbolic", title: "Do Not Disturb",
      onClick: () => notifications.setDoNotDisturb(!notifications.doNotDisturb()),
      state: () => ({
        there: true,
        active: notifications.doNotDisturb(),
        subtitle: notifications.doNotDisturb() ? "On" : "Off",
      }),
    },
    {
      icon: "weather-clear-night-symbolic", title: "Dark Style",
      onClick: () => command("theme", { dark: !data("theme")?.dark }),
      state: () => ({
        there: Boolean(data("theme")),
        active: Boolean(data("theme")?.dark),
        subtitle: data("theme")?.dark ? "On" : "Off",
      }),
    },
    {
      icon: "power-profile-balanced-symbolic", title: "Power Mode",
      onMore: () => setPage("profile"),
      state: profileState,
    },
    {
      icon: "microphone-sensitivity-high-symbolic", title: "Microphone",
      onClick: () => toggleMute("inputs"),
      // The microphone's devices and level are on the sound page, behind the volume.
      state: () => {
        const mic = activeDevice("inputs");
        return {
          there: devices().inputs.length > 0,
          active: mic ? !mic.muted : false,
          icon: micIcon(mic ?? { muted: true }),
          subtitle: !mic ? "None" : mic.muted ? "Muted" : mic.label,
        };
      },
    },
  ];
  // The tiles there's something behind, two to a row.
  const rows = () => {
    const shown = TILES.filter((tile) => tile.state().there);
    const out: (TileSpec | null)[][] = [];
    for (let i = 0; i < shown.length; i += 2) out.push([shown[i], shown[i + 1] ?? null]);
    return out;
  };

  const display = () => displayOf(props.output);

  const main = () => (
    <box style={{ flexDirection: "column", gap: 14 }} enter={{ opacity: 0, blur: 6 }}>
      <box style={{ flexDirection: "row", alignItems: "center", gap: GAP }}>
        <box style={{ flexGrow: 1 }}>
          <text style={{ color: "fg/0.7", fontWeight: 600, fontVariantNumeric: "tabular-nums" }}>
            {battery()?.words ?? ""}
          </text>
        </box>
        <Round icon="camera-photo-symbolic" onClick={() => capture("screenshot")} />
        <Round icon="media-record-symbolic" onClick={() => capture("record")} />
        <Round
          icon="emblem-system-symbolic"
          onClick={() => {
            popup.close();
            action("spawn", ["bento-settings"]);
          }}
        />
        <Round icon="system-shutdown-symbolic" onClick={() => setPage("power")} />
      </box>
      <box style={{ flexDirection: "column", gap: GAP }}>
        <box style={{ flexDirection: "row", alignItems: "center", gap: GAP }}>
          <VolumeSlider width={SLIDER} />
          <More onClick={() => setPage("sound")} />
        </box>
        <Show when={display()}>
          <box style={{ flexDirection: "row", alignItems: "center", gap: GAP }}>
            <BrightnessSlider display={display()!} width={SLIDER} />
            <More onClick={() => setPage("displays")} />
          </box>
        </Show>
      </box>
      <box style={{ flexDirection: "column", gap: GAP }}>
        <For each={rows()}>
          {(row) => (
            <box style={{ flexDirection: "row", gap: GAP }}>
              <Tile spec={row[0]!} />
              <Show when={row[1]} fallback={<box style={{ width: TILE }} />}>
                <Tile spec={row[1]!} />
              </Show>
            </box>
          )}
        </For>
      </box>
      <MediaCard />
      {props.children}
    </box>
  );

  const pageBody = (which: Page): JSX.Element => (
    <Switch>
      <Match when={which === "sound"}>
        <SoundPage width={WIDTH} />
      </Match>
      <Match when={which === "wifi"}>
        <WifiPage />
      </Match>
      <Match when={which === "bluetooth"}>
        <BluetoothPage />
      </Match>
      <Match when={which === "profile"}>
        <ProfilePage />
      </Match>
      <Match when={which === "power"}>
        <PowerPage onDone={props.onClose} />
      </Match>
      <Match when={which === "displays"}>
        <DisplaysPage width={WIDTH} />
      </Match>
    </Switch>
  );
  const switchOf = (which: Page) =>
    which === "wifi" ? { on: Boolean(data("wifi")?.enabled), set: setWifi }
    : which === "bluetooth" ? { on: Boolean(data("bluetooth")?.powered), set: setBluetooth }
    : null;

  return (
    <box style={{ flexDirection: "column", width: WIDTH }}>
      <Show when={page()} keyed fallback={main()}>
        {(which: Page) => (
          <box style={{ flexDirection: "column", gap: 10 }} enter={{ opacity: 0, blur: 6 }}>
            <box style={{ flexDirection: "row", alignItems: "center", gap: GAP }}>
              <Button
                icon="go-previous-symbolic"
                style={{ width: 32, height: 32, borderRadius: 16 }}
                onClick={() => setPage(null)}
              />
              <box style={{ flexGrow: 1 }}>
                <text style={{ fontWeight: 700, fontSize: 16 }}>{PAGES[which]}</text>
              </box>
              <Show when={switchOf(which)}>
                <Toggle on={switchOf(which)!.on} onChange={(on) => switchOf(which)!.set(on)} />
              </Show>
            </box>
            {pageBody(which)}
          </box>
        )}
      </Show>
    </box>
  );
}

/** The control center of bento's own shell: quick settings, with the notifications under
 * them. */
export function ControlCenter(props: { output: string; onClose: () => void }) {
  return (
    <QuickSettings output={props.output} onClose={props.onClose}>
      <NotificationCenter onUsed={props.onClose} />
    </QuickSettings>
  );
}

export const meta = {
  settings: {
    "screenshot-command": {
      type: "string",
      default: "",
      description: "What the screenshot button runs, with sh. Without a command, it takes bento's own screenshot.",
      page: "shell",
      group: "Screenshot Button",
    },
    "screenshot-notification": {
      type: "string",
      default: "",
      description: "The title of a notification that shows once the command is done; none shows without one.",
      page: "shell",
      group: "Screenshot Button",
    },
    "screenshot-notification-body": {
      type: "string",
      default: "",
      description: "The notification's text.",
      page: "shell",
      group: "Screenshot Button",
    },
    "record-command": {
      type: "string",
      default: "",
      description: "What the screen record button runs, with sh, and again to stop a recording if that's how the program works.",
      page: "shell",
      group: "Screen Record Button",
    },
    "record-notification": {
      type: "string",
      default: "",
      description: "The title of a notification that shows once the command is done; none shows without one.",
      page: "shell",
      group: "Screen Record Button",
    },
    "record-notification-body": {
      type: "string",
      default: "",
      description: "The notification's text.",
      page: "shell",
      group: "Screen Record Button",
    },
  },
  actions: {
    "toggle-clipboard": {
      description: "Show or hide the clipboard's history.",
    },
  },
} satisfies Meta;

const { settings } = useModule(import.meta, meta);

// The clipboard's shortcut opens the history in the focused output's status island.
onAction("toggle-clipboard", ({ output }) => {
  popup.toggle(`clipboard:${output ?? focusedOutput() ?? outputs()[0]}`);
});

/** The status island of the output it's on: the tray, the sound output switcher and the
 * clipboard's button, and the status icons, which open the control center. `onOpen` hears whether it's open into
 * anything. */
export function StatusIsland(props: { ref?: (node: any) => void }) {
  const output = useOutput();
  const control = `control:${output}`;
  const clipboard = `clipboard:${output}`;
  const open = () =>
    popup.is(control) ? "control" : popup.is(clipboard) ? "clipboard" : null;
  // The island's own row gives way to whatever it opens into.
  const away = () => open() !== null;
  const fade = () => ({ opacity: away() ? 0 : 1, blur: away() ? 4 : 0 });
  const out = () => activeDevice("outputs");
  return (
    <Island
      ref={props.ref}
      hold="end"
      gap={8}
      padding={[0, 14 - ICON_ROOM, 0, 6]}
      open={open()}
      style={{ cursor: "pointer" }}
      onClick={() => popup.toggle(control)}
      panels={{
        control: () => <ControlCenter output={output} onClose={() => popup.close()} />,
        clipboard: () => <ClipboardHistory onPicked={() => popup.close()} />,
      }}
    >
      {/* Three areas, a line between each: the tray, quick things like the sound output switcher
          and the clipboard, and the status, which opens the control center. */}
      <box style={[{ flexDirection: "row", alignItems: "center", gap: 8 }, fade()]}>
        <Tray output={output} side="bottom" />
        <Divider style={{ margin: [0, -4] }} />
        <OutputSwitcher onMore={() => popup.open(control)} />
        <Button icon="edit-paste-symbolic" onClick={() => popup.toggle(clipboard)} />
        <Divider style={{ margin: [0, -4] }} />
        <box style={{ flexDirection: "row", alignItems: "center", gap: 8, padding: [0, ICON_ROOM] }}>
          <Show when={notifications.doNotDisturb()}>
            <image icon="notifications-disabled-symbolic" />
          </Show>
          <image icon={networkIcon()} />
          <Show when={out()}>
            <image icon={volumeIcon(out())} />
          </Show>
          <Show when={battery()}>
            <image icon={battery()!.icon} />
            <text style={{ fontWeight: 600, fontVariantNumeric: "tabular-nums" }}>
              {battery()!.percent}
            </text>
          </Show>
          {/* A dot while there are notifications that haven't been looked at. */}
          <Show when={notifications.unseen() && !away()}>
            <box style={{ width: 6, height: 6, borderRadius: 3, background: "accent" }} />
          </Show>
        </box>
      </box>
    </Island>
  );
}
