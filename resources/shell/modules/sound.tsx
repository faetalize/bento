// Sound: the volume and the devices, the apps playing and recording on them, the outputs pinned
// for the switcher, and the on-screen indicator of a volume changed elsewhere, as by a key or a
// headset's knob.

import {
  Surface, after, bentoStyle, command, data, focusedOutput, notify, onAction, outputs, stored,
  useModule, type AudioStream, type Meta, type PointerEvent, type SceneNode,
} from "bento";
import { createEffect, createMemo, createSignal, For, onCleanup, Show } from "solid-js";
import { Button, Line, Note, Slider, grows, growsWith } from "bento:modules/ui";
import { openMenu, type MenuItem } from "bento:modules/menu";
import { appById, appIcon } from "bento:modules/apps";

export type Kind = "outputs" | "inputs";

export interface Device {
  name: string;
  label: string;
  /** A short name, like "Line Out" or the headset's own; a data program from before these says
   * only `label`. */
  title?: string;
  detail?: string;
  virtual?: boolean;
  icon: string;
  active: boolean;
  volume: number;
  muted: boolean;
}

/** An app's stream on a device, as the data program says it. */
export type Stream = AudioStream;

export const devices = (): { outputs: Device[]; inputs: Device[] } =>
  data("audio-devices") ?? { outputs: [], inputs: [] };
/** The apps' streams on the outputs or the inputs. */
export const streams = (kind: Kind): Stream[] =>
  (kind === "outputs" ? data("audio-devices")?.playing : data("audio-devices")?.recording) ?? [];
/** Whether apps can be made to follow the default or kept where they are. */
const canKeep = () => Boolean(data("audio-devices")?.canKeep);
const titleOf = (device: Device) => device.title || device.label;
export const activeDevice = (kind: Kind) => devices()[kind].find((device) => device.active);

export const volumeIcon = (device?: { volume: number; muted: boolean }) =>
  !device || device.muted || device.volume <= 0 ? "audio-volume-muted-symbolic"
  : device.volume < 0.34 ? "audio-volume-low-symbolic"
  : device.volume < 0.67 ? "audio-volume-medium-symbolic"
  : "audio-volume-high-symbolic";

export const micIcon = (device?: { muted: boolean }) =>
  !device || device.muted
    ? "microphone-sensitivity-muted-symbolic"
    : "microphone-sensitivity-high-symbolic";

const one = (kind: Kind) => kind.slice(0, -1) as "output" | "input";

// Until when changes are the shell's own, from a slider that shows them already.
let quietUntil = 0;

/** Says the shell itself is changing the volume, which needs no indicator. */
export function quiet() {
  quietUntil = Date.now() + 1000;
}

export function setVolume(kind: Kind, value: number) {
  const device = activeDevice(kind);
  quiet();
  if (device) command("device-volume", { kind: one(kind), name: device.name, value });
}

export function toggleMute(kind: Kind) {
  const device = activeDevice(kind);
  quiet();
  if (device) command("device-mute", { kind: one(kind), name: device.name, value: !device.muted });
}

// ------------------------------------------------------------------------------------------------
// Pinned outputs, and the switcher that goes around them.
// ------------------------------------------------------------------------------------------------

export const meta = {
  settings: {
    pinned: {
      type: "string",
      list: true,
      default: [] as string[],
      description:
        "The sound outputs that the output switcher and the Switch Sound Output bind go " +
        "through, by their name in the sound server.",
    },
  },
  actions: {
    "switch-audio-output": {
      description: "Switch the sound to the next pinned output.",
    },
  },
} satisfies Meta;

export const { settings, setSetting } = useModule(import.meta, meta);

/** The sound outputs pinned for the switcher, by name, in the order they were pinned. */
export const pinned = () => settings.pinned;

export function setPinned(name: string, pin: boolean) {
  const list = pinned().filter((other) => other !== name);
  if (pin) list.push(name);
  setSetting("pinned", list);
}

// Pins kept as the shell's state before they were a setting move to the settings.
{
  const [old, setOld] = stored<string[]>("pinnedOutputs", []);
  if (old().length > 0) {
    if (pinned().length === 0) setSetting("pinned", old());
    setOld([]);
  }
}

/** Makes the next pinned output the default one, going around, past the ones that aren't
 * connected, and says which it is. */
export function switchOutput() {
  const list = devices().outputs;
  const connected = pinned().filter((name) => list.some((output) => output.name === name));
  const say = (summary: string, body: string, icon = "") =>
    notify(summary, body, icon, { transient: true });
  if (pinned().length === 0) {
    say("Sound Output", "No outputs are pinned. Pin some in the control center.");
    return;
  }
  if (connected.length === 0) {
    say("Sound Output", "None of the pinned outputs are connected.");
    return;
  }
  const current = list.find((output) => output.active)?.name;
  const next = connected[(connected.indexOf(current as string) + 1) % connected.length];
  const device = list.find((output) => output.name === next)!;
  command("device", { kind: "output", name: next });
  say("Sound Output Switched", titleOf(device), device.icon);
}
onAction("switch-audio-output", switchOutput);

/** The switcher's button: the output that plays, which a click switches from, and a right click
 * opens `onMore`. It shows while outputs are pinned. */
export function OutputSwitcher(props: { onMore?: () => void; style?: any; size?: number }) {
  return (
    <Show when={pinned().length > 0}>
      <Button
        icon={activeDevice("outputs")?.icon ?? "audio-speakers-symbolic"}
        size={props.size}
        style={props.style}
        onClick={switchOutput}
        onRightClick={props.onMore}
      />
    </Show>
  );
}

// ------------------------------------------------------------------------------------------------
// Sliders and the sound page.
// ------------------------------------------------------------------------------------------------

/** The active device's volume, as a slider, its icon muting it. */
export function VolumeSlider(props: { kind?: Kind; width?: number; onChange?: () => void }) {
  const kind = () => props.kind ?? "outputs";
  const device = () => activeDevice(kind());
  return (
    <Slider
      width={props.width}
      value={Math.min(device()?.volume ?? 0, 1)}
      icon={kind() === "outputs" ? volumeIcon(device()) : micIcon(device())}
      onChange={(value) => {
        setVolume(kind(), value);
        props.onChange?.();
      }}
      onIcon={() => {
        toggleMute(kind());
        props.onChange?.();
      }}
    />
  );
}

/** An app on a device: its streams there, as one. */
interface App {
  key: string;
  name: string;
  icon: string;
  title: string | null;
  device: string;
  volume: number;
  muted: boolean;
  paused: boolean;
  kept: boolean;
  own: boolean;
  fixed: boolean;
  ids: number[];
}

/** The apps on the outputs or the inputs, each app's streams on one device together, as an app
 * that plays several, like a browser, is moved and remembered as one. */
function apps(kind: Kind): App[] {
  const byKey = new Map<string, App>();
  for (const stream of streams(kind)) {
    const key = `${stream.app}\n${stream.device}`;
    const app = byKey.get(key);
    if (app) {
      app.ids.push(stream.id);
      app.volume = Math.max(app.volume, stream.volume);
      app.muted &&= stream.muted;
      app.paused &&= stream.paused;
      app.kept ||= stream.kept;
      app.own ||= stream.own;
      app.fixed ||= stream.fixed;
      app.title ??= stream.title;
      continue;
    }
    const entry = appById(stream.binary ?? "") ?? appById(stream.app.toLowerCase());
    byKey.set(key, {
      key,
      name: entry?.name ?? stream.app,
      icon: stream.icon ?? appIcon(entry),
      title: stream.title,
      device: stream.device,
      volume: stream.volume,
      muted: stream.muted,
      paused: stream.paused,
      kept: stream.kept,
      own: stream.own,
      fixed: stream.fixed,
      ids: [stream.id],
    });
  }
  return [...byKey.values()];
}

const each = (app: App, run: (id: number) => void) => app.ids.forEach(run);

/** Puts `app` on the device named `to`: the default's means following the default, where it
 * can be told to; another, being kept there. */
function moveApp(kind: Kind, app: App, to: string) {
  const isDefault = devices()[kind].find((device) => device.active)?.name === to;
  if (isDefault && canKeep() && !app.own) {
    if (app.kept || app.device !== to) each(app, (id) => command("stream-follow", { kind: one(kind), id }));
    return;
  }
  if (app.device !== to) each(app, (id) => command("stream-move", { kind: one(kind), id, device: to }));
}

function appMenu(kind: Kind, app: App, event: PointerEvent) {
  const here = devices()[kind].find((device) => device.name === app.device);
  const thing = kind === "outputs" ? "output" : "microphone";
  const items: MenuItem[] = app.fixed
    ? [{ label: `${app.name} can't be moved`, disabled: true }]
    : app.own
      ? [{ label: `${app.name} picks its own ${thing}`, disabled: true }]
      : !canKeep()
        ? []
        : app.kept
          ? [{ label: "Follow Default", run: () => each(app, (id) => command("stream-follow", { kind: one(kind), id })) }]
          : [{ label: `Keep on ${here ? titleOf(here) : "This Device"}`, run: () => each(app, (id) => command("stream-keep", { kind: one(kind), id })) }];
  if (items.length > 0) openMenu({ output: event.output, x: event.x, y: event.y }, items);
}

function deviceMenu(kind: Kind, device: Device, event: PointerEvent) {
  const items: MenuItem[] = [
    { label: "Make Default", disabled: device.active, run: () => command("device", { kind: one(kind), name: device.name }) },
  ];
  if (kind === "outputs") {
    const pin = pinned().includes(device.name);
    items.push({ label: pin ? "Unpin from Switcher" : "Pin to Switcher", run: () => setPinned(device.name, !pin) });
  }
  openMenu({ output: event.output, x: event.x, y: event.y }, items);
}

// How far a press moves before it's a drag.
const SLOP = 6;
const CARD = { borderRadius: 14, padding: [2, 0, 6, 0] } as const;

/** The sound devices, and the apps on them. A device that's the default, or that something plays
 * on, is a card with its volume and its apps; the rest are chips under the cards, and virtual
 * ones nothing uses fold away. Apps are dragged between devices, and a switch at the top picks
 * the outputs or the inputs. */
export function SoundPage(props: { width: number }) {
  const [kind, setKind] = createSignal<Kind>("outputs");
  const [showOther, setShowOther] = createSignal(false);
  // Where apps can be dropped: each device's card or chip, by its name.
  const drops = new Map<string, SceneNode>();
  let root: SceneNode | undefined;
  let pressed: { app: App; x: number; y: number } | null = null;
  const [dragged, setDragged] = createSignal<{ app: App; x: number; y: number } | null>(null);
  let dropped = false;
  const over = () => {
    const drag = dragged();
    if (!drag) return null;
    for (const [name, node] of drops) {
      const r = node.layout();
      if (r && drag.x >= r.x && drag.x <= r.x + r.width && drag.y >= r.y && drag.y <= r.y + r.height) return name;
    }
    return null;
  };
  const release = () => {
    pressed = null;
    const drag = dragged();
    const to = over();
    setDragged(null);
    if (!drag) return;
    dropped = true;
    after(0, () => (dropped = false));
    if (to) moveApp(kind(), drag.app, to);
  };
  const dropTarget = (name: string) => (node: SceneNode) => {
    drops.set(name, node);
    onCleanup(() => drops.get(name) === node && drops.delete(name));
  };

  // Cards, chips and apps go by their names and keys, so each stays the same node as what it
  // shows changes: its hover, and a drag that started on it, live through every update.
  const list = () => devices()[kind()];
  const allApps = createMemo(() => apps(kind()));
  const appsOn = (name: string) => allApps().filter((app) => app.device === name).map((app) => app.key);
  const appOf = (key: string) => allApps().find((app) => app.key === key);
  const deviceOf = (name: string) => list().find((device) => device.name === name);
  const isCard = (device: Device) => device.active || appsOn(device.name).length > 0;
  const names = (devices: Device[]) => devices.map((device) => device.name);
  const cards = () => {
    const all = list().filter(isCard);
    return names([...all.filter((device) => device.active), ...all.filter((device) => !device.active)]);
  };
  const chips = () => names(list().filter((device) => !isCard(device) && !device.virtual));
  const other = () => names(list().filter((device) => !isCard(device) && device.virtual));

  const AppRow = (key: string) => (
    <Show when={appOf(key)}>{(app) => AppRowOf(app)}</Show>
  );
  const AppRowOf = (app: () => App) => {
    const [hover, setHover] = createSignal(false);
    const setVolume = (value: number) => {
      quiet();
      each(app(), (id) => command("stream-volume", { kind: one(kind()), id, value }));
    };
    return (
      <box
        style={{
          flexDirection: "row", alignItems: "center", gap: 10, padding: [3, 10], borderRadius: 10,
          cursor: app().fixed ? "default" : "grab",
          opacity: dragged()?.app.key === app().key ? 0.4 : app().paused ? 0.7 : 1,
          ":hover": { background: "fg/0.04" },
        }}
        onHover={({ inside }) => setHover(inside)}
        onPress={(event) => {
          if (event.button === 1 && !app().fixed) pressed = { app: app(), x: event.x, y: event.y };
        }}
        onDrag={(event) => {
          if (!pressed) return;
          const far = Math.hypot(event.x - pressed.x, event.y - pressed.y) >= SLOP;
          if (dragged() || far) setDragged({ app: app(), x: event.x, y: event.y });
        }}
        onRelease={release}
        onRightClick={(event) => appMenu(kind(), app(), event)}
      >
        <box
          style={{
            width: 22, height: 22, alignItems: "center", justifyContent: "center", flexShrink: 0,
            opacity: app().muted ? 0.35 : 1, cursor: "pointer",
          }}
          onClick={() =>
            !dropped && each(app(), (id) => command("stream-mute", { kind: one(kind()), id, value: !app().muted }))
          }
        >
          <image icon={app().icon} size={20} />
        </box>
        <box
          style={{
            flexDirection: "row", alignItems: "center", gap: 4, width: 110, flexShrink: 0,
            opacity: app().muted ? 0.5 : 1,
          }}
        >
          <text style={{ fontSize: 13, lineClamp: 1, flexShrink: 1 }}>
            {hover() && app().title ? `${app().name} · ${app().title}` : app().name}
          </text>
          <Show when={app().fixed || (app().kept && !app().own)}>
            <image
              icon={app().fixed ? "ph-lock-simple-symbolic" : "ph-map-pin-symbolic"}
              size={10}
              style={{ color: app().fixed ? "fg/0.5" : "accent", flexShrink: 0 }}
            />
          </Show>
        </box>
        <Line
          value={Math.min(app().volume, 1)}
          onPick={setVolume}
          onWheel={(up) => setVolume(Math.min(Math.max(app().volume + (up ? 0.05 : -0.05), 0), 1))}
        />
      </box>
    );
  };

  /** The pin for the switcher, while `shown` or pinned. */
  const PinButton = (device: () => Device, shown: () => boolean) => {
    const pin = () => pinned().includes(device().name);
    return (
      <Show when={kind() === "outputs" && (shown() || pin())}>
        <Button
          icon="view-pin-symbolic"
          size={12}
          active={pin()}
          style={{ width: 26, height: 26, borderRadius: 8, flexShrink: 0 }}
          onClick={() => setPinned(device().name, !pin())}
        />
      </Show>
    );
  };

  const DeviceCard = (name: string) => (
    <Show when={deviceOf(name)}>{(device) => DeviceCardOf(device)}</Show>
  );
  const DeviceCardOf = (device: () => Device) => {
    const [hover, setHover] = createSignal(false);
    const lit = () => over() === device().name;
    const setThis = (value: number) => {
      quiet();
      command("device-volume", { kind: one(kind()), name: device().name, value });
    };
    return (
      <box
        ref={dropTarget(device().name)}
        style={{
          ...CARD, flexDirection: "column",
          background: lit() ? "accent/0.2" : device().active ? "fg/0.1" : "fg/0.05",
          borderWidth: lit() ? 2 : 0, borderColor: "accent",
        }}
        onHover={({ inside }) => setHover(inside)}
      >
        <box
          style={{
            flexDirection: "row", alignItems: "center", gap: 10, padding: [8, 10], borderRadius: 12,
            cursor: "pointer", ":hover": { background: "fg/0.04" },
          }}
          onClick={() => !device().active && command("device", { kind: one(kind()), name: device().name })}
          onRightClick={(event) => deviceMenu(kind(), device(), event)}
        >
          <image
            icon={device().icon}
            size={18}
            style={{ color: device().active ? "accent" : "fg", flexShrink: 0 }}
          />
          <box style={{ flexDirection: "column", flexGrow: 1, flexShrink: 1, minWidth: 0 }}>
            <text style={{ fontWeight: device().active ? 700 : 400, lineClamp: 1 }}>
              {titleOf(device())}
            </text>
            <Show
              when={device().active}
              fallback={
                <box style={{ flexDirection: "column", marginTop: 2 }}>
                  <Line
                    value={Math.min(device().volume, 1)}
                    onPick={setThis}
                    onWheel={(up) => setThis(Math.min(Math.max(device().volume + (up ? 0.05 : -0.05), 0), 1))}
                  />
                </box>
              }
            >
              <Show when={device().detail}>
                <text style={{ fontSize: 11, color: "fg/0.55", lineClamp: 1 }}>{device().detail!}</text>
              </Show>
            </Show>
          </box>
          {PinButton(device, hover)}
        </box>
        <Show when={device().active}>
          <box style={{ padding: [0, 10, 10, 10] }}>
            <VolumeSlider kind={kind()} width={props.width - 20} />
          </box>
        </Show>
        <For each={appsOn(device().name)}>{(key) => AppRow(key)}</For>
      </box>
    );
  };

  const Chip = (name: string) => (
    <Show when={deviceOf(name)}>{(device) => ChipOf(device)}</Show>
  );
  const ChipOf = (device: () => Device) => {
    const [hover, setHover] = createSignal(false);
    const lit = () => over() === device().name;
    const pin = () => kind() === "outputs" && (hover() || pinned().includes(device().name));
    return (
      <box
        ref={dropTarget(device().name)}
        style={{
          flexDirection: "row", alignItems: "center", gap: 6, height: 32,
          padding: [0, pin() ? 4 : 12, 0, 10], borderRadius: 10, cursor: "pointer",
          background: lit() ? "accent/0.2" : hover() ? "fg/0.1" : "fg/0.06",
          borderWidth: lit() ? 2 : 0, borderColor: "accent",
        }}
        onHover={({ inside }) => setHover(inside)}
        onClick={() => command("device", { kind: one(kind()), name: device().name })}
        onRightClick={(event) => deviceMenu(kind(), device(), event)}
      >
        <image icon={device().icon} size={14} />
        <text style={{ fontSize: 12, lineClamp: 1 }}>{titleOf(device())}</text>
        {PinButton(device, hover)}
      </box>
    );
  };

  const ghost = () => {
    const drag = dragged();
    const rect = root?.layout();
    if (!drag || !rect) return null;
    return { left: drag.x - rect.x - 16, top: drag.y - rect.y - 16 };
  };

  return (
    <box ref={root} style={{ flexDirection: "column", gap: 8 }}>
      <box style={{ flexDirection: "row", padding: 3, borderRadius: 12, background: "fg/0.08" }}>
        <For each={["outputs", "inputs"] as Kind[]}>
          {(which) => (
            <box
              style={{
                flexGrow: 1, flexBasis: 0, height: 30, borderRadius: 9, alignItems: "center",
                justifyContent: "center", cursor: "pointer",
                background: kind() === which ? "accent/0.85" : "#00000000",
              }}
              onClick={() => setKind(which)}
            >
              <text style={{ fontSize: 13, fontWeight: kind() === which ? 700 : 400, color: kind() === which ? "#ffffff" : "fg/0.75" }}>
                {which === "outputs" ? "Output" : "Input"}
              </text>
            </box>
          )}
        </For>
      </box>
      <Show when={list().length > 0} fallback={<Note>{kind() === "outputs" ? "No Outputs" : "No Inputs"}</Note>}>
        <For each={cards()}>{(name) => DeviceCard(name)}</For>
        <Show when={chips().length > 0}>
          <box style={{ flexDirection: "row", flexWrap: "wrap", gap: 6 }}>
            <For each={chips()}>{(name) => Chip(name)}</For>
          </box>
        </Show>
        <Show when={other().length > 0}>
          <box
            style={{ flexDirection: "row", alignItems: "center", gap: 6, padding: [4, 6], borderRadius: 8, cursor: "pointer", ":hover": { background: "fg/0.05" } }}
            onClick={() => setShowOther(!showOther())}
          >
            <image icon={showOther() ? "pan-down-symbolic" : "pan-end-symbolic"} size={12} style={{ color: "fg/0.55" }} />
            <text style={{ fontSize: 12, color: "fg/0.55" }}>{`Other Devices (${other().length})`}</text>
          </box>
          <Show when={showOther()}>
            <box style={{ flexDirection: "row", flexWrap: "wrap", gap: 6 }}>
              <For each={other()}>{(name) => Chip(name)}</For>
            </box>
          </Show>
        </Show>
      </Show>
      <Show when={ghost()}>
        <box
          style={{
            position: "absolute", left: ghost()!.left, top: ghost()!.top, flexDirection: "row",
            alignItems: "center", gap: 8, padding: [6, 10], borderRadius: 10, background: "window/0.95",
            borderWidth: 1, borderColor: "fg/0.12", transition: { frame: 0.04, translate: 0.04 },
          }}
        >
          <image icon={dragged()!.app.icon} size={18} />
          <text style={{ fontSize: 13 }}>{dragged()!.app.name}</text>
        </box>
      </Show>
    </box>
  );
}

// ------------------------------------------------------------------------------------------------
// The on-screen indicator.
// ------------------------------------------------------------------------------------------------

// How long the indicator stays after the last change, in milliseconds.
const STAYS = 1500;
// How far above the bottom edge it shows, past bento's gaps.
const ABOVE = 48;
const HEIGHT = 44;

/** The volume's level at the bottom middle of the focused output, for a moment after it
 * changes elsewhere. It grows out of nothing and shrinks back into nothing. */
export function VolumeIndicator() {
  // Which list it shows a device of, while it shows.
  const [showing, setShowing] = createSignal<Kind | null>(null);
  let stop: (() => void) | null = null;
  const stay = () => {
    stop?.();
    stop = after(STAYS, () => setShowing(null));
  };

  // What the active devices were, to tell a change of volume from a change of device.
  const sound = (list: Device[]) => {
    const device = list.find((one) => one.active);
    return device && { name: device.name, volume: device.volume, muted: device.muted };
  };
  const heard = createMemo(() => {
    const all = data("audio-devices");
    return all ? { outputs: sound(all.outputs), inputs: sound(all.inputs) } : null;
  });
  createEffect(heard, (now, before) => {
    if (!now || !before || Date.now() < quietUntil) return;
    const changed = (["outputs", "inputs"] as Kind[]).find((kind) => {
      const [was, is] = [before[kind], now[kind]];
      return was && is && was.name === is.name
        && (was.volume !== is.volume || was.muted !== is.muted);
    });
    if (!changed) return;
    setShowing(changed);
    stay();
  });

  const output = () => focusedOutput() ?? outputs()[0];
  return (
    <Show when={showing() && output()}>
      <Surface
        output={output()}
        layer="overlay"
        z={-1}
        anchor={["bottom"]}
        style={{ marginBottom: bentoStyle().gaps + ABOVE }}
      >
        <box
          style={{
            material: "glass", borderRadius: HEIGHT / 2, height: HEIGHT, padding: [0, 14, 0, 8],
            flexDirection: "row", alignItems: "center", overflow: "hidden",
          }}
          {...grows(HEIGHT / 2, HEIGHT)}
        >
          <box style={{ flexDirection: "row", alignItems: "center", gap: 10 }} {...growsWith}>
            <VolumeSlider kind={showing()!} width={220} onChange={stay} />
            <box style={{ width: 40, flexDirection: "row", justifyContent: "end" }}>
              <text
                style={{
                  fontWeight: 600, fontVariantNumeric: "tabular-nums",
                  color: activeDevice(showing()!)?.muted ? "fg/0.5" : "fg",
                }}
              >
                {`${Math.round((activeDevice(showing()!)?.volume ?? 0) * 100)}%`}
              </text>
            </box>
          </box>
        </box>
      </Surface>
    </Show>
  );
}
