// Networks: whether the computer is online and how, and Wi-Fi's networks to join.

import { command, data, onData } from "bento";
import { createSignal, For, Show } from "solid-js";
import { Button, Note, Row } from "bento:modules/ui";

/** The icon for a Wi-Fi signal of `strength`, from 0 to 100. */
export const signalIcon = (strength: number) =>
  `network-wireless-signal-${
    strength > 75 ? "excellent" : strength > 50 ? "good" : strength > 25 ? "ok" : "weak"
  }-symbolic`;

/** Whether the computer is online by cable. */
export const wired = () => {
  const network = data("network");
  return Boolean(network?.connected && network.icon?.includes("wired"));
};

/** The status icon: how the computer is online, if it is. */
export const networkIcon = () => data("network")?.icon ?? "network-offline-symbolic";

/** How Wi-Fi is, for a tile. */
export function wifiState() {
  const wifi = data("wifi");
  const joined = wifi?.networks?.find((n: any) => n.active);
  return {
    there: Boolean(wifi?.available),
    active: Boolean(wifi?.enabled),
    icon: joined ? signalIcon(joined.strength) : "network-wireless-symbolic",
    subtitle: wifi?.joining ? "Connecting…" : joined ? joined.ssid
      : wired() ? "Ethernet" : wifi?.enabled ? "Not Connected" : "Off",
  };
}

export const setWifi = (on: boolean) => command("wifi-enable", { on });

// What went wrong joining, last.
const [error, setError] = createSignal("");
onData("wifi-error", (event) => setError(event.message));

/** The networks around, joined with a click, and with a password when one's new and has one. */
export function WifiPage() {
  // The network a password is asked for.
  const [asking, setAsking] = createSignal<string | null>(null);
  let field: any;
  const join = (ssid: string, password?: string) => {
    setError("");
    setAsking(null);
    command("wifi-join", password ? { ssid, password } : { ssid });
  };
  setError("");
  command("wifi-scan", {});
  const wifi = () => data("wifi");
  return (
    <box style={{ flexDirection: "column", gap: 2 }}>
      <Show when={wired()}>
        <Row icon={data("network")?.icon} title="Ethernet" subtitle="Connected" />
      </Show>
      <Show when={wifi()?.enabled} fallback={<Note>Wi-Fi is off.</Note>}>
        <Show when={wifi()!.networks.length > 0} fallback={<Note>Looking for networks…</Note>}>
          <For each={wifi()!.networks}>
            {(network: any) => (
              <box style={{ flexDirection: "column" }}>
                <Row
                  icon={signalIcon(network.strength)}
                  title={network.ssid}
                  subtitle={wifi()!.joining === network.ssid ? "Connecting…"
                    : network.active ? "Connected" : network.known ? "Saved" : ""}
                  lit={network.active}
                  onClick={network.active || wifi()!.joining === network.ssid ? undefined : () => {
                    if (network.secure && !network.known) {
                      setAsking(network.ssid);
                      queueMicrotask(() => field?.focus());
                    } else {
                      join(network.ssid);
                    }
                  }}
                >
                  <Show when={network.secure}>
                    <image icon="system-lock-screen-symbolic" size={14} style={{ color: "fg/0.5" }} />
                  </Show>
                  <Show when={network.active}>
                    <Button
                      label="Disconnect"
                      style={{ height: 26, background: "fg/0.08" }}
                      onClick={() => command("wifi-disconnect", {})}
                    />
                  </Show>
                </Row>
                <Show when={asking() === network.ssid}>
                  <box style={{ flexDirection: "row", alignItems: "center", gap: 8, padding: [4, 0, 6, 12] }}>
                    <box
                      style={{
                        flexGrow: 1, flexShrink: 1, minWidth: 0, padding: [8, 12], borderRadius: 12,
                        background: "fg/0.08", fontSize: 14,
                      }}
                    >
                      <input
                        ref={field}
                        placeholder="Password"
                        onSubmit={(typed) => join(network.ssid, typed)}
                      />
                    </box>
                    <Button
                      label="Join"
                      active
                      style={{ height: 34 }}
                      onClick={() => join(network.ssid, field?.text)}
                    />
                  </box>
                </Show>
              </box>
            )}
          </For>
        </Show>
      </Show>
      <Show when={error()}>
        <Note>{error()}</Note>
      </Show>
    </box>
  );
}
