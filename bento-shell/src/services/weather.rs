//! The weather where the user says they are, from Open-Meteo (open-meteo.com), which needs no
//! account. Only the place's coordinates are sent.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use serde_json::{json, Value};
use soup::prelude::*;

/// How often the weather is fetched again, in seconds.
const EVERY: u32 = 20 * 60;

/// Where to fetch the weather for, and in which units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Place {
    pub latitude: f64,
    pub longitude: f64,
    pub fahrenheit: bool,
}

pub struct WeatherService {
    session: soup::Session,
    place: Cell<Option<Place>>,
    /// Goes up with each fetch, so that an older one's answer is dropped.
    fetch: Cell<u64>,
    timer: RefCell<Option<glib::SourceId>>,
    on_change: Box<dyn Fn(&Value)>,
}

impl WeatherService {
    pub fn start(on_change: impl Fn(&Value) + 'static) -> Rc<Self> {
        let session = soup::Session::new();
        session.set_user_agent("bento-shell");
        session.set_timeout(15);
        Rc::new(Self {
            session,
            place: Cell::new(None),
            fetch: Cell::new(0),
            timer: RefCell::default(),
            on_change: Box::new(on_change),
        })
    }

    /// Fetches the weather for `place` now and every so often from now on, or stops with `None`.
    pub fn set_place(self: &Rc<Self>, place: Option<Place>) {
        self.place.set(place);
        if let Some(timer) = self.timer.borrow_mut().take() {
            timer.remove();
        }
        self.fetch.set(self.fetch.get() + 1);
        if place.is_none() {
            return;
        }
        self.fetch_now();
        let weak = Rc::downgrade(self);
        let timer = glib::timeout_add_seconds_local(EVERY, move || match weak.upgrade() {
            Some(this) => {
                this.fetch_now();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
        *self.timer.borrow_mut() = Some(timer);
    }

    fn fetch_now(self: &Rc<Self>) {
        let Some(place) = self.place.get() else {
            return;
        };
        let fetch = self.fetch.get() + 1;
        self.fetch.set(fetch);
        let this = self.clone();
        glib::spawn_future_local(async move {
            let weather = this
                .get(&url(place))
                .await
                .and_then(|json| read(&json, place));
            if this.fetch.get() != fetch {
                return;
            }
            (this.on_change)(&weather.unwrap_or_else(|| json!({ "ok": false })));
        });
    }

    async fn get(&self, url: &str) -> Option<Value> {
        let message = soup::Message::new("GET", url).ok()?;
        let body = self
            .session
            .send_and_read_future(&message, glib::Priority::DEFAULT)
            .await
            .ok()?;
        if message.status() != soup::Status::Ok {
            return None;
        }
        serde_json::from_slice(&body).ok()
    }
}

fn url(place: Place) -> String {
    let units = if place.fahrenheit {
        "&temperature_unit=fahrenheit&wind_speed_unit=mph"
    } else {
        ""
    };
    format!(
        "https://api.open-meteo.com/v1/forecast?latitude={:.4}&longitude={:.4}\
         &current=temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,is_day,\
         wind_speed_10m&daily=weather_code,temperature_2m_max,temperature_2m_min\
         &timezone=auto&forecast_days=5{units}",
        place.latitude, place.longitude,
    )
}

/// What a WMO weather code is called, and its symbolic icon by day.
fn describe(code: u64) -> (&'static str, &'static str) {
    match code {
        0 => ("Clear", "weather-clear"),
        1 => ("Mostly clear", "weather-clear"),
        2 => ("Partly cloudy", "weather-few-clouds"),
        3 => ("Overcast", "weather-overcast"),
        45 | 48 => ("Fog", "weather-fog"),
        51 | 53 | 55 => ("Drizzle", "weather-showers-scattered"),
        56 | 57 => ("Freezing drizzle", "weather-showers-scattered"),
        61 | 63 | 65 => ("Rain", "weather-showers"),
        66 | 67 => ("Freezing rain", "weather-showers"),
        71 | 73 | 75 | 77 => ("Snow", "weather-snow"),
        80..=82 => ("Showers", "weather-showers"),
        85 | 86 => ("Snow showers", "weather-snow"),
        95 => ("Thunderstorm", "weather-storm"),
        96 | 99 => ("Thunderstorm with hail", "weather-storm"),
        _ => ("Unknown", "weather-overcast"),
    }
}

/// The icon's name for `code`: clear and nearly clear skies have one for the night.
fn icon(code: u64, day: bool) -> String {
    let (_, icon) = describe(code);
    let night = !day && matches!(icon, "weather-clear" | "weather-few-clouds");
    format!("{icon}{}-symbolic", if night { "-night" } else { "" })
}

/// What the shell shows, from Open-Meteo's answer.
fn read(json: &Value, place: Place) -> Option<Value> {
    let current = &json["current"];
    let code = current["weather_code"].as_u64()?;
    let day = current["is_day"].as_u64() != Some(0);
    let daily = &json["daily"];
    let days: Vec<Value> = daily["time"]
        .as_array()?
        .iter()
        .enumerate()
        .filter_map(|(i, date)| {
            let code = daily["weather_code"][i].as_u64()?;
            Some(json!({
                "date": date.as_str()?,
                "label": describe(code).0,
                "icon": icon(code, true),
                "min": daily["temperature_2m_min"][i].as_f64()?.round(),
                "max": daily["temperature_2m_max"][i].as_f64()?.round(),
            }))
        })
        .collect();
    Some(json!({
        "ok": true,
        "temperature": current["temperature_2m"].as_f64()?.round(),
        "feels": current["apparent_temperature"].as_f64().map(f64::round),
        "humidity": current["relative_humidity_2m"].as_f64(),
        "wind": current["wind_speed_10m"].as_f64().map(f64::round),
        "windUnit": if place.fahrenheit { "mph" } else { "km/h" },
        "unit": if place.fahrenheit { "°F" } else { "°C" },
        "label": describe(code).0,
        "icon": icon(code, day),
        "days": days,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLACE: Place = Place {
        latitude: 64.15,
        longitude: -21.94,
        fahrenheit: false,
    };

    #[test]
    fn it_asks_for_the_place_in_its_units() {
        let asked = url(PLACE);
        assert!(asked.contains("latitude=64.1500&longitude=-21.9400"));
        assert!(!asked.contains("fahrenheit") && !asked.contains(' '));
        let fahrenheit = Place {
            fahrenheit: true,
            ..PLACE
        };
        assert!(url(fahrenheit).contains("temperature_unit=fahrenheit"));
    }

    #[test]
    fn it_reads_now_and_the_days_ahead() {
        let answer = json!({
            "current": {
                "temperature_2m": 3.6, "apparent_temperature": -0.4,
                "relative_humidity_2m": 81, "weather_code": 1, "is_day": 0,
                "wind_speed_10m": 17.3,
            },
            "daily": {
                "time": ["2026-11-09", "2026-11-10"],
                "weather_code": [3, 71],
                "temperature_2m_max": [4.4, 1.2],
                "temperature_2m_min": [-0.6, -3.5],
            },
        });
        let weather = read(&answer, PLACE).unwrap();
        assert_eq!(weather["temperature"], 4.);
        assert_eq!(weather["feels"], 0.);
        assert_eq!(weather["label"], "Mostly clear");
        // Clear at night has its own icon; the days ahead show the day's.
        assert_eq!(weather["icon"], "weather-clear-night-symbolic");
        assert_eq!(weather["unit"], "°C");
        assert_eq!(weather["days"][0]["icon"], "weather-overcast-symbolic");
        assert_eq!(weather["days"][1]["label"], "Snow");
        assert_eq!(weather["days"][1]["min"], -4.);
        // An answer without the weather in it isn't one.
        assert_eq!(read(&json!({ "error": true }), PLACE), None);
    }
}
