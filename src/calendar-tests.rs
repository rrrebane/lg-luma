use super::*;
use time::Month;

fn date(year: i32, month: u8, day: u8) -> Date {
    Date::from_calendar_date(year, Month::try_from(month).unwrap(), day).unwrap()
}

fn local(year: i32, month: u8, day: u8, hour: u8, minute: u8) -> PrimitiveDateTime {
    PrimitiveDateTime::new(
        date(year, month, day),
        Time::from_hms(hour, minute, 0).unwrap(),
    )
}

fn feed(events: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{}\r\nEND:VCALENDAR\r\n",
        events.trim().replace('\n', "\r\n")
    )
}

fn on(events: &str, day: Date) -> Vec<Occurrence> {
    events_on(&feed(events), day)
}

fn summaries(found: &[Occurrence]) -> Vec<&str> {
    found.iter().map(|o| o.summary.as_str()).collect()
}

// 2026-10-29 is a Thursday. Summer time ends on Sunday 2026-10-25 and began on 2026-03-29.

#[test]
fn utc_event_is_shown_in_estonian_time() {
    let found = on(
        "BEGIN:VEVENT\nUID:a\nSUMMARY:Standup\nDTSTART:20261029T070000Z\nDTEND:20261029T083000Z\nEND:VEVENT",
        date(2026, 10, 29),
    );
    // Winter time (UTC+2): 09:00 to 10:30.
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].start, found[0].end), (9 * 60, 10 * 60 + 30));
    assert!(!found[0].all_day);
}

#[test]
fn events_on_other_days_are_not_returned() {
    let event = "BEGIN:VEVENT\nUID:a\nSUMMARY:Standup\nDTSTART:20261029T070000Z\nDTEND:20261029T083000Z\nEND:VEVENT";
    assert!(on(event, date(2026, 10, 28)).is_empty());
    assert!(on(event, date(2026, 10, 30)).is_empty());
}

#[test]
fn utc_event_just_before_midnight_lands_on_the_next_local_day() {
    // 22:30 UTC is 00:30 the next day in Estonia.
    let event = "BEGIN:VEVENT\nUID:a\nSUMMARY:Late\nDTSTART:20261028T223000Z\nDTEND:20261028T233000Z\nEND:VEVENT";
    assert!(on(event, date(2026, 10, 28)).is_empty());
    assert_eq!(on(event, date(2026, 10, 29))[0].start, 30);
}

#[test]
fn all_day_event_covers_its_start_date_up_to_the_end_date() {
    let event = "BEGIN:VEVENT\nUID:a\nSUMMARY:Holiday\nDTSTART;VALUE=DATE:20261028\nDTEND;VALUE=DATE:20261031\nEND:VEVENT";
    for day in 28..=30 {
        let found = on(event, date(2026, 10, day));
        assert_eq!(found.len(), 1, "day {day}");
        assert!(found[0].all_day);
    }
    assert!(on(event, date(2026, 10, 27)).is_empty());
    assert!(on(event, date(2026, 10, 31)).is_empty());
}

#[test]
fn all_day_event_without_end_lasts_one_day() {
    let event = "BEGIN:VEVENT\nUID:a\nSUMMARY:Name day\nDTSTART;VALUE=DATE:20261029\nEND:VEVENT";
    assert_eq!(on(event, date(2026, 10, 29)).len(), 1);
    assert!(on(event, date(2026, 10, 30)).is_empty());
}

#[test]
fn overnight_event_shows_on_both_days() {
    let event = "BEGIN:VEVENT\nUID:a\nSUMMARY:Party\nDTSTART;TZID=Europe/Tallinn:20261029T230000\nDTEND;TZID=Europe/Tallinn:20261030T010000\nEND:VEVENT";
    let first = on(event, date(2026, 10, 29));
    assert_eq!((first[0].start, first[0].end), (23 * 60, 25 * 60));
    let second = on(event, date(2026, 10, 30));
    assert_eq!((second[0].start, second[0].end), (-60, 60));
}

#[test]
fn event_without_end_belongs_to_its_start_day() {
    let event = "BEGIN:VEVENT\nUID:a\nSUMMARY:Reminder\nDTSTART;TZID=Europe/Tallinn:20261029T120000\nEND:VEVENT";
    let found = on(event, date(2026, 10, 29));
    assert_eq!((found[0].start, found[0].end), (720, 720));
    assert!(on(event, date(2026, 10, 30)).is_empty());
}

#[test]
fn weekly_rule_with_weekdays_and_interval() {
    let event = "BEGIN:VEVENT\nUID:a\nSUMMARY:Gym\nDTSTART;TZID=Europe/Tallinn:20261005T180000\nDTEND;TZID=Europe/Tallinn:20261005T190000\nRRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE\nEND:VEVENT";
    assert_eq!(on(event, date(2026, 10, 5)).len(), 1); // first Monday
    assert_eq!(on(event, date(2026, 10, 7)).len(), 1); // Wednesday of the same week
    assert!(on(event, date(2026, 10, 12)).is_empty()); // the skipped week
    assert!(on(event, date(2026, 10, 14)).is_empty());
    assert_eq!(on(event, date(2026, 10, 19)).len(), 1);
    assert_eq!(on(event, date(2026, 10, 21)).len(), 1);
    assert!(on(event, date(2026, 10, 20)).is_empty()); // a Tuesday
    assert!(on(event, date(2026, 10, 4)).is_empty()); // before the start
}

#[test]
fn weekly_rule_defaults_to_the_start_weekday() {
    let event = "BEGIN:VEVENT\nUID:a\nSUMMARY:Call\nDTSTART;TZID=Europe/Tallinn:20261001T090000\nRRULE:FREQ=WEEKLY\nEND:VEVENT"; // a Thursday
    assert_eq!(on(event, date(2026, 10, 29)).len(), 1);
    assert!(on(event, date(2026, 10, 28)).is_empty());
}

#[test]
fn daily_rule_with_interval() {
    let event = "BEGIN:VEVENT\nUID:a\nSUMMARY:Pills\nDTSTART;TZID=Europe/Tallinn:20261027T080000\nRRULE:FREQ=DAILY;INTERVAL=3\nEND:VEVENT";
    assert_eq!(on(event, date(2026, 10, 27)).len(), 1);
    assert!(on(event, date(2026, 10, 28)).is_empty());
    assert_eq!(on(event, date(2026, 10, 30)).len(), 1);
}

#[test]
fn exdate_removes_one_instance() {
    let event = "BEGIN:VEVENT\nUID:a\nSUMMARY:Class\nDTSTART;TZID=Europe/Tallinn:20261005T100000\nRRULE:FREQ=WEEKLY\nEXDATE;TZID=Europe/Tallinn:20261026T100000\nEND:VEVENT";
    assert_eq!(on(event, date(2026, 10, 19)).len(), 1);
    assert!(on(event, date(2026, 10, 26)).is_empty());
    assert_eq!(on(event, date(2026, 11, 2)).len(), 1);
}

#[test]
fn exdate_for_an_all_day_instance() {
    let event = "BEGIN:VEVENT\nUID:a\nSUMMARY:Bins\nDTSTART;VALUE=DATE:20261005\nRRULE:FREQ=WEEKLY\nEXDATE;VALUE=DATE:20261026\nEND:VEVENT";
    assert!(on(event, date(2026, 10, 26)).is_empty());
    assert_eq!(on(event, date(2026, 11, 2)).len(), 1);
}

#[test]
fn moved_instance_replaces_the_generated_one() {
    let events = "BEGIN:VEVENT\nUID:series\nSUMMARY:Class\nDTSTART;TZID=Europe/Tallinn:20261005T100000\nDTEND;TZID=Europe/Tallinn:20261005T110000\nRRULE:FREQ=WEEKLY\nEND:VEVENT\n\
BEGIN:VEVENT\nUID:series\nRECURRENCE-ID;TZID=Europe/Tallinn:20261026T100000\nSUMMARY:Class (moved)\nDTSTART;TZID=Europe/Tallinn:20261026T150000\nDTEND;TZID=Europe/Tallinn:20261026T160000\nEND:VEVENT";
    let found = on(events, date(2026, 10, 26));
    assert_eq!(summaries(&found), vec!["Class (moved)"]);
    assert_eq!(found[0].start, 15 * 60);
    // Other weeks are untouched.
    assert_eq!(summaries(&on(events, date(2026, 10, 19))), vec!["Class"]);
}

#[test]
fn cancelled_instance_is_removed() {
    let events = "BEGIN:VEVENT\nUID:series\nSUMMARY:Class\nDTSTART;TZID=Europe/Tallinn:20261005T100000\nRRULE:FREQ=WEEKLY\nEND:VEVENT\n\
BEGIN:VEVENT\nUID:series\nRECURRENCE-ID;TZID=Europe/Tallinn:20261026T100000\nSTATUS:CANCELLED\nSUMMARY:Class\nDTSTART;TZID=Europe/Tallinn:20261026T100000\nEND:VEVENT";
    assert!(on(events, date(2026, 10, 26)).is_empty());
    assert_eq!(on(events, date(2026, 11, 2)).len(), 1);
}

#[test]
fn cancelled_single_event_is_ignored() {
    let event = "BEGIN:VEVENT\nUID:a\nSUMMARY:Off\nSTATUS:CANCELLED\nDTSTART;TZID=Europe/Tallinn:20261029T100000\nEND:VEVENT";
    assert!(on(event, date(2026, 10, 29)).is_empty());
}

#[test]
fn override_with_a_different_uid_does_not_hide_other_series() {
    let events = "BEGIN:VEVENT\nUID:one\nSUMMARY:One\nDTSTART;TZID=Europe/Tallinn:20261005T100000\nRRULE:FREQ=WEEKLY\nEND:VEVENT\n\
BEGIN:VEVENT\nUID:two\nRECURRENCE-ID;TZID=Europe/Tallinn:20261026T100000\nSTATUS:CANCELLED\nSUMMARY:Two\nDTSTART;TZID=Europe/Tallinn:20261026T100000\nEND:VEVENT";
    assert_eq!(summaries(&on(events, date(2026, 10, 26))), vec!["One"]);
}

#[test]
fn yearly_all_day_event_such_as_a_birthday() {
    let event = "BEGIN:VEVENT\nUID:a\nSUMMARY:Birthday\nDTSTART;VALUE=DATE:19800305\nDTEND;VALUE=DATE:19800306\nRRULE:FREQ=YEARLY\nEND:VEVENT";
    assert_eq!(on(event, date(2026, 3, 5)).len(), 1);
    assert_eq!(on(event, date(1980, 3, 5)).len(), 1);
    assert!(on(event, date(2026, 3, 6)).is_empty());
    assert!(on(event, date(1979, 3, 5)).is_empty());
}

#[test]
fn yearly_rule_honours_until() {
    let event = "BEGIN:VEVENT\nUID:a\nSUMMARY:Anniversary\nDTSTART;VALUE=DATE:20150305\nRRULE:FREQ=YEARLY;UNTIL=20200305\nEND:VEVENT";
    assert_eq!(on(event, date(2020, 3, 5)).len(), 1);
    assert!(on(event, date(2021, 3, 5)).is_empty());
}

#[test]
fn yearly_rule_with_bymonth_and_bymonthday() {
    let event = "BEGIN:VEVENT\nUID:a\nSUMMARY:Fixed\nDTSTART;VALUE=DATE:20250601\nRRULE:FREQ=YEARLY;BYMONTH=10;BYMONTHDAY=29\nEND:VEVENT";
    assert_eq!(on(event, date(2026, 10, 29)).len(), 1);
    assert!(on(event, date(2026, 6, 1)).is_empty());
}

#[test]
fn monthly_rule_on_the_second_tuesday() {
    let event = "BEGIN:VEVENT\nUID:a\nSUMMARY:Board\nDTSTART;TZID=Europe/Tallinn:20260113T180000\nRRULE:FREQ=MONTHLY;BYDAY=2TU\nEND:VEVENT";
    assert_eq!(on(event, date(2026, 10, 13)).len(), 1);
    assert!(on(event, date(2026, 10, 6)).is_empty());
    assert!(on(event, date(2026, 10, 20)).is_empty());
}

#[test]
fn monthly_rule_on_the_last_friday() {
    let event = "BEGIN:VEVENT\nUID:a\nSUMMARY:Review\nDTSTART;TZID=Europe/Tallinn:20260130T150000\nRRULE:FREQ=MONTHLY;BYDAY=-1FR\nEND:VEVENT";
    assert_eq!(on(event, date(2026, 10, 30)).len(), 1);
    assert!(on(event, date(2026, 10, 23)).is_empty());
}

#[test]
fn monthly_rule_defaults_to_the_start_day_of_month() {
    let event = "BEGIN:VEVENT\nUID:a\nSUMMARY:Rent\nDTSTART;VALUE=DATE:20260129\nRRULE:FREQ=MONTHLY\nEND:VEVENT";
    assert_eq!(on(event, date(2026, 10, 29)).len(), 1);
    assert!(on(event, date(2026, 10, 28)).is_empty());
    // The 31st never occurs in a 30-day month.
    let late = "BEGIN:VEVENT\nUID:a\nSUMMARY:Late\nDTSTART;VALUE=DATE:20260131\nRRULE:FREQ=MONTHLY\nEND:VEVENT";
    assert!(on(late, date(2026, 11, 30)).is_empty());
    assert_eq!(on(late, date(2026, 12, 31)).len(), 1);
}

#[test]
fn monthly_rule_on_the_last_day() {
    let event = "BEGIN:VEVENT\nUID:a\nSUMMARY:Close\nDTSTART;VALUE=DATE:20260131\nRRULE:FREQ=MONTHLY;BYMONTHDAY=-1\nEND:VEVENT";
    assert_eq!(on(event, date(2026, 11, 30)).len(), 1);
    assert!(on(event, date(2026, 11, 29)).is_empty());
}

#[test]
fn count_limits_the_number_of_instances() {
    let event = "BEGIN:VEVENT\nUID:a\nSUMMARY:Course\nDTSTART;TZID=Europe/Tallinn:20261005T100000\nRRULE:FREQ=WEEKLY;COUNT=3\nEND:VEVENT";
    for day in [5, 12, 19] {
        assert_eq!(on(event, date(2026, 10, day)).len(), 1, "day {day}");
    }
    assert!(on(event, date(2026, 10, 26)).is_empty());
}

#[test]
fn until_limits_a_weekly_rule() {
    let event = "BEGIN:VEVENT\nUID:a\nSUMMARY:Course\nDTSTART;TZID=Europe/Tallinn:20261005T100000\nRRULE:FREQ=WEEKLY;UNTIL=20261012T235959Z\nEND:VEVENT";
    assert_eq!(on(event, date(2026, 10, 12)).len(), 1);
    assert!(on(event, date(2026, 10, 19)).is_empty());
}

#[test]
fn utc_recurrence_keeps_its_utc_time_across_the_summer_time_change() {
    // 10:00 UTC every Friday: 12:00 in winter, 13:00 after summer time starts on 2026-03-29.
    let event = "BEGIN:VEVENT\nUID:a\nSUMMARY:Sync\nDTSTART:20260320T100000Z\nDTEND:20260320T110000Z\nRRULE:FREQ=WEEKLY\nEND:VEVENT";
    assert_eq!(on(event, date(2026, 3, 27))[0].start, 12 * 60);
    assert_eq!(on(event, date(2026, 4, 3))[0].start, 13 * 60);
}

#[test]
fn local_recurrence_keeps_its_wall_time_across_the_summer_time_change() {
    let event = "BEGIN:VEVENT\nUID:a\nSUMMARY:Sync\nDTSTART;TZID=Europe/Tallinn:20260320T100000\nDTEND;TZID=Europe/Tallinn:20260320T110000\nRRULE:FREQ=WEEKLY\nEND:VEVENT";
    assert_eq!(on(event, date(2026, 3, 27))[0].start, 10 * 60);
    assert_eq!(on(event, date(2026, 4, 3))[0].start, 10 * 60);
}

#[test]
fn estonian_time_changes_on_the_last_sunday_of_march_and_october() {
    // Spring: 01:00 UTC on 2026-03-29 jumps from 02:59 to 04:00 local.
    assert_eq!(
        utc_to_local(local(2026, 3, 29, 0, 59)),
        local(2026, 3, 29, 2, 59)
    );
    assert_eq!(
        utc_to_local(local(2026, 3, 29, 1, 0)),
        local(2026, 3, 29, 4, 0)
    );
    // Autumn: 01:00 UTC on 2026-10-25 goes back from 03:59 to 03:00 local.
    assert_eq!(
        utc_to_local(local(2026, 10, 25, 0, 59)),
        local(2026, 10, 25, 3, 59)
    );
    assert_eq!(
        utc_to_local(local(2026, 10, 25, 1, 0)),
        local(2026, 10, 25, 3, 0)
    );
    // The repeated hour takes its first occurrence; ordinary times convert plainly.
    assert_eq!(
        local_to_utc(local(2026, 10, 25, 3, 30)),
        local(2026, 10, 25, 0, 30)
    );
    assert_eq!(
        local_to_utc(local(2026, 10, 29, 12, 0)),
        local(2026, 10, 29, 10, 0)
    );
    assert_eq!(
        local_to_utc(local(2026, 7, 1, 12, 0)),
        local(2026, 7, 1, 9, 0)
    );
}

#[test]
fn folded_lines_escapes_and_alarms_are_handled() {
    let event = "BEGIN:VEVENT\nUID:a\nSUMMARY:Dinner\\, with \n friends\\nat home\nLOCATION:Pärnu mnt 1\\; Tallinn\nDTSTART;TZID=Europe/Tallinn:20261029T190000\nBEGIN:VALARM\nACTION:DISPLAY\nSUMMARY:Alarm text\nTRIGGER:-PT15M\nEND:VALARM\nEND:VEVENT";
    let found = on(event, date(2026, 10, 29));
    assert_eq!(found[0].summary, "Dinner, with friends at home");
    assert_eq!(found[0].location, "Pärnu mnt 1; Tallinn");
}

#[test]
fn missing_title_gets_a_placeholder() {
    let event = "BEGIN:VEVENT\nUID:a\nDTSTART;VALUE=DATE:20261029\nEND:VEVENT";
    assert_eq!(on(event, date(2026, 10, 29))[0].summary, "(no title)");
}

#[test]
fn all_day_events_come_first_then_by_start() {
    let events = "BEGIN:VEVENT\nUID:1\nSUMMARY:Evening\nDTSTART;TZID=Europe/Tallinn:20261029T190000\nEND:VEVENT\n\
BEGIN:VEVENT\nUID:2\nSUMMARY:Morning\nDTSTART;TZID=Europe/Tallinn:20261029T080000\nEND:VEVENT\n\
BEGIN:VEVENT\nUID:3\nSUMMARY:Holiday\nDTSTART;VALUE=DATE:20261029\nEND:VEVENT";
    assert_eq!(
        summaries(&on(events, date(2026, 10, 29))),
        vec!["Holiday", "Morning", "Evening"]
    );
}

#[test]
fn garbage_and_truncated_input_do_not_panic() {
    assert!(events_on("", date(2026, 10, 29)).is_empty());
    assert!(events_on("this is not a calendar", date(2026, 10, 29)).is_empty());
    assert!(events_on("BEGIN:VEVENT\nDTSTART:2026", date(2026, 10, 29)).is_empty());
    assert!(
        on(
            "BEGIN:VEVENT\nUID:a\nDTSTART:notadate\nEND:VEVENT",
            date(2026, 10, 29)
        )
        .is_empty()
    );
    assert!(
        on(
            "BEGIN:VEVENT\nUID:a\nDTSTART;VALUE=DATE:20261029\nRRULE:FREQ=SOMETIMES\nEND:VEVENT",
            date(2026, 10, 29)
        )
        .len()
            <= 1
    );
    // Non-ASCII junk in a date must not split a character in half.
    assert!(
        on(
            "BEGIN:VEVENT\nUID:a\nDTSTART:ääääääääääääää\nEND:VEVENT",
            date(2026, 10, 29)
        )
        .is_empty()
    );
}

#[test]
fn a_large_feed_is_handled() {
    let mut text = String::new();
    for i in 0..1500 {
        text.push_str(&format!(
            "BEGIN:VEVENT\nUID:e{i}\nSUMMARY:Event {i}\nDTSTART:2024{:02}{:02}T100000Z\nDTEND:2024{:02}{:02}T110000Z\nEND:VEVENT\n",
            i % 12 + 1,
            i % 28 + 1,
            i % 12 + 1,
            i % 28 + 1
        ));
    }
    text.push_str("BEGIN:VEVENT\nUID:r\nSUMMARY:Yearly\nDTSTART;VALUE=DATE:19700101\nRRULE:FREQ=YEARLY\nEND:VEVENT");
    let found = on(&text, date(2026, 1, 1));
    assert_eq!(summaries(&found), vec!["Yearly"]);
}

/// Smoke test against a real feed: `LUMA_TEST_ICS=/path/to/feed.ics cargo test -- --ignored real_feed`.
/// Prints counts and timing only, never event titles.
#[test]
#[ignore]
fn real_feed_smoke_test() {
    let Ok(path) = std::env::var("LUMA_TEST_ICS") else {
        return;
    };
    let text = std::fs::read_to_string(path).unwrap();
    let started = std::time::Instant::now();
    println!("parsed events: {}", parse_feed(&text).len());
    let days: i64 = std::env::var("LUMA_TEST_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(7);
    let first = std::env::var("LUMA_TEST_FIRST")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    for offset in first..first + days {
        let day = date(2026, 1, 1)
            .checked_add(Duration::days(offset))
            .unwrap();
        let found = events_on(&text, day);
        let all_day = found.iter().filter(|o| o.all_day).count();
        println!("COUNT {day} {} {all_day}", found.len());
    }
    println!("{days} days took {:?}", started.elapsed());
}
