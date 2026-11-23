// What's playing, from the players that say over MPRIS.

import { command, data, now } from "bento";
import { createSignal, Show } from "solid-js";
import { Button, Slider, popup } from "bento:modules/ui";
import { volumeIcon } from "bento:modules/sound";

export const player = () => data("media")?.players?.[0];

const PADDING = 10;

/** What's playing: its cover, title and artist, and its controls; where it is in it, to go
 * elsewhere, and its own volume, when the player lets them be set. A click on the cover or the
 * title brings the player's window to the front, when it can. */
export function MediaCard() {
  const control = (icon: string, act: "PlayPause" | "Next" | "Previous", can: () => boolean) => (
    <Button
      icon={icon}
      size={18}
      style={{ width: 34, height: 34, borderRadius: 17, opacity: can() ? 1 : 0.4 }}
      onClick={() => can() && command("media", { player: player()!.id, action: act })}
    />
  );
  // The card's width, for the volume slider, which is as wide as it's told.
  const [room, setRoom] = createSignal(340);
  const raise = () => {
    if (!player()!.canRaise) return;
    popup.close();
    command("media-raise", { player: player()!.id });
  };
  return (
    <Show when={player()}>
      <box
        style={{
          flexDirection: "column", gap: 8, padding: PADDING, borderRadius: 18,
          background: "fg/0.08",
        }}
        onLayout={(rect) => setRoom(rect.width)}
      >
        <box style={{ flexDirection: "row", alignItems: "center", gap: 10 }}>
          <box
            style={{
              flexDirection: "row", alignItems: "center", gap: 10, flexGrow: 1, flexShrink: 1,
              minWidth: 0, cursor: player()!.canRaise ? "pointer" : "default",
            }}
            onClick={raise}
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
                <image src={player()!.art!} style={{ width: 48, height: 48 }} />
              </box>
            </Show>
            <box style={{ flexDirection: "column", flexGrow: 1, flexShrink: 1, minWidth: 0 }}>
              <text style={{ fontWeight: 700, lineClamp: 1 }}>{player()!.title ?? ""}</text>
              <text style={{ fontSize: 12, color: "fg/0.7", lineClamp: 1 }}>
                {player()!.artist ?? player()!.name}
              </text>
            </box>
          </box>
          {control("media-skip-backward-symbolic", "Previous", () => player()!.canPrevious)}
          {control(
            player()!.playing ? "media-playback-pause-symbolic" : "media-playback-start-symbolic",
            "PlayPause",
            () => player()!.canPlay,
          )}
          {control("media-skip-forward-symbolic", "Next", () => player()!.canNext)}
        </box>
        <Show when={player()!.canSeek}>
          <Seek />
        </Show>
        {/* `!= null`: a data program older than these says nothing of volume. */}
        <Show when={player()!.volume != null}>
          <Slider
            value={Math.min(player()!.volume!, 1)}
            width={room() - PADDING * 2}
            icon={volumeIcon({ volume: player()!.volume!, muted: false })}
            onChange={(value) => command("media-volume", { player: player()!.id, value })}
          />
        </Show>
      </box>
    </Show>
  );
}

/** Where the player is in what it plays, between how far it's gone and how long it is; a press
 * or a drag along it picks a place, which the player goes to once it's let go. */
function Seek() {
  const time = now(500);
  const length = () => player()!.length ?? 0;
  // The place picked while the press is held, in seconds.
  const [held, setHeld] = createSignal<number | null>(null);
  // The place gone to, from `since`, until the player says where it is after `at`.
  const [sent, setSent] = createSignal<{ position: number; since: number; at: number } | null>(null);
  const position = () => {
    time();
    const p = player()!;
    if (held() !== null) return held()!;
    const s = sent();
    const [from, since] = s && s.at === p.at ? [s.position, s.since] : [p.position, p.at];
    const moved = p.playing ? ((Date.now() - since) / 1000) * p.rate : 0;
    return Math.min(Math.max(from + moved, 0), length());
  };
  const at = (event: any) =>
    event.frame?.width > 0 &&
    setHeld(Math.min(Math.max(event.offsetX / event.frame.width, 0), 1) * length());
  const go = () => {
    const place = held();
    if (place === null) return;
    setHeld(null);
    setSent({ position: place, since: Date.now(), at: player()!.at });
    command("media-position", { player: player()!.id, position: place });
  };
  const label = { fontSize: 11, color: "fg/0.6", fontVariantNumeric: "tabular-nums" } as const;
  return (
    <box style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
      <text style={label}>{clock(position())}</text>
      <box
        style={{ flexGrow: 1, height: 16, justifyContent: "center", cursor: "pointer" }}
        onPress={(event) => event.button === 1 && at(event)}
        onDrag={at}
        onRelease={go}
      >
        <box style={{ height: 4, borderRadius: 2, background: "fg/0.15", overflow: "hidden" }}>
          <box
            style={{
              width: `${length() > 0 ? (position() / length()) * 100 : 0}%`, height: 4,
              background: "fg/0.8",
            }}
          />
        </box>
      </box>
      <text style={label}>{clock(length())}</text>
    </box>
  );
}

/** Seconds as `m:ss`, or `h:mm:ss` from an hour on. */
function clock(seconds: number) {
  const whole = Math.floor(seconds);
  const [h, m, s] = [Math.floor(whole / 3600), Math.floor(whole / 60) % 60, whole % 60];
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}
