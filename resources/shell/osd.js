// On-screen indicators, at the bottom middle of the focused output: the volume's level for a
// moment after it changes elsewhere, by a key or a headset's knob. Each grows out of nothing and
// shrinks back into nothing.

import {
  box, text, slider, root, on, last, data, after, cancel, focusedOutput, outputs, grow, shrink,
  style,
} from "bento:ui";

// How long an indicator stays after the last change, in milliseconds.
const STAYS = 1500;
// How far above the bottom edge it shows, past bento's gaps.
const ABOVE = 48;
const HEIGHT = 44;
const RADIUS = HEIGHT / 2;

export const volumeIcon = ({ volume, muted }) =>
  muted || volume <= 0 ? "audio-volume-muted-symbolic"
  : volume < 0.34 ? "audio-volume-low-symbolic"
  : volume < 0.67 ? "audio-volume-medium-symbolic"
  : "audio-volume-high-symbolic";

export const micIcon = ({ muted }) =>
  muted ? "microphone-sensitivity-muted-symbolic" : "microphone-sensitivity-high-symbolic";

// Until when changes are the shell's own, from a slider that shows them already.
let quietUntil = 0;

// Says that the shell itself is changing the volume, which needs no indicator.
export function quiet() {
  quietUntil = Date.now() + 1000;
}

const devices = () => last("audio-devices") ?? { outputs: [], inputs: [] };
const active = (kind) => devices()[kind].find((device) => device.active);

// Each output's indicator, made the first time it shows there.
const indicators = new Map();
// Which list the indicator shows a device of, "outputs" or "inputs".
let kind = "outputs";
let showing = null;
let timer;

function indicator(output) {
  if (indicators.has(output)) return indicators.get(output);
  const top = root(output);
  if (!top) return null;
  const level = slider({
    width: 220, icon: "audio-volume-high-symbolic",
    onChange: (value) => {
      const device = active(kind);
      if (device) data("device-volume", { kind: kind.slice(0, -1), name: device.name, value });
      stay();
    },
    onIcon: () => {
      const device = active(kind);
      if (device) {
        data("device-mute", { kind: kind.slice(0, -1), name: device.name, value: !device.muted });
      }
      stay();
    },
  });
  const percent = text("", { weight: 600, tabular: true });
  const content = box({ align: "center", gap: 10 }, [
    level,
    box({ width: 40, justify: "end" }, [percent]),
  ]);
  const pill = box(
    {
      material: "glass", radius: RADIUS, height: HEIGHT, padding: [0, 14, 0, 8], align: "center",
      clip: true, hidden: true,
    },
    [content],
  );
  pill.gone = true;
  const frame = box(
    {
      position: "absolute", top: 0, left: 0, right: 0, bottom: 0, direction: "column",
      align: "center", justify: "end", padding: [0, 0, (style().gaps ?? 8) + ABOVE, 0],
    },
    [pill],
  );
  top.add(frame);
  const made = { output, pill, content, level, percent };
  indicators.set(output, made);
  return made;
}

// Shows what the device of `kind` is at now.
function fill(shown) {
  const device = active(kind);
  if (!device) return;
  const icon = kind === "outputs" ? volumeIcon(device) : micIcon(device);
  shown.level.show(Math.min(device.volume, 1), icon);
  shown.percent.set({
    text: `${Math.round(device.volume * 100)}%`, color: device.muted ? "fg/0.5" : "fg",
  });
}

// It goes once nothing has changed for a moment.
function stay() {
  if (timer !== undefined) cancel(timer);
  timer = after(STAYS, () => {
    timer = undefined;
    if (!showing) return;
    shrink(showing.pill, { content: [showing.content] });
    showing = null;
  });
}

function show(output) {
  const shown = indicator(output);
  if (!shown) return;
  if (showing && showing !== shown) {
    shrink(showing.pill, { content: [showing.content] });
  }
  fill(shown);
  grow(shown.pill, { radius: RADIUS, content: [shown.content] });
  showing = shown;
  stay();
}

// What the active devices were, to tell a change of volume from a change of device.
const sound = (list) => {
  const device = list.find((one) => one.active);
  return device && { name: device.name, volume: device.volume, muted: device.muted };
};
let heard = last("audio-devices")
  ? { outputs: sound(devices().outputs), inputs: sound(devices().inputs) }
  : null;

on("audio-devices", () => {
  const before = heard;
  heard = { outputs: sound(devices().outputs), inputs: sound(devices().inputs) };
  if (showing) fill(showing);
  if (!before || Date.now() < quietUntil) return;
  const changed = ["outputs", "inputs"].find((which) => {
    const [was, is] = [before[which], heard[which]];
    return was && is && was.name === is.name
      && (was.volume !== is.volume || was.muted !== is.muted);
  });
  if (!changed) return;
  kind = changed;
  show(focusedOutput() ?? outputs()[0]);
});
