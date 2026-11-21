// Power: the battery, the power mode, and suspending, restarting, turning off and logging out.

import { action, after, command, data } from "bento";
import { createSignal, For } from "solid-js";
import { Row } from "bento:modules/ui";

const PROFILES: Record<string, [string, string]> = {
  "power-saver": ["Power Saver", "power-profile-power-saver-symbolic"],
  balanced: ["Balanced", "power-profile-balanced-symbolic"],
  performance: ["Performance", "power-profile-performance-symbolic"],
};
export const profileName = (name: string) => PROFILES[name]?.[0] ?? name;
export const profileIcon = (name: string) => PROFILES[name]?.[1] ?? "power-profile-balanced-symbolic";

/** How the power mode is, for a tile. */
export function profileState() {
  const profile = data("power-profile");
  const active = profile?.active ?? "balanced";
  return {
    there: Boolean(profile?.available),
    active: active !== "balanced",
    icon: profileIcon(active),
    subtitle: profileName(active),
  };
}

/** The power modes, to pick one. */
export function ProfilePage() {
  const profile = () => data("power-profile");
  return (
    <box style={{ flexDirection: "column", gap: 2 }}>
      <For each={profile()?.profiles ?? []}>
        {(name: string) => (
          <Row
            icon={profileIcon(name)}
            title={profileName(name)}
            lit={profile()?.active === name}
            onClick={() => command("power-profile", { name })}
          />
        )}
      </For>
    </box>
  );
}

/** The battery, if there's one: its icon, and how full it is. */
export function battery() {
  const state = data("battery");
  if (!state?.present) return null;
  const percent = `${Math.round(state.percentage ?? 0)}%`;
  return {
    icon: state.icon || "battery-missing-symbolic",
    percent,
    words: `${percent}${state.charging ? " · Charging" : ""}`,
  };
}

/** Suspending at once; restarting, turning off and logging out on a second click. `onDone`
 * hears one go. */
export function PowerPage(props: { onDone?: () => void }) {
  // Which waits for its second click, for a moment.
  const [armed, setArmed] = createSignal<string | null>(null);
  let disarm: (() => void) | null = null;
  const POWER: [string, string, () => void, boolean][] = [
    ["Suspend", "weather-clear-night-symbolic", () => command("suspend", {}), false],
    ["Restart", "system-reboot-symbolic", () => command("reboot", {}), true],
    ["Power Off", "system-shutdown-symbolic", () => command("power-off", {}), true],
    [
      "Log Out", "system-log-out-symbolic",
      () => action("quit", [], { properties: { "skip-confirmation": true } }), true,
    ],
  ];
  return (
    <box style={{ flexDirection: "column", gap: 2 }}>
      <For each={POWER}>
        {([title, icon, act, confirm]) => (
          <Row
            icon={icon}
            title={title}
            subtitle={armed() === title ? "Click again to confirm" : ""}
            lit={armed() === title}
            onClick={() => {
              if (confirm && armed() !== title) {
                setArmed(title);
                disarm?.();
                disarm = after(3000, () => setArmed(null));
                return;
              }
              setArmed(null);
              props.onDone?.();
              act();
            }}
          />
        )}
      </For>
    </box>
  );
}
