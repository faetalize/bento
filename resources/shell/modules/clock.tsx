// The clock: the time and the date, on one line or two, and the calendar, with the month and the
// weather where the config says the user is. In bento's own shell it's the middle island, which
// opens into the calendar, its time growing into the calendar's.

import { action, after, command, config, data, now, onData, paced, settling, useOutput } from "bento";
import { createEffect, createMemo, createRoot, createSignal, For, Show } from "solid-js";
import { ACTIVE, Button, Island, popup } from "bento:modules/ui";

const DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const LONG_DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const LONG_MONTHS = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
];
// Weeks start on Monday.
const WEEK = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];

const CELL = 44;
const WIDTH = CELL * 7;

const pad = (n: number) => String(n).padStart(2, "0");
export const clockTime = (at: Date) => `${pad(at.getHours())}:${pad(at.getMinutes())}`;
export const clockDate = (at: Date) => `${DAYS[at.getDay()]} ${at.getDate()} ${MONTHS[at.getMonth()]}`;
export const numericDate = (at: Date) => `${pad(at.getDate())}/${pad(at.getMonth() + 1)}/${at.getFullYear()}`;
export const longDate = (at: Date) =>
  `${LONG_DAYS[at.getDay()]}, ${at.getDate()} ${LONG_MONTHS[at.getMonth()]} ${at.getFullYear()}`;
const sameDay = (a: Date, b: Date) =>
  a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();

// The data program fetches the weather for the place the config has, and stops without one. It
// hears the place again whenever it changes, and when the program starts.
createRoot(() => {
  const ask = () => {
    const { location, fahrenheit } = config();
    command("weather", location ? { ...location, fahrenheit: Boolean(fahrenheit) } : {});
  };
  createEffect(() => JSON.stringify([config().location, config().fahrenheit]), ask);
  onData("ready", ask);
});

/** The time and the date: on one line, the date then the time; on two, the time over the date,
 * as on a taskbar. */
export function Clock(props: { lines?: 1 | 2; style?: any }) {
  const time = now();
  return (
    <Show
      when={props.lines === 2}
      fallback={
        <box style={[{ flexDirection: "row", alignItems: "center", gap: 10, fontWeight: 600 }, props.style]}>
          <text>{clockDate(time())}</text>
          <text style={{ fontVariantNumeric: "tabular-nums" }}>{clockTime(time())}</text>
        </box>
      }
    >
      <box style={[{ flexDirection: "column", alignItems: "end", fontSize: 12 }, props.style]}>
        <text style={{ fontVariantNumeric: "tabular-nums" }}>{clockTime(time())}</text>
        <text style={{ fontVariantNumeric: "tabular-nums" }}>{numericDate(time())}</text>
      </box>
    </Show>
  );
}

/** The month `shown` is in, as six weeks of days from the Monday on or before its first. */
function weeks(shown: Date) {
  const first = new Date(shown.getFullYear(), shown.getMonth(), 1);
  const lead = (first.getDay() + 6) % 7;
  return Array.from({ length: 42 }, (_, i) =>
    new Date(shown.getFullYear(), shown.getMonth(), 1 - lead + i));
}

/** A month: its days, today lit, with arrows and scrolling to turn the page, and its title to
 * come back to today's. */
export function Month() {
  const time = now(60_000);
  const [shown, setShown] = createSignal(new Date(time().getFullYear(), time().getMonth(), 1));
  const turn = (by: number) => setShown((at) => new Date(at.getFullYear(), at.getMonth() + by, 1));
  const today = () => setShown(new Date(time().getFullYear(), time().getMonth(), 1));
  let scrolled = 0;
  const days = createMemo(() => weeks(shown()));
  return (
    <box
      style={{ flexDirection: "column", gap: 6 }}
      onScroll={({ dx, dy }) => {
        scrolled += dy || dx;
        if (Math.abs(scrolled) < 15) return;
        turn(Math.sign(scrolled));
        scrolled = 0;
      }}
    >
      <box style={{ flexDirection: "row", alignItems: "center", gap: 4, paddingLeft: 12 }}>
        <box style={{ flexGrow: 1, cursor: "pointer" }} onClick={today}>
          <text style={{ fontSize: 15, fontWeight: 700 }}>
            {`${LONG_MONTHS[shown().getMonth()]} ${shown().getFullYear()}`}
          </text>
        </box>
        <Button icon="go-previous-symbolic" onClick={() => turn(-1)} />
        <Button icon="go-next-symbolic" onClick={() => turn(1)} />
      </box>
      <box style={{ flexDirection: "row" }}>
        <For each={WEEK}>
          {(day) => (
            <box style={{ width: CELL, justifyContent: "center" }}>
              <text style={{ fontSize: 12, fontWeight: 600, color: "fg/0.5" }}>{day}</text>
            </box>
          )}
        </For>
      </box>
      <box style={{ flexDirection: "column", gap: 2 }}>
        <For each={[0, 1, 2, 3, 4, 5]}>
          {(week) => (
            <box style={{ flexDirection: "row" }}>
              <For each={[0, 1, 2, 3, 4, 5, 6]}>
                {(day) => {
                  const date = () => days()[week * 7 + day];
                  const isToday = () => sameDay(date(), time());
                  return (
                    <box style={{ width: CELL, justifyContent: "center" }}>
                      {/* A circle in the middle of its column, which today's fills. */}
                      <box
                        style={{
                          width: 32, height: 32, borderRadius: 16, alignItems: "center",
                          justifyContent: "center", background: isToday() ? ACTIVE : undefined,
                        }}
                      >
                        <text
                          style={{
                            fontVariantNumeric: "tabular-nums", fontWeight: isToday() ? 700 : 400,
                            color: isToday() ? "#ffffff"
                              : date().getMonth() === shown().getMonth() ? "fg" : "fg/0.35",
                          }}
                        >
                          {String(date().getDate())}
                        </text>
                      </box>
                    </box>
                  );
                }}
              </For>
            </box>
          )}
        </For>
      </box>
    </box>
  );
}

/** The weather where the config says the user is, or what's in the way of it. `onSettings` hears
 * the button that opens the settings to set the place. */
export function Weather(props: { onSettings?: () => void }) {
  const degrees = (value: number) => `${Math.round(value)}°`;
  const weather = () => data("weather");
  const location = () => config().location;
  const details = () => {
    const w = weather();
    if (!w?.ok) return "";
    return [
      w.feels !== null && `Feels ${degrees(w.feels)}`,
      w.wind !== null && `Wind ${Math.round(w.wind)} ${w.windUnit}`,
      w.humidity !== null && `Humidity ${Math.round(w.humidity)}%`,
    ].filter(Boolean).join("  ·  ");
  };
  const dim = { color: "fg/0.7" };
  return (
    <box
      style={{
        flexDirection: "column", gap: 10, padding: 12, borderRadius: 16, background: "fg/0.06",
        width: WIDTH,
      }}
    >
      <Show
        when={location()}
        fallback={
          <>
            <text style={dim}>Set where you are to see the weather here.</text>
            <box style={{ flexDirection: "row" }}>
              <Button
                icon="find-location-symbolic"
                label="Set Location"
                style={{ background: "fg/0.1" }}
                onClick={() => {
                  props.onSettings?.();
                  action("spawn", ["bento-settings", "location"]);
                }}
              />
            </box>
          </>
        }
      >
        <Show when={weather()} fallback={<text style={dim}>Fetching the weather…</text>}>
          <Show when={weather()!.ok} fallback={<text style={dim}>The weather couldn't be fetched.</text>}>
            <box style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
              <image icon={weather()!.icon} size={36} />
              <box style={{ flexDirection: "column", flexGrow: 1 }}>
                <text style={{ fontSize: 22, fontWeight: 700 }}>
                  {`${Math.round(weather()!.temperature)}${weather()!.unit}`}
                </text>
                <text style={dim}>{weather()!.label}</text>
              </box>
            </box>
            <text style={{ fontSize: 12, color: "fg/0.6" }}>{details()}</text>
            <Show when={location()!.name}>
              <text style={{ fontSize: 12, color: "fg/0.6" }}>{location()!.name}</text>
            </Show>
            <box style={{ flexDirection: "row", justifyContent: "space-between" }}>
              <For each={weather()!.days}>
                {(day: any) => {
                  // Noon, so the day is the same in every timezone.
                  const date = new Date(`${day.date}T12:00:00`);
                  return (
                    <box style={{ flexDirection: "column", alignItems: "center", gap: 4, width: 48 }}>
                      <text style={{ fontSize: 12, fontWeight: 600, color: "fg/0.6" }}>
                        {DAYS[date.getDay()]}
                      </text>
                      <image icon={day.icon} size={20} />
                      <text style={{ fontSize: 12 }}>{`${degrees(day.max)} ${degrees(day.min)}`}</text>
                    </box>
                  );
                }}
              </For>
            </box>
          </Show>
        </Show>
      </Show>
    </box>
  );
}

type Rect = { x: number; y: number; width: number; height: number };

/** Moves something at `own` to `target`'s place, as tall as it, its start on the target's start
 * and its middle on its middle, as one thing turning into another. */
function travel(own: Rect | null, target: Rect | null) {
  if (!own || !target || own.height <= 0 || target.height <= 0) {
    return { translate: [0, 0] as [number, number], scale: 1 };
  }
  // Scaled around its middle, so its start moves by half of what it gains or loses.
  const scale = target.height / own.height;
  const dx = target.x - own.x - (own.width * (1 - scale)) / 2;
  const dy = target.y + target.height / 2 - (own.y + own.height / 2);
  return { translate: [dx, dy] as [number, number], scale };
}

/** The middle island of bento's own shell: the date and the time, which open into the calendar.
 * Opening, the island's time grows into the calendar's place for it, and the date travels to the
 * start of the calendar's date and, once there, blurs into it. Closing, the date blurs back into
 * the island's at once, and both go home with the glass. */
export function ClockIsland() {
  const output = useOutput();
  const key = `calendar:${output}`;
  const time = now();
  const open = () => popup.is(key);
  // Where each part is laid out, to travel between.
  const [timeAt, setTimeAt] = createSignal<Rect | null>(null);
  const [dateAt, setDateAt] = createSignal<Rect | null>(null);
  const [bigTimeAt, setBigTimeAt] = createSignal<Rect | null>(null);
  const [bigDateAt, setBigDateAt] = createSignal<Rect | null>(null);
  // Whether the date has blurred into the calendar's.
  const [handed, setHanded] = createSignal(false);
  let stop: (() => void) | null = null;
  createEffect(open, (isOpen) => {
    stop?.();
    stop = null;
    setHanded(false);
    if (isOpen) stop = after(paced(settling()), () => setHanded(true));
  });
  const QUICK = { opacity: 0.1, blur: 0.1 };
  return (
    <Island
      hold="center"
      gap={10}
      open={open() ? "calendar" : null}
      style={{ cursor: "pointer" }}
      onClick={() => popup.toggle(key)}
      panels={{
        calendar: () => (
          <box style={{ flexDirection: "column", gap: 14, width: WIDTH }}>
            <box style={{ flexDirection: "column", gap: 2, paddingLeft: 12 }}>
              <text
                style={{ fontSize: 40, fontWeight: 600, fontVariantNumeric: "tabular-nums", opacity: 0 }}
                onLayout={setBigTimeAt}
              >
                {clockTime(time())}
              </text>
              <text
                style={[
                  { color: "fg/0.6" },
                  handed() ? { opacity: 1, blur: 0 } : { opacity: 0, blur: 4, transition: QUICK },
                ]}
                onLayout={setBigDateAt}
              >
                {longDate(time())}
              </text>
            </box>
            <Month />
            <Weather onSettings={() => popup.close()} />
          </box>
        ),
      }}
    >
      <text
        style={[
          { fontWeight: 600 },
          open() ? travel(dateAt(), bigDateAt()) : { translate: [0, 0], scale: 1 },
          handed() ? { opacity: 0, blur: 4 } : { opacity: 1, blur: 0, transition: QUICK },
        ]}
        onLayout={setDateAt}
      >
        {clockDate(time())}
      </text>
      <text
        style={[
          { fontWeight: 600, fontVariantNumeric: "tabular-nums" },
          open() ? travel(timeAt(), bigTimeAt()) : { translate: [0, 0], scale: 1 },
        ]}
        onLayout={setTimeAt}
      >
        {clockTime(time())}
      </text>
    </Island>
  );
}
