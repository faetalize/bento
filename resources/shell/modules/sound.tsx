// Sound: the volume and the devices, the outputs pinned for the switcher, and the on-screen
// indicator of a volume changed elsewhere, as by a key or a headset's knob.

import {
  Surface, after, bentoStyle, command, data, focusedOutput, notify, onAction, outputs, stored,
  useModule, type Meta,
} from "bento";
import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import { Button, Heading, Note, Row, Slider, grows, growsWith } from "bento:modules/ui";

export type Kind = "outputs" | "inputs";

export interface Device {
  name: string;
  label: string;
  icon: string;
  active: boolean;
  volume: number;
  muted: boolean;
}

export const devices = (): { outputs: Device[]; inputs: Device[] } =>
  data("audio-devices") ?? { outputs: [], inputs: [] };
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
        "The sound outputs that the output switcher and the Switch Sound Output shortcut go " +
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
  say("Sound Output Switched", device.label, device.icon);
}
onAction("switch-audio-output", switchOutput);

/** The switcher's button: the output that plays, which a click switches from, and a right click
 * opens `onMore`. It shows while outputs are pinned. */
export function OutputSwitcher(props: { onMore?: () => void }) {
  return (
    <Show when={pinned().length > 0}>
      <Button
        icon={activeDevice("outputs")?.icon ?? "audio-speakers-symbolic"}
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

/** The sound devices, outputs pinned or not, the microphone's level, and the inputs, to pick
 * from. */
export function SoundPage(props: { width: number }) {
  const deviceRow = (kind: Kind, device: Device) => (
    <Row
      icon={device.icon}
      title={device.label}
      lit={device.active}
      onClick={() => command("device", { kind: one(kind), name: device.name })}
    >
      <Show when={kind === "outputs"}>
        <Button
          icon="view-pin-symbolic"
          active={pinned().includes(device.name)}
          style={{ height: 26, borderRadius: 13 }}
          onClick={() => setPinned(device.name, !pinned().includes(device.name))}
        />
      </Show>
    </Row>
  );
  return (
    <box style={{ flexDirection: "column", gap: 2 }}>
      <Heading>Output</Heading>
      <For each={devices().outputs} fallback={<Note>No Outputs</Note>}>
        {(device) => deviceRow("outputs", device)}
      </For>
      <box style={{ height: 10 }} />
      <Heading>Input</Heading>
      <box style={{ height: 4 }} />
      <VolumeSlider kind="inputs" width={props.width} />
      <box style={{ height: 4 }} />
      <For each={devices().inputs} fallback={<Note>No Inputs</Note>}>
        {(device) => deviceRow("inputs", device)}
      </For>
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
