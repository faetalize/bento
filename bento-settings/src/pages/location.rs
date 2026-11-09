//! Where the user is, which the shell's weather is for: a place found by its name, or its
//! coordinates typed in. The search asks Open-Meteo (open-meteo.com), which the weather comes
//! from too, and sends it only what's typed.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use soup::prelude::*;

use crate::kdl::Value;
use crate::model::{BoolStyle, Category, Group, Setting};
use crate::page::{keep_alive, PageContext};

const KEY: &str = "location";

pub fn category() -> Category {
    Category::settings("location", "Location")
        .icon("ph-map-pin-symbolic")
        .keywords(&["weather", "city", "place", "where", "forecast"])
        .group(Group::custom(build).search_item(
            "location.place",
            "Place",
            &["city", "weather", "search"],
        ))
        .group(
            Group::new("Coordinates")
                .setting(
                    Setting::number("Latitude", "location.latitude", |c| {
                        c.location.latitude.map_or(0., |v| v.0)
                    })
                    .range(-90., 90., 0.01)
                    .digits(4)
                    .keywords(&["north", "south"]),
                )
                .setting(
                    Setting::number("Longitude", "location.longitude", |c| {
                        c.location.longitude.map_or(0., |v| v.0)
                    })
                    .range(-180., 180., 0.01)
                    .digits(4)
                    .keywords(&["east", "west"]),
                ),
        )
        .group(
            Group::new("Weather").setting(
                Setting::switch("Fahrenheit", "shell.fahrenheit", |c| {
                    c.shell.as_ref().is_some_and(|shell| shell.fahrenheit)
                })
                .style(BoolStyle::Presence)
                .keywords(&["celsius", "temperature", "units", "degrees"]),
            ),
        )
}

/// A place the search found.
#[derive(Debug, Clone, PartialEq)]
struct Place {
    name: String,
    /// Its region and country, to tell places of one name apart.
    region: String,
    latitude: f64,
    longitude: f64,
}

struct State {
    ctx: PageContext,
    group: adw::PreferencesGroup,
    current: adw::ActionRow,
    clear: gtk::Button,
    session: soup::Session,
    /// The rows of what the last search found.
    results: RefCell<Vec<gtk::Widget>>,
}

fn build(ctx: &PageContext) -> gtk::Widget {
    let group = adw::PreferencesGroup::builder()
        .title("Place")
        .description(
            "The shell shows the weather here. The place's coordinates go to Open-Meteo, which \
             the weather comes from.",
        )
        .build();

    let current = adw::ActionRow::builder().title("Current Place").build();
    ctx.anchor("location.place", &current);
    let clear = gtk::Button::builder()
        .label("Clear")
        .valign(gtk::Align::Center)
        .build();
    current.add_suffix(&clear);
    group.add(&current);

    let search = adw::EntryRow::builder()
        .title("Search for a City")
        .show_apply_button(true)
        .build();
    group.add(&search);

    let session = soup::Session::new();
    session.set_user_agent("bento-settings");
    session.set_timeout(15);
    let state = Rc::new(State {
        ctx: ctx.clone(),
        group: group.clone(),
        current,
        clear: clear.clone(),
        session,
        results: RefCell::default(),
    });
    keep_alive(&group, state.clone());

    state.show();
    let weak = Rc::downgrade(&state);
    ctx.store.subscribe(move || {
        if let Some(state) = weak.upgrade() {
            state.show();
        }
    });
    let weak = Rc::downgrade(&state);
    clear.connect_clicked(move |_| {
        if let Some(state) = weak.upgrade() {
            state.ctx.store.edit(KEY, |doc| {
                doc.remove(&["location"]);
            });
        }
    });
    let weak = Rc::downgrade(&state);
    search.connect_apply(move |row| {
        if let Some(state) = weak.upgrade() {
            state.search(row.text().trim().to_owned());
        }
    });
    group.upcast()
}

impl State {
    /// Shows the place the config has.
    fn show(&self) {
        let config = self.ctx.store.config();
        let location = &config.location;
        let set = location.latitude.is_some() && location.longitude.is_some();
        let name = match (&location.name, set) {
            (Some(name), true) => name.clone(),
            (None, true) => String::from("Set by its coordinates"),
            (_, false) => String::from("Not set"),
        };
        self.current.set_subtitle(&name);
        self.clear.set_visible(set || location.name.is_some());
    }

    fn search(self: &Rc<Self>, query: String) {
        if query.is_empty() {
            return;
        }
        let this = self.clone();
        gtk::glib::spawn_future_local(async move {
            let found = this.find(&query).await;
            this.show_results(found);
        });
    }

    /// Asks Open-Meteo for the places called `query`.
    async fn find(&self, query: &str) -> Option<Vec<Place>> {
        let name = gtk::glib::Uri::escape_string(query, None, true);
        let url = format!(
            "https://geocoding-api.open-meteo.com/v1/search?name={name}&count=6&language=en&format=json"
        );
        let message = soup::Message::new("GET", &url).ok()?;
        let body = self
            .session
            .send_and_read_future(&message, gtk::glib::Priority::DEFAULT)
            .await
            .ok()?;
        if message.status() != soup::Status::Ok {
            return None;
        }
        Some(places(&serde_json::from_slice(&body).ok()?))
    }

    fn show_results(self: &Rc<Self>, found: Option<Vec<Place>>) {
        for row in self.results.borrow_mut().drain(..) {
            self.group.remove(&row);
        }
        let places = match found {
            Some(places) if !places.is_empty() => places,
            Some(_) => return self.ctx.toast("No place by that name"),
            None => return self.ctx.toast("Couldn't search for places"),
        };
        for place in places {
            let row = adw::ActionRow::builder()
                .title(gtk::glib::markup_escape_text(&place.name))
                .subtitle(gtk::glib::markup_escape_text(&place.region))
                .activatable(true)
                .build();
            row.add_suffix(&gtk::Image::from_icon_name("ph-caret-right-symbolic"));
            let weak = Rc::downgrade(self);
            row.connect_activated(move |_| {
                if let Some(state) = weak.upgrade() {
                    state.choose(place.clone());
                }
            });
            self.group.add(&row);
            self.results.borrow_mut().push(row.upcast());
        }
    }

    fn choose(&self, place: Place) {
        let label = if place.region.is_empty() {
            place.name.clone()
        } else {
            format!("{}, {}", place.name, place.region)
        };
        self.ctx.toast(&format!("Location set to {label}"));
        self.ctx.store.edit(KEY, move |doc| {
            doc.set(&["location", "name"], vec![Value::Str(label)]);
            doc.set(
                &["location", "latitude"],
                vec![Value::Float(round(place.latitude))],
            );
            doc.set(
                &["location", "longitude"],
                vec![Value::Float(round(place.longitude))],
            );
        });
        for row in self.results.borrow_mut().drain(..) {
            self.group.remove(&row);
        }
    }
}

/// To four decimals, about ten metres: more says nothing about the weather.
fn round(degrees: f64) -> f64 {
    (degrees * 10_000.).round() / 10_000.
}

/// The places in Open-Meteo's answer to a search.
fn places(json: &serde_json::Value) -> Vec<Place> {
    let Some(results) = json["results"].as_array() else {
        return Vec::new();
    };
    results
        .iter()
        .filter_map(|result| {
            let region = [&result["admin1"], &result["country"]]
                .into_iter()
                .filter_map(|part| part.as_str())
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(", ");
            Some(Place {
                name: result["name"].as_str()?.to_owned(),
                region,
                latitude: result["latitude"].as_f64()?,
                longitude: result["longitude"].as_f64()?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_reads_the_places_a_search_finds() {
        let answer = serde_json::json!({
            "results": [
                {
                    "name": "Reykjavik", "latitude": 64.13548, "longitude": -21.89541,
                    "country": "Iceland", "admin1": "Capital Region",
                },
                { "name": "Nowhere", "latitude": 1.5 },
                { "name": "Tórshavn", "latitude": 62.00973, "longitude": -6.77164 },
            ],
        });
        let found = places(&answer);
        assert_eq!(found.len(), 2, "one without coordinates isn't a place");
        assert_eq!(found[0].name, "Reykjavik");
        assert_eq!(found[0].region, "Capital Region, Iceland");
        assert_eq!(found[1].region, "");
        assert_eq!(round(found[0].latitude), 64.1355);
        // Nothing found comes without the list.
        assert!(places(&serde_json::json!({ "generationtime_ms": 0.3 })).is_empty());
    }
}
