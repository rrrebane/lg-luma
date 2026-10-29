use super::*;
use serde_json::json;
use time::Month;

fn date(year: i32, month: u8, day: u8) -> Date {
    Date::from_calendar_date(year, Month::try_from(month).unwrap(), day).unwrap()
}

fn train(depart: i32, vaike: Option<i32>, tallinn: Option<i32>, line: &str) -> Departure {
    Departure {
        depart,
        vaike,
        tallinn,
        line: line.into(),
    }
}

fn table() -> Timetable {
    Timetable {
        days: vec![
            (
                date(2026, 10, 29),
                vec![
                    train(369, Some(403), Some(409), "R21"),
                    train(404, Some(438), Some(444), "R22"),
                    train(1290, Some(1324), Some(1330), "R21"),
                    train(1451, None, Some(1490), "R23"), // 00:11 after midnight, on today's service day
                ],
            ),
            (
                date(2026, 10, 30),
                vec![
                    train(369, Some(403), Some(409), "R21"),
                    train(404, None, Some(444), "R22"),
                ],
            ),
        ],
    }
}

#[test]
fn timetable_file_is_parsed_and_sorted() {
    let value = json!({
        "v": 1,
        "days": {
            "2026-10-30": [{"d": 404, "l": "R22", "t": 444}, {"d": 369, "l": "R21", "w": 403, "t": 409}],
            "2026-10-29": [{"d": 100, "l": "R21", "w": 130}],
            "not-a-date": [{"d": 1}],
        }
    });
    let parsed = parse_timetable(&value).unwrap();
    assert_eq!(parsed.days.len(), 2);
    assert_eq!(parsed.days[0].0, date(2026, 10, 29));
    let second: Vec<_> = parsed.days[1].1.iter().map(|d| d.depart).collect();
    assert_eq!(second, vec![369, 404]);
    assert_eq!(parsed.days[1].1[1].vaike, None);
    assert_eq!(parsed.days[1].1[1].tallinn, Some(444));
}

#[test]
fn unusable_timetables_are_rejected() {
    assert!(parse_timetable(&json!({})).is_err());
    assert!(parse_timetable(&json!({"days": {}})).is_err());
    assert!(parse_timetable(&json!({"days": {"2026-10-29": "no"}})).is_err());
    assert!(parse_timetable(&json!({"days": {"2026-13-40": []}})).is_err());
}

#[test]
fn upcoming_lists_the_next_trains_in_order() {
    // 06:30: the 06:09 has gone, the 06:44 is next.
    let found = table().upcoming(date(2026, 10, 29), 6 * 60 + 30, 3);
    let times: Vec<_> = found.iter().map(|u| u.abs).collect();
    assert_eq!(times, vec![404, 1290, 1451]);
    assert!(!found[0].later_day());
    assert!(found[2].later_day());
    assert_eq!(found[0].minutes_until(6 * 60 + 30), 14);
}

#[test]
fn a_train_leaving_this_minute_is_still_listed() {
    let found = table().upcoming(date(2026, 10, 29), 404, 1);
    assert_eq!(found[0].abs, 404);
    assert_eq!(found[0].minutes_until(404), 0);
}

#[test]
fn after_the_last_train_the_list_runs_into_tomorrow() {
    let found = table().upcoming(date(2026, 10, 29), 22 * 60, 4);
    let times: Vec<_> = found.iter().map(|u| u.abs).collect();
    // 21:30 has gone; today's 00:11 train, then tomorrow's 06:09 and 06:44 (as 1440 + minutes).
    assert_eq!(times, vec![1451, 1440 + 369, 1440 + 404]);
    assert_eq!(clock(found[0].abs), "00:11");
    assert_eq!(clock(found[1].abs), "06:09");
}

#[test]
fn days_before_today_and_far_ahead_are_ignored() {
    assert!(table().upcoming(date(2026, 11, 5), 0, 5).is_empty());
    assert!(table().upcoming(date(2026, 10, 31), 0, 5).is_empty());
}

#[test]
fn clock_wraps_past_midnight() {
    assert_eq!(clock(0), "00:00");
    assert_eq!(clock(369), "06:09");
    assert_eq!(clock(1451), "00:11");
    assert_eq!(clock(1440 + 369), "06:09");
    assert_eq!(clock(-60), "23:00");
}

#[test]
fn weekdays_are_in_estonian() {
    assert_eq!(weekday_name(date(2026, 10, 29)), "neljapäev");
    assert_eq!(weekday_name(date(2026, 11, 1)), "pühapäev");
    assert_eq!(weekday_name(date(2026, 11, 2)), "esmaspäev");
}

#[test]
fn estonian_time_does_not_depend_on_the_tvs_own_zone() {
    // 22:30 UTC on 29 Oct is 00:30 on the 30th in Estonia (winter time), however it is written.
    let utc = OffsetDateTime::new_utc(date(2026, 10, 29), time::Time::from_hms(22, 30, 0).unwrap());
    assert_eq!(estonian_now(utc), (date(2026, 10, 30), 30));
    let shifted = utc.to_offset(UtcOffset::from_hms(-5, 0, 0).unwrap());
    assert_eq!(estonian_now(shifted), (date(2026, 10, 30), 30));
    // In summer it is UTC+3.
    let summer =
        OffsetDateTime::new_utc(date(2026, 7, 1), time::Time::from_hms(21, 30, 0).unwrap());
    assert_eq!(estonian_now(summer), (date(2026, 7, 2), 30));
}

#[test]
fn only_https_addresses_are_accepted() {
    let config = Config {
        calendar_url: Some("  https://example.test/secret.ics ".into()),
        ..Config::default()
    };
    assert_eq!(
        config.calendar_url(),
        Some("https://example.test/secret.ics")
    );
    for bad in [
        "http://example.test/a.ics",
        "file:///etc/passwd",
        "",
        "example.test",
    ] {
        let config = Config {
            calendar_url: Some(bad.into()),
            ..Config::default()
        };
        assert_eq!(config.calendar_url(), None, "{bad}");
    }
}

#[test]
fn the_train_widget_defaults_on_and_can_be_turned_off_or_redirected() {
    assert_eq!(
        Config::default().timetable_url(),
        Some(DEFAULT_TIMETABLE_URL)
    );
    let off = Config {
        show_trains: Some(false),
        ..Config::default()
    };
    assert_eq!(off.timetable_url(), None);
    let custom = Config {
        timetable_url: Some("https://example.test/t.json".into()),
        ..Config::default()
    };
    assert_eq!(custom.timetable_url(), Some("https://example.test/t.json"));
    let insecure = Config {
        timetable_url: Some("http://example.test/t.json".into()),
        ..Config::default()
    };
    assert_eq!(insecure.timetable_url(), None);
}

#[test]
fn config_file_is_read_leniently() {
    let dir = std::env::temp_dir().join(format!("luma-widgets-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("widgets.json");
    assert_eq!(load_config(&path), Config::default()); // missing
    std::fs::write(&path, b"{ not json").unwrap();
    assert_eq!(load_config(&path), Config::default()); // broken
    std::fs::write(
        &path,
        br#"{"calendar_url": "https://example.test/a.ics", "unknown": 1}"#,
    )
    .unwrap();
    assert_eq!(
        load_config(&path).calendar_url(),
        Some("https://example.test/a.ics")
    );
    assert_eq!(
        config_path(Path::new("/x/luma-native/preferences.json")),
        PathBuf::from("/x/luma-native/widgets.json")
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

fn configured() -> Widgets {
    let mut widgets = Widgets::default();
    widgets.set_config(Config {
        calendar_url: Some("https://example.test/a.ics".into()),
        ..Config::default()
    });
    widgets
}

#[test]
fn nothing_is_fetched_while_home_is_hidden() {
    let mut widgets = configured();
    assert!(widgets.sync(false, 1000, date(2026, 10, 29)).is_empty());
}

#[test]
fn both_fetches_start_once_and_not_again_while_pending() {
    let mut widgets = configured();
    let first = widgets.sync(true, 1000, date(2026, 10, 29));
    assert_eq!(first.len(), 2);
    assert!(widgets.sync(true, 1001, date(2026, 10, 29)).is_empty());
}

#[test]
fn success_waits_a_quarter_of_an_hour_and_failure_a_minute() {
    let mut widgets = configured();
    widgets.sync(true, 1000, date(2026, 10, 29));
    widgets.accept_calendar(Ok(vec![]), 1010);
    widgets.accept_timetable(Err("down".into()), 1010);
    assert!(!widgets.calendar_failed);
    assert!(widgets.timetable_failed);
    // 59 s after the failed timetable nothing is due; at 60 s only it is retried.
    assert!(widgets.sync(true, 1069, date(2026, 10, 29)).is_empty());
    let retry = widgets.sync(true, 1070, date(2026, 10, 29));
    assert_eq!(retry.len(), 1);
    assert!(matches!(retry[0], Command::Timetable { .. }));
    widgets.accept_timetable(Ok(table()), 1075);
    // The calendar comes round again 15 minutes after its success.
    let again = widgets.sync(true, 1010 + 15 * 60, date(2026, 10, 29));
    assert_eq!(again.len(), 1);
    assert!(matches!(again[0], Command::Calendar { .. }));
}

#[test]
fn without_a_calendar_address_only_the_trains_are_fetched() {
    let mut widgets = Widgets::default();
    let commands = widgets.sync(true, 1000, date(2026, 10, 29));
    assert_eq!(commands.len(), 1);
    assert!(matches!(commands[0], Command::Timetable { .. }));
    assert!(!widgets.calendar_enabled());
    assert!(widgets.trains_enabled());
}

#[test]
fn changing_the_calendar_address_forgets_the_old_events() {
    let mut widgets = configured();
    widgets.sync(true, 1000, date(2026, 10, 29));
    widgets.accept_calendar(
        Ok(vec![CalendarDay {
            date: date(2026, 10, 29),
            events: vec![],
        }]),
        1001,
    );
    assert!(widgets.events_today(date(2026, 10, 29)).is_some());
    let same = widgets.config.clone();
    assert!(!widgets.set_config(same)); // unchanged: nothing happens
    assert!(widgets.events_today(date(2026, 10, 29)).is_some());
    widgets.set_config(Config {
        calendar_url: Some("https://example.test/b.ics".into()),
        ..Config::default()
    });
    assert!(widgets.events_today(date(2026, 10, 29)).is_none());
    // The calendar is fetched at once; the trains are still pending from the first sync.
    let commands = widgets.sync(true, 1002, date(2026, 10, 29));
    assert_eq!(commands.len(), 1);
    assert!(matches!(commands[0], Command::Calendar { .. }));
}

#[test]
fn events_are_only_shown_for_a_day_the_feed_covers() {
    let mut widgets = configured();
    assert!(widgets.events_today(date(2026, 10, 29)).is_none());
    let event = Occurrence {
        all_day: false,
        start: 540,
        end: 600,
        summary: "Stand-up".into(),
        location: String::new(),
    };
    widgets.accept_calendar(
        Ok(vec![
            CalendarDay {
                date: date(2026, 10, 29),
                events: vec![event.clone()],
            },
            CalendarDay {
                date: date(2026, 10, 30),
                events: vec![],
            },
        ]),
        1000,
    );
    assert_eq!(widgets.events_today(date(2026, 10, 29)), Some(&[event][..]));
    assert_eq!(widgets.events_today(date(2026, 10, 30)), Some(&[][..]));
    assert!(widgets.events_today(date(2026, 11, 3)).is_none()); // stale feed, missing day
}

#[test]
fn the_config_file_is_checked_every_half_minute() {
    let mut widgets = Widgets::default();
    assert!(widgets.config_check_due(100));
    assert!(!widgets.config_check_due(120));
    assert!(widgets.config_check_due(130));
}
