//! Home-screen widgets: today's calendar events and the next Elron trains from Kohila.
//!
//! The data and the "what is next" logic live here; fetching is in `widgets-service.rs` (a
//! background thread, like the weather) and drawing in `view.rs`. Both widgets are optional and
//! configured in `widgets.json` next to the preferences, which keeps the calendar's private
//! address out of the settings screens, the companion pages and the repository.
//!
//! All times are Estonian time, whatever the TV's own zone says.

use crate::calendar::{self, Occurrence};
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, UtcOffset};

/// The timetable published by this fork's scheduled job (public timetable data only).
pub const DEFAULT_TIMETABLE_URL: &str =
    "https://raw.githubusercontent.com/rrrebane/lg-luma/widget-data/elron-kohila.json";
const REFRESH_SECONDS: i64 = 15 * 60;
const RETRY_SECONDS: i64 = 60;
const CONFIG_CHECK_SECONDS: i64 = 30;

// ---------------------------------------------------------------------------------------
// Configuration.

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Google Calendar's "secret address in iCal format". Never logged, shown or sent anywhere
    /// but to that address.
    pub calendar_url: Option<String>,
    /// Where to download the Kohila timetable from; the fork's published file by default.
    pub timetable_url: Option<String>,
    /// Set to false to hide the train widget.
    pub show_trains: Option<bool>,
}

fn https(url: &str) -> Option<&str> {
    let url = url.trim();
    url.starts_with("https://").then_some(url)
}

impl Config {
    pub fn calendar_url(&self) -> Option<&str> {
        self.calendar_url.as_deref().and_then(https)
    }

    pub fn timetable_url(&self) -> Option<&str> {
        if self.show_trains == Some(false) {
            return None;
        }
        match self.timetable_url.as_deref() {
            Some(custom) => https(custom),
            None => Some(DEFAULT_TIMETABLE_URL),
        }
    }
}

pub fn config_path(preferences: &Path) -> PathBuf {
    preferences.with_file_name("widgets.json")
}

/// A missing or unreadable file means no calendar and the default train widget.
pub fn load_config(path: &Path) -> Config {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------------------
// Time.

/// Estonian date and minutes after midnight for an instant.
pub fn estonian_now(now: OffsetDateTime) -> (Date, i32) {
    let utc = now.to_offset(UtcOffset::UTC);
    let local = calendar::utc_to_local(PrimitiveDateTime::new(utc.date(), utc.time()));
    (local.date(), i32::from(local.hour()) * 60 + i32::from(local.minute()))
}

/// "07:05" for minutes after midnight; times past midnight wrap round.
pub fn clock(minutes: i32) -> String {
    let wrapped = minutes.rem_euclid(1440);
    format!("{:02}:{:02}", wrapped / 60, wrapped % 60)
}

pub fn weekday_name(date: Date) -> &'static str {
    ["esmaspäev", "teisipäev", "kolmapäev", "neljapäev", "reede", "laupäev", "pühapäev"]
        [date.weekday().number_days_from_monday() as usize]
}

// ---------------------------------------------------------------------------------------
// Data.

#[derive(Debug, Clone, PartialEq)]
pub struct CalendarDay {
    pub date: Date,
    pub events: Vec<Occurrence>,
}

/// One train from Kohila with its arrival times (minutes after midnight of the service day;
/// they can pass 1440 for trains running after midnight).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Departure {
    pub depart: i32,
    pub vaike: Option<i32>,
    pub tallinn: Option<i32>,
    pub line: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Timetable {
    pub days: Vec<(Date, Vec<Departure>)>,
}

/// A departure that has not left yet; `abs` counts from midnight of today, so it is 1440 or more
/// for trains on a later day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upcoming {
    pub abs: i32,
    pub departure: Departure,
}

impl Upcoming {
    pub fn later_day(&self) -> bool {
        self.abs >= 1440
    }

    pub fn minutes_until(&self, now_minutes: i32) -> i32 {
        self.abs - now_minutes
    }
}

fn parse_iso_date(text: &str) -> Option<Date> {
    let mut parts = text.splitn(3, '-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u8 = parts.next()?.parse().ok()?;
    let day: u8 = parts.next()?.parse().ok()?;
    Date::from_calendar_date(year, Month::try_from(month).ok()?, day).ok()
}

/// Reads the file the scheduled job publishes (see `tools/elron-kohila/build.py`).
pub fn parse_timetable(value: &Value) -> Result<Timetable, String> {
    let days = value
        .get("days")
        .and_then(Value::as_object)
        .ok_or("The timetable has no days")?;
    let minutes = |item: &Value, key: &str| {
        item.get(key)
            .and_then(Value::as_i64)
            .and_then(|m| i32::try_from(m).ok())
    };
    let mut out = Vec::new();
    for (key, list) in days {
        let (Some(date), Some(items)) = (parse_iso_date(key), list.as_array()) else {
            continue;
        };
        let mut departures: Vec<Departure> = items
            .iter()
            .filter_map(|item| {
                Some(Departure {
                    depart: minutes(item, "d")?,
                    vaike: minutes(item, "w"),
                    tallinn: minutes(item, "t"),
                    line: item.get("l").and_then(Value::as_str).unwrap_or("").to_owned(),
                })
            })
            .collect();
        departures.sort_by_key(|d| d.depart);
        out.push((date, departures));
    }
    out.sort_by_key(|(date, _)| *date);
    if out.is_empty() {
        return Err("The timetable has no usable days".into());
    }
    Ok(Timetable { days: out })
}

impl Timetable {
    /// The next `limit` departures from `now_minutes` after midnight of `today`, running on into
    /// the following days when today's trains are over.
    pub fn upcoming(&self, today: Date, now_minutes: i32, limit: usize) -> Vec<Upcoming> {
        let mut found = Vec::new();
        for (date, departures) in &self.days {
            let offset = (*date - today).whole_days();
            if !(0..=2).contains(&offset) {
                continue;
            }
            for departure in departures {
                let abs = departure.depart + offset as i32 * 1440;
                if abs >= now_minutes {
                    found.push(Upcoming { abs, departure: departure.clone() });
                }
            }
        }
        found.sort_by_key(|u| u.abs);
        found.truncate(limit);
        found
    }
}

// ---------------------------------------------------------------------------------------
// State.

#[derive(Debug, Clone, PartialEq)]
pub struct CalendarData {
    pub days: Vec<CalendarDay>,
}

#[derive(Default)]
pub struct Widgets {
    pub config: Config,
    pub calendar: Option<CalendarData>,
    pub timetable: Option<Timetable>,
    pub calendar_failed: bool,
    pub timetable_failed: bool,
    calendar_pending: bool,
    timetable_pending: bool,
    next_calendar: i64,
    next_timetable: i64,
    next_config_check: i64,
}

/// What to fetch; built by `Widgets::sync` and run by the service.
pub enum Command {
    Calendar { url: String, today: Date },
    Timetable { url: String },
}

impl Widgets {
    pub fn calendar_enabled(&self) -> bool {
        self.config.calendar_url().is_some()
    }

    pub fn trains_enabled(&self) -> bool {
        self.config.timetable_url().is_some()
    }

    /// Picks up an edited `widgets.json`. Changing an address starts again for that widget.
    pub fn set_config(&mut self, config: Config) -> bool {
        if config == self.config {
            return false;
        }
        if config.calendar_url() != self.config.calendar_url() {
            self.calendar = None;
            self.calendar_failed = false;
            self.next_calendar = 0;
        }
        if config.timetable_url() != self.config.timetable_url() {
            self.timetable = None;
            self.timetable_failed = false;
            self.next_timetable = 0;
        }
        self.config = config;
        true
    }

    /// True now and then, when `widgets.json` is worth reading again.
    pub fn config_check_due(&mut self, now: i64) -> bool {
        if now < self.next_config_check {
            return false;
        }
        self.next_config_check = now + CONFIG_CHECK_SECONDS;
        true
    }

    /// The fetches that are due. Nothing is fetched while Home is not showing.
    pub fn sync(&mut self, home_visible: bool, now: i64, today: Date) -> Vec<Command> {
        let mut commands = Vec::new();
        if !home_visible {
            return commands;
        }
        if let Some(url) = self.config.calendar_url() {
            if !self.calendar_pending && now >= self.next_calendar {
                self.calendar_pending = true;
                commands.push(Command::Calendar { url: url.to_owned(), today });
            }
        }
        if let Some(url) = self.config.timetable_url() {
            if !self.timetable_pending && now >= self.next_timetable {
                self.timetable_pending = true;
                commands.push(Command::Timetable { url: url.to_owned() });
            }
        }
        commands
    }

    pub fn accept_calendar(&mut self, result: Result<Vec<CalendarDay>, String>, now: i64) {
        self.calendar_pending = false;
        match result {
            Ok(days) => {
                self.calendar = Some(CalendarData { days });
                self.calendar_failed = false;
                self.next_calendar = now + REFRESH_SECONDS;
            }
            Err(_) => {
                self.calendar_failed = true;
                self.next_calendar = now + RETRY_SECONDS;
            }
        }
    }

    pub fn accept_timetable(&mut self, result: Result<Timetable, String>, now: i64) {
        self.timetable_pending = false;
        match result {
            Ok(table) => {
                self.timetable = Some(table);
                self.timetable_failed = false;
                self.next_timetable = now + REFRESH_SECONDS;
            }
            Err(_) => {
                self.timetable_failed = true;
                self.next_timetable = now + RETRY_SECONDS;
            }
        }
    }

    /// Today's events, or None until a feed that covers today has arrived.
    pub fn events_today(&self, today: Date) -> Option<&[Occurrence]> {
        self.calendar
            .as_ref()?
            .days
            .iter()
            .find(|day| day.date == today)
            .map(|day| day.events.as_slice())
    }
}

#[cfg(test)]
#[path = "widgets-tests.rs"]
mod tests;
