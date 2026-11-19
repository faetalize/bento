// Bluetooth: whether it's on, and the devices known and nearby, connected with a click.

import { command, data, onData } from "bento";
import { createSignal, For, onCleanup, Show } from "solid-js";
import { Heading, Note, Row } from "bento:modules/ui";

/** How Bluetooth is, for a tile. */
export function bluetoothState() {
  const bluetooth = data("bluetooth");
  const connected = bluetooth?.devices?.filter((d: any) => d.connected) ?? [];
  return {
    there: Boolean(bluetooth?.available),
    active: Boolean(bluetooth?.powered),
    subtitle: !bluetooth?.powered ? "Off"
      : connected.length === 1 ? connected[0].name
      : connected.length ? `${connected.length} Devices` : "On",
  };
}

export const setBluetooth = (on: boolean) => command("bluetooth-power", { on });

const [error, setError] = createSignal("");
onData("bluetooth-error", (event) => setError(event.message));

/** The devices known and nearby, looked for while it shows. */
export function BluetoothPage() {
  const bluetooth = () => data("bluetooth");
  setError("");
  if (bluetooth()?.powered) command("bluetooth-scan", { on: true });
  onCleanup(() => command("bluetooth-scan", { on: false }));
  const deviceRow = (device: any) => (
    <Row
      icon={device.icon}
      title={device.name}
      lit={device.connected}
      subtitle={device.connecting ? "Connecting…" : device.connected ? "Connected" : ""}
      onClick={device.connecting ? undefined : () => command(
        device.connected ? "bluetooth-disconnect" : "bluetooth-connect",
        { path: device.path },
      )}
    />
  );
  const known = () => bluetooth()?.devices?.filter((d: any) => d.paired) ?? [];
  const nearby = () => bluetooth()?.devices?.filter((d: any) => !d.paired) ?? [];
  return (
    <box style={{ flexDirection: "column", gap: 2 }}>
      <Show when={bluetooth()?.powered} fallback={<Note>Bluetooth is off.</Note>}>
        <For each={known()} fallback={<Note>No devices yet.</Note>}>{deviceRow}</For>
        <box style={{ padding: [10, 12, 2, 12] }}>
          <Heading>{bluetooth()!.discovering ? "Nearby · Looking…" : "Nearby"}</Heading>
        </box>
        <For each={nearby()}>{deviceRow}</For>
      </Show>
      <Show when={error()}>
        <Note>{error()}</Note>
      </Show>
    </box>
  );
}
