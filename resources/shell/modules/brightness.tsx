// The displays' brightness, set by ddcutil or the backlight through the data program.

import { command, data } from "bento";
import { For } from "solid-js";
import { Slider } from "bento:modules/ui";

export interface Display {
  id: string;
  label: string;
  value: number;
}

export const displays = (): Display[] => data("brightness")?.displays ?? [];

/** The display of output `output`, or the first one. */
export const displayOf = (output: string) =>
  displays().find((display) => display.id === output) ?? displays()[0];

export const setBrightness = (id: string, value: number) => command("brightness", { id, value });

/** A display's brightness, as a slider. */
export function BrightnessSlider(props: { display: Display; width?: number }) {
  return (
    <Slider
      width={props.width}
      icon="display-brightness-symbolic"
      value={props.display.value}
      onChange={(value) => setBrightness(props.display.id, value)}
    />
  );
}

/** Every display's brightness. */
export function DisplaysPage(props: { width: number }) {
  return (
    <box style={{ flexDirection: "column", gap: 2 }}>
      <For each={displays()}>
        {(display) => (
          <box style={{ flexDirection: "column", gap: 4, paddingBottom: 8 }}>
            <text style={{ fontSize: 12, color: "fg/0.7" }}>{display.label}</text>
            <BrightnessSlider display={display} width={props.width} />
          </box>
        )}
      </For>
    </box>
  );
}
