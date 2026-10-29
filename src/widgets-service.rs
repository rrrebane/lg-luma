//! Fetches the widgets' data on a background thread, so DNS, TLS and parsing a large calendar
//! never block the UI (the same arrangement as the weather service).
//!
//! Error messages are deliberately generic: the calendar address is a secret, and a library
//! error that echoed the URL must never reach a log or the screen.

use crate::{
    calendar,
    widgets::{CalendarDay, Command, Timetable, parse_timetable},
};
use std::{
    io::Read,
    sync::mpsc,
    thread,
    time::Duration,
};
use time::Date;

const CALENDAR_LIMIT: usize = 8 * 1024 * 1024;
const TIMETABLE_LIMIT: usize = 512 * 1024;
/// Today and the two days after, so a feed fetched just before midnight still covers the new day.
const DAYS_AHEAD: i64 = 3;

pub enum Response {
    Calendar(Result<Vec<CalendarDay>, String>),
    Timetable(Result<Timetable, String>),
}

pub struct Service {
    sender: mpsc::Sender<Command>,
    pub receiver: mpsc::Receiver<Response>,
}

impl Service {
    pub fn new() -> Self {
        let (sender, commands) = mpsc::channel::<Command>();
        let (responses, receiver) = mpsc::channel();
        // Detached on purpose: a fetch in flight must not delay quitting.
        thread::spawn(move || {
            let agent = ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(20)))
                .max_redirects(0)
                .proxy(None)
                .build()
                .new_agent();
            for command in commands {
                let response = match command {
                    Command::Calendar { url, today } => {
                        Response::Calendar(fetch_calendar(&agent, &url, today))
                    }
                    Command::Timetable { url } => Response::Timetable(fetch_timetable(&agent, &url)),
                };
                if responses.send(response).is_err() {
                    break;
                }
            }
        });
        Self { sender, receiver }
    }

    pub fn send(&self, command: Command) {
        let _ = self.sender.send(command);
    }
}

fn download(agent: &ureq::Agent, url: &str, accept: &str, limit: usize) -> Result<Vec<u8>, &'static str> {
    // Every failure maps to a fixed message; never format the error, which may carry the URL.
    let mut response = agent
        .get(url)
        .header("Accept", accept)
        .header("User-Agent", "LumaNative")
        .call()
        .map_err(|_| "The server could not be reached")?;
    if !response.status().is_success() {
        return Err("The server did not accept the request");
    }
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "The response could not be read")?;
    if bytes.len() > limit {
        return Err("The response is too large");
    }
    Ok(bytes)
}

fn fetch_calendar(agent: &ureq::Agent, url: &str, today: Date) -> Result<Vec<CalendarDay>, String> {
    let bytes = download(agent, url, "text/calendar", CALENDAR_LIMIT)?;
    let feed = calendar::parse_feed(&String::from_utf8_lossy(&bytes));
    Ok((0..DAYS_AHEAD)
        .filter_map(|offset| today.checked_add(time::Duration::days(offset)))
        .map(|date| CalendarDay { date, events: feed.events_on(date) })
        .collect())
}

fn fetch_timetable(agent: &ureq::Agent, url: &str) -> Result<Timetable, String> {
    let bytes = download(agent, url, "application/json", TIMETABLE_LIMIT)?;
    let value = serde_json::from_slice(&bytes).map_err(|_| "The timetable could not be read")?;
    parse_timetable(&value)
}
