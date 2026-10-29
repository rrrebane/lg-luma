//! Today's events from an iCalendar (.ics) feed, e.g. a Google Calendar "secret address".
//!
//! Pure logic with no I/O, so it can be tested without a network or a TV. The feed is parsed
//! into events and the events that fall on a given local day are worked out, expanding the
//! recurring ones. Supported: DAILY/WEEKLY/MONTHLY/YEARLY rules with INTERVAL, COUNT, UNTIL,
//! BYDAY (with ordinals such as 2TU), BYMONTHDAY, BYMONTH and WKST, plus EXDATE, moved or
//! cancelled single instances (RECURRENCE-ID), all-day and multi-day events. Not supported:
//! RDATE and the other BY* parts, which Google Calendar does not produce.
//!
//! Wall-clock times are Estonian time (Europe/Tallinn, with EU daylight saving) for events
//! that carry a TZID or no zone at all, and instants for events in UTC.

use std::collections::HashSet;
use time::{Date, Duration, Month, PrimitiveDateTime, Time, Weekday};

/// One event on the day asked for. Minutes count from local midnight of that day; an event that
/// started earlier has a negative start and one that runs on has an end past 1440.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    pub all_day: bool,
    pub start: i32,
    pub end: i32,
    pub summary: String,
    pub location: String,
}

/// A parsed feed. Parsing is the slow part, so it is done once and days are then asked for cheaply.
pub struct Feed {
    events: Vec<Event>,
    /// (uid, original start) of instances that a moved or cancelled one stands in for.
    replaced: HashSet<(String, i64)>,
}

pub fn parse_feed(ics: &str) -> Feed {
    let events = parse(ics);
    // A moved or cancelled instance of a recurring event replaces the generated one.
    let replaced = events
        .iter()
        .filter_map(|e| e.recurrence_id.map(|r| (e.uid.clone(), r.key())))
        .collect();
    Feed { events, replaced }
}

impl Feed {
    /// The events of `day` (a local date), all-day ones first, then by start.
    pub fn events_on(&self, day: Date) -> Vec<Occurrence> {
        let mut found = Vec::new();
        for event in self.events.iter().filter(|e| !e.cancelled) {
            for (start, end) in event.instances_near(day) {
                if event.rule.is_some() && self.replaced.contains(&(event.uid.clone(), start.key())) {
                    continue;
                }
                if event.exdates.iter().any(|x| x.key() == start.key()) {
                    continue;
                }
                if let Some(occurrence) = event.occurrence_on(day, start, end) {
                    found.push(occurrence);
                }
            }
        }
        found.sort_by(|a, b| {
            b.all_day
                .cmp(&a.all_day)
                .then(a.start.cmp(&b.start))
                .then_with(|| a.summary.cmp(&b.summary))
        });
        found.dedup();
        found
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.events.len()
    }
}

/// The events of `day` found in the feed text (parses the whole feed; for one-off use and tests).
#[cfg(test)]
pub fn events_on(ics: &str, day: Date) -> Vec<Occurrence> {
    parse_feed(ics).events_on(day)
}

// ---------------------------------------------------------------------------------------
// Time: Estonian time with EU daylight saving.

const HOUR: Duration = Duration::hours(1);

fn last_sunday(year: i32, month: Month) -> Date {
    let mut day = Date::from_calendar_date(year, month, 31).expect("March and October have 31 days");
    while day.weekday() != Weekday::Sunday {
        day = day.previous_day().expect("within range");
    }
    day
}

/// Summer time runs from 01:00 UTC on the last Sunday of March to 01:00 UTC on the last Sunday of October.
fn is_summer_time_utc(utc: PrimitiveDateTime) -> bool {
    let year = utc.year();
    let start = PrimitiveDateTime::new(last_sunday(year, Month::March), Time::from_hms(1, 0, 0).unwrap());
    let end = PrimitiveDateTime::new(last_sunday(year, Month::October), Time::from_hms(1, 0, 0).unwrap());
    utc >= start && utc < end
}

pub fn utc_to_local(utc: PrimitiveDateTime) -> PrimitiveDateTime {
    utc + if is_summer_time_utc(utc) { 3 * HOUR } else { 2 * HOUR }
}

/// An ambiguous wall time (the repeated hour in autumn) takes its first occurrence; a time
/// that does not exist (the skipped hour in spring) is read as just after the change.
pub fn local_to_utc(local: PrimitiveDateTime) -> PrimitiveDateTime {
    let as_summer = local - 3 * HOUR;
    if is_summer_time_utc(as_summer) {
        as_summer
    } else {
        local - 2 * HOUR
    }
}

fn add_days(date: Date, days: i64) -> Date {
    date.checked_add(Duration::days(days)).unwrap_or(date)
}

fn days_between(from: Date, to: Date) -> i64 {
    (to - from).whole_days()
}

// ---------------------------------------------------------------------------------------
// The feed: lines, properties, events.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum When {
    /// A calendar date (an all-day event).
    Day(Date),
    /// An instant, written in UTC.
    Utc(PrimitiveDateTime),
    /// A wall-clock time in Estonia (TZID=Europe/Tallinn, or a floating time).
    Local(PrimitiveDateTime),
}

impl When {
    /// A number that is equal for equal instants (and equal dates), used to compare moments.
    fn key(self) -> i64 {
        match self {
            When::Day(d) => i64::from(d.to_julian_day()) * 1440,
            When::Utc(t) => minutes(t),
            When::Local(t) => minutes(local_to_utc(t)),
        }
    }

    fn date(self) -> Date {
        match self {
            When::Day(d) => d,
            When::Utc(t) | When::Local(t) => t.date(),
        }
    }
}

fn minutes(t: PrimitiveDateTime) -> i64 {
    i64::from(t.date().to_julian_day()) * 1440 + i64::from(t.hour()) * 60 + i64::from(t.minute())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Debug, Clone)]
struct Rule {
    freq: Freq,
    interval: i64,
    count: Option<u32>,
    until: Option<When>,
    /// (ordinal, weekday): the ordinal is 0 when the rule names every such weekday.
    by_day: Vec<(i8, Weekday)>,
    by_month_day: Vec<i8>,
    by_month: Vec<u8>,
    week_start: Weekday,
}

#[derive(Debug, Clone)]
struct Event {
    uid: String,
    summary: String,
    location: String,
    start: When,
    end: Option<When>,
    rule: Option<Rule>,
    exdates: Vec<When>,
    recurrence_id: Option<When>,
    cancelled: bool,
}

fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = line.strip_prefix([' ', '\t']) {
            if let Some(last) = lines.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        lines.push(line.to_string());
    }
    lines
}

struct Property {
    name: String,
    params: Vec<(String, String)>,
    value: String,
}

fn parse_property(line: &str) -> Option<Property> {
    // NAME;PARAM=value;PARAM="quoted:value":VALUE: the first colon outside quotes ends the name part.
    let mut in_quotes = false;
    let mut split = None;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => {
                split = Some(i);
                break;
            }
            _ => {}
        }
    }
    let split = split?;
    let (head, value) = (&line[..split], &line[split + 1..]);
    let mut parts = head.split(';');
    let name = parts.next()?.to_ascii_uppercase();
    let params = parts
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.to_ascii_uppercase(), v.trim_matches('"').to_string()))
        .collect();
    Some(Property {
        name,
        params,
        value: value.to_string(),
    })
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push(' '),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_date(text: &str) -> Option<Date> {
    if text.len() < 8 || !text.is_char_boundary(8) {
        return None;
    }
    let year: i32 = text[0..4].parse().ok()?;
    let month: u8 = text[4..6].parse().ok()?;
    let day: u8 = text[6..8].parse().ok()?;
    Date::from_calendar_date(year, Month::try_from(month).ok()?, day).ok()
}

fn parse_when(property: &Property) -> Option<When> {
    let value = property.value.trim();
    let is_date = property
        .params
        .iter()
        .any(|(k, v)| k == "VALUE" && v.eq_ignore_ascii_case("DATE"))
        || (value.len() == 8 && !value.contains('T'));
    if is_date {
        return parse_date(value).map(When::Day);
    }
    let date = parse_date(value)?;
    let time_part = value.get(9..)?;
    let utc = time_part.ends_with('Z');
    let digits = time_part.trim_end_matches('Z');
    if digits.len() < 4 || !digits.is_ascii() {
        return None;
    }
    let hour: u8 = digits[0..2].parse().ok()?;
    let minute: u8 = digits[2..4].parse().ok()?;
    let second: u8 = digits.get(4..6).and_then(|s| s.parse().ok()).unwrap_or(0);
    let moment = PrimitiveDateTime::new(date, Time::from_hms(hour, minute, second).ok()?);
    Some(if utc { When::Utc(moment) } else { When::Local(moment) })
}

fn parse_weekday(text: &str) -> Option<Weekday> {
    Some(match text {
        "MO" => Weekday::Monday,
        "TU" => Weekday::Tuesday,
        "WE" => Weekday::Wednesday,
        "TH" => Weekday::Thursday,
        "FR" => Weekday::Friday,
        "SA" => Weekday::Saturday,
        "SU" => Weekday::Sunday,
        _ => return None,
    })
}

fn parse_rule(value: &str) -> Option<Rule> {
    let mut rule = Rule {
        freq: Freq::Daily,
        interval: 1,
        count: None,
        until: None,
        by_day: Vec::new(),
        by_month_day: Vec::new(),
        by_month: Vec::new(),
        week_start: Weekday::Monday,
    };
    let mut have_freq = false;
    for part in value.split(';') {
        let Some((key, val)) = part.split_once('=') else { continue };
        match key.to_ascii_uppercase().as_str() {
            "FREQ" => {
                rule.freq = match val.to_ascii_uppercase().as_str() {
                    "DAILY" => Freq::Daily,
                    "WEEKLY" => Freq::Weekly,
                    "MONTHLY" => Freq::Monthly,
                    "YEARLY" => Freq::Yearly,
                    _ => return None,
                };
                have_freq = true;
            }
            "INTERVAL" => rule.interval = val.parse::<i64>().ok().filter(|n| *n >= 1)?,
            "COUNT" => rule.count = Some(val.parse().ok()?),
            "UNTIL" => {
                rule.until = parse_when(&Property {
                    name: "UNTIL".into(),
                    params: Vec::new(),
                    value: val.to_string(),
                })
            }
            "BYDAY" => {
                for item in val.split(',') {
                    let item = item.trim();
                    if item.len() < 2 || !item.is_ascii() {
                        continue;
                    }
                    let (ordinal, day) = item.split_at(item.len() - 2);
                    let ordinal = if ordinal.is_empty() { 0 } else { ordinal.parse().ok()? };
                    rule.by_day.push((ordinal, parse_weekday(day)?));
                }
            }
            "BYMONTHDAY" => rule.by_month_day = val.split(',').filter_map(|n| n.trim().parse().ok()).collect(),
            "BYMONTH" => rule.by_month = val.split(',').filter_map(|n| n.trim().parse().ok()).collect(),
            "WKST" => rule.week_start = parse_weekday(val)?,
            _ => {}
        }
    }
    have_freq.then_some(rule)
}

fn parse(ics: &str) -> Vec<Event> {
    let mut events = Vec::new();
    let mut current: Option<Vec<Property>> = None;
    for line in unfold(ics) {
        let line = line.trim_end();
        if line.eq_ignore_ascii_case("BEGIN:VEVENT") {
            current = Some(Vec::new());
        } else if line.eq_ignore_ascii_case("END:VEVENT") {
            if let Some(properties) = current.take() {
                if let Some(event) = build_event(properties) {
                    events.push(event);
                }
            }
        } else if let Some(properties) = current.as_mut() {
            // Alarms and other nested components carry their own properties; none we need.
            if let Some(property) = parse_property(line) {
                properties.push(property);
            }
        }
    }
    events
}

fn build_event(properties: Vec<Property>) -> Option<Event> {
    let mut event = Event {
        uid: String::new(),
        summary: String::new(),
        location: String::new(),
        start: When::Day(Date::MIN),
        end: None,
        rule: None,
        exdates: Vec::new(),
        recurrence_id: None,
        cancelled: false,
    };
    let mut have_start = false;
    let mut in_alarm = false;
    for property in &properties {
        if property.name == "BEGIN" && property.value.eq_ignore_ascii_case("VALARM") {
            in_alarm = true;
        } else if property.name == "END" && property.value.eq_ignore_ascii_case("VALARM") {
            in_alarm = false;
        }
        if in_alarm {
            continue;
        }
        match property.name.as_str() {
            "UID" => event.uid = property.value.clone(),
            "SUMMARY" => event.summary = unescape(&property.value),
            "LOCATION" => event.location = unescape(&property.value),
            "STATUS" => event.cancelled = property.value.trim().eq_ignore_ascii_case("CANCELLED"),
            "DTSTART" => {
                if let Some(when) = parse_when(property) {
                    event.start = when;
                    have_start = true;
                }
            }
            "DTEND" => event.end = parse_when(property),
            "RRULE" => event.rule = parse_rule(&property.value),
            "EXDATE" => {
                for item in property.value.split(',') {
                    let single = Property {
                        name: property.name.clone(),
                        params: property.params.clone(),
                        value: item.to_string(),
                    };
                    if let Some(when) = parse_when(&single) {
                        event.exdates.push(when);
                    }
                }
            }
            "RECURRENCE-ID" => event.recurrence_id = parse_when(property),
            _ => {}
        }
    }
    have_start.then_some(event)
}

// ---------------------------------------------------------------------------------------
// Recurrence.

fn nth_weekday_matches(date: Date, ordinal: i8, weekday: Weekday) -> bool {
    if date.weekday() != weekday {
        return false;
    }
    if ordinal == 0 {
        return true;
    }
    let from_start = (i32::from(date.day()) - 1) / 7 + 1;
    let days_in_month = i32::from(date.month().length(date.year()));
    let from_end = (days_in_month - i32::from(date.day())) / 7 + 1;
    if ordinal > 0 {
        from_start == i32::from(ordinal)
    } else {
        from_end == -i32::from(ordinal)
    }
}

fn month_day_matches(date: Date, wanted: i8) -> bool {
    let days_in_month = i8::try_from(date.month().length(date.year())).unwrap_or(31);
    let day = date.day() as i8;
    if wanted > 0 {
        day == wanted
    } else {
        day == days_in_month + wanted + 1
    }
}

fn week_start_of(date: Date, week_start: Weekday) -> Date {
    let back = (i64::from(date.weekday().number_days_from_monday()) - i64::from(week_start.number_days_from_monday()) + 7) % 7;
    add_days(date, -back)
}

impl Rule {
    /// Does the rule produce an instance on `date`, given the first instance on `start`?
    /// This ignores COUNT and UNTIL, which `allows` applies.
    fn matches(&self, start: Date, date: Date) -> bool {
        if date < start {
            return false;
        }
        if !self.by_month.is_empty() && !self.by_month.contains(&u8::from(date.month())) {
            return false;
        }
        // For monthly and yearly rules: the day of the month, which defaults to the start's.
        let day_ok = || -> bool {
            if !self.by_month_day.is_empty() {
                return self.by_month_day.iter().any(|d| month_day_matches(date, *d));
            }
            if !self.by_day.is_empty() {
                return self.by_day.iter().any(|(n, wd)| nth_weekday_matches(date, *n, *wd));
            }
            date.day() == start.day()
        };
        match self.freq {
            Freq::Daily => {
                days_between(start, date) % self.interval == 0
                    && (self.by_day.is_empty() || self.by_day.iter().any(|(n, wd)| nth_weekday_matches(date, *n, *wd)))
                    && (self.by_month_day.is_empty() || self.by_month_day.iter().any(|d| month_day_matches(date, *d)))
            }
            Freq::Weekly => {
                let weeks = days_between(week_start_of(start, self.week_start), week_start_of(date, self.week_start)) / 7;
                let weekday_ok = if self.by_day.is_empty() {
                    date.weekday() == start.weekday()
                } else {
                    self.by_day.iter().any(|(_, wd)| *wd == date.weekday())
                };
                weeks % self.interval == 0 && weekday_ok
            }
            Freq::Monthly => {
                let months = i64::from(date.year() - start.year()) * 12 + i64::from(u8::from(date.month())) - i64::from(u8::from(start.month()));
                months % self.interval == 0 && day_ok()
            }
            Freq::Yearly => {
                let month_ok = !self.by_month.is_empty() || date.month() == start.month();
                i64::from(date.year() - start.year()) % self.interval == 0 && month_ok && day_ok()
            }
        }
    }

    /// COUNT and UNTIL: is the instance on `date` still part of the series?
    fn allows(&self, start: Date, date: Date) -> bool {
        if let Some(until) = self.until {
            if date > until.date() {
                return false;
            }
        }
        if let Some(count) = self.count {
            let mut seen = 0u32;
            let mut day = start;
            let mut guard = 0;
            while day <= date && guard < 40_000 {
                if self.matches(start, day) {
                    seen += 1;
                    if day == date {
                        return seen <= count;
                    }
                    if seen >= count {
                        return false;
                    }
                }
                day = add_days(day, 1);
                guard += 1;
            }
            return false;
        }
        true
    }
}

impl Event {
    /// Length of the event as a duration; all-day events last whole days (at least one).
    fn length(&self) -> Duration {
        match (self.start, self.end) {
            (When::Day(s), Some(When::Day(e))) if e > s => e - s,
            (When::Day(_), _) => Duration::days(1),
            (_, Some(end)) => Duration::minutes((end.key() - self.start.key()).max(0)),
            _ => Duration::ZERO,
        }
    }

    /// Instances of the event that might touch `day`: (start, end) pairs, as moments.
    fn instances_near(&self, day: Date) -> Vec<(When, When)> {
        let length = self.length();
        let Some(rule) = &self.rule else {
            return vec![(self.start, self.shift(self.start, length))];
        };

        // Dates are generated in the event's own frame (UTC for UTC events), so a UTC event
        // keeps its UTC time all year and moves in local time when summer time changes.
        let first = self.start.date();
        let reach = length.whole_days() + 2;
        let mut out = Vec::new();
        for offset in -reach..=1 {
            let date = add_days(day, offset);
            if !rule.matches(first, date) || !rule.allows(first, date) {
                continue;
            }
            let start = match self.start {
                When::Day(_) => When::Day(date),
                When::Utc(t) => When::Utc(PrimitiveDateTime::new(date, t.time())),
                When::Local(t) => When::Local(PrimitiveDateTime::new(date, t.time())),
            };
            out.push((start, self.shift(start, length)));
        }
        out
    }

    fn shift(&self, start: When, length: Duration) -> When {
        match start {
            When::Day(d) => When::Day(d.checked_add(length).unwrap_or(d)),
            When::Utc(t) => When::Utc(t + length),
            When::Local(t) => When::Local(t + length),
        }
    }

    /// The part of an instance that falls on `day`, if any.
    fn occurrence_on(&self, day: Date, start: When, end: When) -> Option<Occurrence> {
        let summary = if self.summary.is_empty() { "(no title)".to_string() } else { self.summary.clone() };
        if let (When::Day(s), When::Day(e)) = (start, end) {
            // An all-day event covers its start date up to, not including, its end date.
            return (s <= day && day < e).then_some(Occurrence {
                all_day: true,
                start: 0,
                end: 1440,
                summary,
                location: self.location.clone(),
            });
        }
        let to_local = |w: When| match w {
            When::Utc(t) => utc_to_local(t),
            When::Local(t) => t,
            When::Day(d) => PrimitiveDateTime::new(d, Time::MIDNIGHT),
        };
        let (s, e) = (to_local(start), to_local(end));
        let midnight = PrimitiveDateTime::new(day, Time::MIDNIGHT);
        let from_midnight = |t: PrimitiveDateTime| (t - midnight).whole_minutes() as i32;
        let (from, to) = (from_midnight(s), from_midnight(e));
        // An instant event (no end) belongs to the day it starts on.
        let overlaps = if to > from { from < 1440 && to > 0 } else { (0..1440).contains(&from) };
        overlaps.then_some(Occurrence {
            all_day: false,
            start: from,
            end: to.max(from),
            summary,
            location: self.location.clone(),
        })
    }
}

#[cfg(test)]
#[path = "calendar-tests.rs"]
mod tests;
