// The clock, in the middle island, which opens into the calendar: the time and the date, the
// month, and the weather where the config says the user is.

import {
  box, text, image, button, island, expand, refill, travel, home, after, cancel, paced,
  settling, on, config, data, action, last, closeAll, ACTIVE,
} from "bento:ui";

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

const pad = (n) => String(n).padStart(2, "0");
const clockTime = (now) => `${pad(now.getHours())}:${pad(now.getMinutes())}`;
const clockDate = (now) => `${DAYS[now.getDay()]} ${now.getDate()} ${MONTHS[now.getMonth()]}`;
const longDate = (now) =>
  `${LONG_DAYS[now.getDay()]}, ${now.getDate()} ${LONG_MONTHS[now.getMonth()]} ${now.getFullYear()}`;
const sameDay = (a, b) =>
  a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();

// The data program fetches the weather for the place the config has, and stops without one. It
// hears the place again whenever it changes, and when the program starts.
function askForWeather() {
  const { location, fahrenheit } = config();
  data("weather", location ? { ...location, fahrenheit: Boolean(fahrenheit) } : {});
}
askForWeather();
on("config", askForWeather);
on("data-ready", askForWeather);

// The month `shown` is in, as six weeks of days from the Monday on or before its first.
function weeks(shown) {
  const first = new Date(shown.getFullYear(), shown.getMonth(), 1);
  const lead = (first.getDay() + 6) % 7;
  return Array.from({ length: 42 }, (_, i) =>
    new Date(shown.getFullYear(), shown.getMonth(), 1 - lead + i));
}

// What the weather section shows: the weather, or what's in the way of it.
function weatherRows(weather, location) {
  if (!location) {
    return [
      text("Set where you are to see the weather here.", { color: "fg/0.7" }),
      box({}, [
        button({
          icon: "find-location-symbolic", label: "Set Location", fill: "fg/0.1",
          onClick: () => {
            closeAll();
            action("spawn", ["bento-settings", "location"]);
          },
        }),
      ]),
    ];
  }
  if (!weather) return [text("Fetching the weather…", { color: "fg/0.7" })];
  if (!weather.ok) return [text("The weather couldn't be fetched.", { color: "fg/0.7" })];

  const degrees = (value) => `${Math.round(value)}°`;
  const details = [
    weather.feels !== null && `Feels ${degrees(weather.feels)}`,
    weather.wind !== null && `Wind ${Math.round(weather.wind)} ${weather.windUnit}`,
    weather.humidity !== null && `Humidity ${Math.round(weather.humidity)}%`,
  ].filter(Boolean);
  return [
    box({ align: "center", gap: 12 }, [
      image(weather.icon, { size: 36, color: "fg" }),
      box({ direction: "column", grow: 1 }, [
        text(`${Math.round(weather.temperature)}${weather.unit}`, { size: 22, weight: 700 }),
        text(weather.label, { color: "fg/0.7" }),
      ]),
    ]),
    text(details.join("  ·  "), { size: 12, color: "fg/0.6" }),
    location.name ? text(location.name, { size: 12, color: "fg/0.6" }) : null,
    box({ justify: "between" }, weather.days.map((day) => {
      // Noon, so the day is the same in every timezone.
      const date = new Date(`${day.date}T12:00:00`);
      return box({ direction: "column", align: "center", gap: 4, width: 48 }, [
        text(DAYS[date.getDay()], { size: 12, weight: 600, color: "fg/0.6" }),
        image(day.icon, { size: 20, color: "fg" }),
        text(`${degrees(day.max)} ${degrees(day.min)}`, { size: 12 }),
      ]);
    })),
  ];
}

// The middle island of an output. Gives `{ island, panel, tick(now) }`: `tick` shows the time,
// and `panel` is the calendar, which `onToggle(open)` hears open and close.
export function calendar(onToggle) {
  let now = new Date();
  // The island's time and date, and the calendar's date. The island's time is the calendar's
  // too: opening, it grows into the room the calendar keeps for it, as big as the calendar's
  // time would be, and stays there.
  const time = text(clockTime(now), { weight: 600, tabular: true });
  const shortDate = text(clockDate(now), { weight: 600 });
  const bigTime = text(clockTime(now), { size: 40, weight: 600, tabular: true, opacity: 0 });
  const bigDate = text(longDate(now), { color: "fg/0.6", opacity: 0 });

  // The month on show, which the arrows and scrolling change, and its days.
  let shown = new Date(now.getFullYear(), now.getMonth(), 1);
  const title = text("", { size: 15, weight: 700 });
  const days = Array.from({ length: 42 }, () => {
    const label = text("", { tabular: true });
    // A circle in the middle of its column, which today's fills.
    const circle = box({ width: 32, height: 32, radius: 16, align: "center", justify: "center" }, [label]);
    return { label, circle, cell: box({ width: CELL, justify: "center" }, [circle]) };
  });
  const showMonth = () => {
    title.set({ text: `${LONG_MONTHS[shown.getMonth()]} ${shown.getFullYear()}` });
    weeks(shown).forEach((date, i) => {
      const today = sameDay(date, now);
      const inMonth = date.getMonth() === shown.getMonth();
      days[i].circle.set({ fill: today ? ACTIVE : null });
      days[i].label.set({
        text: String(date.getDate()),
        weight: today ? 700 : 400,
        color: today ? "#ffffff" : inMonth ? "fg" : "fg/0.35",
      });
    });
  };
  const turn = (by) => {
    shown = new Date(shown.getFullYear(), shown.getMonth() + by, 1);
    showMonth();
  };
  const backToToday = () => {
    shown = new Date(now.getFullYear(), now.getMonth(), 1);
    showMonth();
  };
  showMonth();

  let scrolled = 0;
  const month = box(
    {
      direction: "column", gap: 6,
      onScroll: ({ dx, dy }) => {
        scrolled += dy || dx;
        if (Math.abs(scrolled) < 15) return;
        turn(Math.sign(scrolled));
        scrolled = 0;
      },
    },
    [
      box({ align: "center", gap: 4, padding: [0, 0, 0, 12] }, [
        box({ grow: 1, cursor: "pointer", onClick: backToToday }, [title]),
        button({ icon: "go-previous-symbolic", onClick: () => turn(-1) }),
        button({ icon: "go-next-symbolic", onClick: () => turn(1) }),
      ]),
      box({}, WEEK.map((day) =>
        box({ width: CELL, justify: "center" }, [
          text(day, { size: 12, weight: 600, color: "fg/0.5" }),
        ]))),
      box({ direction: "column", gap: 2 }, Array.from({ length: 6 }, (_, week) =>
        box({}, days.slice(week * 7, week * 7 + 7).map((day) => day.cell)))),
    ],
  );

  const weather = box({
    direction: "column", gap: 10, padding: 12, radius: 16, fill: "fg/0.06", width: WIDTH,
  });
  const showWeather = () => {
    refill(weather, weatherRows(last("weather"), config().location));
    panel.fit();
  };

  const content = box({ direction: "column", gap: 14, width: WIDTH }, [
    box({ direction: "column", gap: 2, padding: [0, 0, 0, 12] }, [bigTime, bigDate]),
    month,
    weather,
  ]);
  // Held to its middle, which stays put as the island grows both ways.
  const pill = island({ gap: 10, justify: "center", cursor: "pointer", onClick: () => panel.toggle() }, [
    shortDate,
    time,
  ]);
  // Opening, the island's time grows into the calendar's place for it, and the date travels to
  // the start of the calendar's date and, once there, blurs into it. Closing, the date blurs
  // back into the island's at once, and both go home with the glass.
  let handing;
  const handOver = (open) => {
    if (handing !== undefined) cancel(handing);
    handing = undefined;
    if (open) {
      travel(time, bigTime);
      travel(shortDate, bigDate);
      handing = after(paced(settling()), () => {
        handing = undefined;
        shortDate.set({ opacity: 0, blur: 4 });
        bigDate.set({ opacity: 1, blur: 0, jump: { blur: 4 } });
      });
    } else {
      const QUICK = { opacity: 0.1, blur: 0.1 };
      shortDate.set({ opacity: 1, blur: 0, transition: QUICK });
      bigDate.set({ opacity: 0, blur: 4, transition: QUICK });
      home(time);
      home(shortDate);
    }
  };
  const panel = expand(pill, content, {
    hold: "center",
    onToggle: (open) => {
      if (open) backToToday();
      handOver(open);
      onToggle?.(open);
    },
  });
  showWeather();
  on("weather", showWeather);
  on("config", showWeather);

  return {
    island: pill,
    panel,
    tick(at) {
      const newDay = !sameDay(at, now);
      now = at;
      time.set({ text: clockTime(now) });
      bigTime.set({ text: clockTime(now) });
      if (newDay) {
        shortDate.set({ text: clockDate(now) });
        bigDate.set({ text: longDate(now) });
        showMonth();
      }
    },
  };
}
