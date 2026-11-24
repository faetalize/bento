// What's playing, from the players that say over MPRIS.

import { command, data, now } from "bento";
import { createEffect, createSignal, Show } from "solid-js";
import { Button, Line, popup } from "bento:modules/ui";
import { volumeIcon } from "bento:modules/sound";
import { appById, appIcon } from "bento:modules/apps";

export const player = () => data("media")?.players?.[0];

type Player = NonNullable<ReturnType<typeof player>>;

const PADDING = 10;
const COVER = 48;
// How far the backdrop's blur reaches past the card, so its edges don't fade.
const BLUR = 18;
// The room on either side of the controls, for the volume and the player's name.
const SIDE = 84;

/** What's playing, over its cover blurred: the cover, title and artist; a line for how far it's
 * gone, to go elsewhere along it; and under it the controls, between the player's own volume and
 * its name, which brings its window to the front, as does a click on the cover or the title.
 * What the player can't do isn't there. */
export function MediaCard() {
  const [rect, setRect] = createSignal({ width: 340, height: 120 });
  const raise = () => {
    if (!player()!.canRaise) return;
    popup.close();
    command("media-raise", { player: player()!.id });
  };
  const control = (
    icon: string,
    act: "PlayPause" | "Next" | "Previous",
    can: () => boolean,
    style?: any,
  ) => (
    <Button
      icon={icon}
      size={18}
      style={{ width: 34, height: 34, borderRadius: 17, opacity: can() ? 1 : 0.4, ...style }}
      onClick={() => can() && command("media", { player: player()!.id, action: act })}
    />
  );
  return (
    <Show when={player()}>
      <box
        style={{
          flexDirection: "column", padding: PADDING, borderRadius: 18, background: "fg/0.08",
          overflow: "hidden",
        }}
        onLayout={(r) => setRect({ width: r.width, height: r.height })}
      >
        <Show when={player()!.art}>
          <Fitted
            src={player()!.art!}
            aspect={player()!.artAspect}
            left={-BLUR}
            top={-BLUR}
            width={rect().width + BLUR * 2}
            height={rect().height + BLUR * 2}
            style={{ blur: BLUR, opacity: 0.55 }}
          />
          <box
            style={{
              position: "absolute", left: 0, top: 0, width: rect().width, height: rect().height,
              background: "window/0.45",
            }}
          />
        </Show>
        <box
          style={{
            flexDirection: "row", alignItems: "center", gap: 10,
            cursor: player()!.canRaise ? "pointer" : "default",
          }}
          onClick={raise}
        >
          <Cover player={player()!} />
          <box style={{ flexDirection: "column", flexGrow: 1, flexShrink: 1, minWidth: 0 }}>
            <text style={{ fontWeight: 700, lineClamp: 1 }}>{player()!.title ?? ""}</text>
            <text style={{ fontSize: 12, color: "fg/0.7", lineClamp: 1 }}>
              {player()!.artist ?? player()!.name}
            </text>
          </box>
        </box>
        <Show when={player()!.canSeek}>
          <box style={{ flexDirection: "column", marginTop: 10 }}>
            <Seek />
          </box>
        </Show>
        <box
          style={{
            flexDirection: "row", alignItems: "center", marginTop: player()!.canSeek ? 2 : 8,
          }}
        >
          <box style={{ width: SIDE, flexShrink: 0, flexDirection: "column" }}>
            {/* `!= null`: a data program older than these says nothing of volume. */}
            <Show when={player()!.volume != null}>
              <Volume />
            </Show>
          </box>
          <box style={{ flexDirection: "row", flexGrow: 1, justifyContent: "center", gap: 6 }}>
            {control("media-skip-backward-symbolic", "Previous", () => player()!.canPrevious)}
            {control(
              player()!.playing ? "media-playback-pause-symbolic" : "media-playback-start-symbolic",
              "PlayPause",
              () => player()!.canPlay,
              { background: "fg/0.14" },
            )}
            {control("media-skip-forward-symbolic", "Next", () => player()!.canNext)}
          </box>
          <box
            style={{ width: SIDE, flexShrink: 0, flexDirection: "row", justifyContent: "flex-end" }}
          >
            <Show when={player()!.canRaise}>
              <Owner player={player()!} onClick={raise} />
            </Show>
          </box>
        </box>
      </box>
    </Show>
  );
}

/** The cover, cropped square, or the player's icon. */
function Cover(props: { player: Player }) {
  return (
    <Show
      when={props.player.art}
      fallback={
        <box
          style={{
            width: COVER, height: COVER, borderRadius: 12, background: "fg/0.1", flexShrink: 0,
            alignItems: "center", justifyContent: "center",
          }}
        >
          <image icon={props.player.icon ?? "audio-x-generic-symbolic"} size={28} />
        </box>
      }
    >
      <box
        style={{
          width: COVER, height: COVER, borderRadius: 12, overflow: "hidden", flexShrink: 0,
        }}
      >
        <Fitted
          src={props.player.art!}
          aspect={props.player.artAspect}
          left={0}
          top={0}
          width={COVER}
          height={COVER}
        />
      </box>
    </Show>
  );
}

/** A picture filling a place as CSS's `object-fit: cover` does, centred and cut to it by what
 * holds it. Without its `aspect`, it's stretched to the place. */
function Fitted(props: {
  src: string;
  aspect: number | null | undefined;
  left: number;
  top: number;
  width: number;
  height: number;
  style?: any;
}) {
  const size = () => {
    const aspect = props.aspect;
    if (!aspect) return { width: props.width, height: props.height };
    return aspect > props.width / props.height
      ? { width: props.height * aspect, height: props.height }
      : { width: props.width, height: props.width / aspect };
  };
  return (
    <image
      src={props.src}
      style={{
        position: "absolute", ...size(),
        left: props.left + (props.width - size().width) / 2,
        top: props.top + (props.height - size().height) / 2,
        ...props.style,
      }}
    />
  );
}

/** The player's app, by its icon and name, with an arrow out, which brings its window to the
 * front. */
function Owner(props: { player: Player; onClick: () => void }) {
  const app = () => (props.player.icon ? appById(props.player.icon) : undefined);
  return (
    <box
      style={{
        flexDirection: "row", alignItems: "center", gap: 4, padding: [4, 6], borderRadius: 8,
        cursor: "pointer", minWidth: 0, flexShrink: 1,
      }}
      onClick={props.onClick}
    >
      <Show when={app()}>
        <image icon={appIcon(app())} size={12} style={{ flexShrink: 0 }} />
      </Show>
      <text style={{ fontSize: 11, color: "fg/0.65", lineClamp: 1, flexShrink: 1 }}>
        {app()?.name ?? props.player.name}
      </text>
      <image
        icon="ph-arrow-square-out-symbolic"
        size={11}
        style={{ color: "fg/0.65", flexShrink: 0 }}
      />
    </box>
  );
}

/** Where the player is in what it plays, over how far it's gone and how long it is. The place
 * picked along it is gone to once the press is let go. */
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
  const go = (place: number) => {
    setSent({ position: place, since: Date.now(), at: player()!.at });
    command("media-position", { player: player()!.id, position: place });
  };
  const label = { fontSize: 11, color: "fg/0.6", fontVariantNumeric: "tabular-nums" } as const;
  return (
    <box style={{ flexDirection: "column" }}>
      <Line
        value={length() > 0 ? position() / length() : 0}
        onPick={(value) => setHeld(value * length())}
        onLetGo={() => {
          const place = held();
          setHeld(null);
          if (place !== null) go(place);
        }}
        onWheel={(up) => go(Math.min(Math.max(position() + (up ? 5 : -5), 0), length()))}
      />
      <box style={{ flexDirection: "row", justifyContent: "space-between" }}>
        <text style={label}>{clock(position())}</text>
        <text style={label}>{clock(length())}</text>
      </box>
    </box>
  );
}

/** The player's own volume, set as it's moved. */
function Volume() {
  // What it's set to here, until the player says the same.
  const [set, setSet] = createSignal<number | null>(null);
  createEffect(
    () => player()!.volume,
    () => {
      setSet(null);
    },
  );
  const value = () => Math.min(set() ?? player()!.volume ?? 0, 1);
  const put = (next: number) => {
    const clamped = Math.min(Math.max(next, 0), 1);
    if (clamped === value()) return;
    setSet(clamped);
    command("media-volume", { player: player()!.id, value: clamped });
  };
  return (
    <box style={{ flexDirection: "row", alignItems: "center", gap: 6 }}>
      <image
        icon={volumeIcon({ volume: value(), muted: false })}
        size={14}
        style={{ color: "fg/0.7", flexShrink: 0 }}
      />
      <Line
        value={value()}
        onPick={put}
        onWheel={(up) => put(value() + (up ? 0.05 : -0.05))}
      />
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
