// What's playing, from the players that say over MPRIS.

import { command, data } from "bento";
import { Show } from "solid-js";
import { Button } from "bento:modules/ui";

export const player = () => data("media")?.players?.[0];

/** What's playing: its cover, title and artist, and its controls. */
export function MediaCard() {
  const control = (icon: string, act: string, can: () => boolean) => (
    <Button
      icon={icon}
      size={18}
      style={{ width: 34, height: 34, borderRadius: 17, opacity: can() ? 1 : 0.4 }}
      onClick={() => can() && command("media", { player: player()!.id, action: act })}
    />
  );
  return (
    <Show when={player()}>
      <box
        style={{
          flexDirection: "row", alignItems: "center", gap: 10, padding: 10, borderRadius: 18,
          background: "fg/0.08",
        }}
      >
        <Show
          when={player()!.art}
          fallback={
            <box
              style={{
                width: 48, height: 48, borderRadius: 12, background: "fg/0.1", flexShrink: 0,
                alignItems: "center", justifyContent: "center",
              }}
            >
              <image icon={player()!.icon ?? "audio-x-generic-symbolic"} size={28} />
            </box>
          }
        >
          <box style={{ width: 48, height: 48, borderRadius: 12, overflow: "hidden", flexShrink: 0 }}>
            <image src={player()!.art} style={{ width: 48, height: 48 }} />
          </box>
        </Show>
        <box style={{ flexDirection: "column", flexGrow: 1, flexShrink: 1, minWidth: 0 }}>
          <text style={{ fontWeight: 700, lineClamp: 1 }}>{player()!.title ?? ""}</text>
          <text style={{ fontSize: 12, color: "fg/0.7", lineClamp: 1 }}>
            {player()!.artist ?? player()!.name}
          </text>
        </box>
        {control("media-skip-backward-symbolic", "Previous", () => player()!.canPrevious)}
        {control(
          player()!.playing ? "media-playback-pause-symbolic" : "media-playback-start-symbolic",
          "PlayPause",
          () => player()!.canPlay,
        )}
        {control("media-skip-forward-symbolic", "Next", () => player()!.canNext)}
      </box>
    </Show>
  );
}
