#!/usr/bin/env python3
"""Builds a small timetable file of Elron trains leaving Kohila from the national GTFS feed.

The full feed is ~50 MB, far too much for a TV widget. This reads it once and writes the
next few days of Kohila departures, each with its arrival times at Tallinn-Väike and Tallinn:

    {"v": 1, "generated": "2026-10-29", "stop": "Kohila",
     "days": {"2026-10-29": [{"d": 369, "w": 403, "t": 409, "l": "R21"}, ...]}}

Times are minutes after midnight of the service day (they can pass 1440 for trains running
after midnight). "d" is the departure from Kohila, "w" the arrival at Tallinn-Väike and "t" the
arrival at Tallinn (Balti jaam); either arrival is omitted when the train does not serve it.
This is the planned timetable only: the feed has no live delay information.
"""

import argparse
import csv
import datetime
import io
import json
import sys
import urllib.request
import zipfile
from zoneinfo import ZoneInfo

FEED_URL = "https://eu-gtfs.remix.com/estonia_unified_gtfs.zip"
AGENCY = "ELRON"
RAIL_ROUTE_TYPES = {"2", "100", "101", "102", "109"}


def last_sunday(year, month):
    day = datetime.date(year, month, 31 if month in (3, 10) else 28)
    while day.weekday() != 6:
        day -= datetime.timedelta(days=1)
    return day


def tallinn_today():
    """Today's date in Estonia. Uses the system time zone database, with a built-in
    European summer-time rule for machines that lack one (e.g. Windows without tzdata)."""
    now = datetime.datetime.now(datetime.timezone.utc)
    try:
        return now.astimezone(ZoneInfo("Europe/Tallinn")).date()
    except Exception:
        # EEST (UTC+3) runs from 01:00 UTC on the last Sunday of March to the last Sunday of October.
        start = datetime.datetime.combine(last_sunday(now.year, 3), datetime.time(1), datetime.timezone.utc)
        end = datetime.datetime.combine(last_sunday(now.year, 10), datetime.time(1), datetime.timezone.utc)
        offset = 3 if start <= now < end else 2
        return (now + datetime.timedelta(hours=offset)).date()


def read_csv(archive, name):
    with archive.open(name) as raw:
        text = io.TextIOWrapper(raw, encoding="utf-8-sig", newline="")
        yield from csv.DictReader(text)


def to_minutes(value):
    """GTFS times are HH:MM:SS and may exceed 24:00:00 for trips after midnight."""
    hours, minutes, _ = value.split(":")
    return int(hours) * 60 + int(minutes)


def running_services(archive, day):
    """Service ids that run on the given date (calendar.txt plus calendar_dates.txt)."""
    ymd = day.strftime("%Y%m%d")
    weekday = day.strftime("%A").lower()
    running = set()
    for row in read_csv(archive, "calendar.txt"):
        if row["start_date"] <= ymd <= row["end_date"] and row[weekday] == "1":
            running.add(row["service_id"])
    for row in read_csv(archive, "calendar_dates.txt"):
        if row["date"] == ymd:
            if row["exception_type"] == "1":
                running.add(row["service_id"])
            elif row["exception_type"] == "2":
                running.discard(row["service_id"])
    return running


def build(archive, today, day_count):
    agencies = {
        row["agency_id"]: row["agency_name"].strip().upper() for row in read_csv(archive, "agency.txt")
    }
    routes = {
        row["route_id"]: row["route_short_name"]
        for row in read_csv(archive, "routes.txt")
        if row["route_type"] in RAIL_ROUTE_TYPES and agencies.get(row["agency_id"]) == AGENCY
    }
    trips = {
        row["trip_id"]: (row["service_id"], routes[row["route_id"]])
        for row in read_csv(archive, "trips.txt")
        if row["route_id"] in routes
    }

    stops = {row["stop_id"]: row["stop_name"].strip().lower() for row in read_csv(archive, "stops.txt")}
    kohila = {s for s, n in stops.items() if n.startswith("kohila")}
    vaike = {s for s, n in stops.items() if n.startswith("tallinn-v")}
    tallinn = {s for s, n in stops.items() if n == "tallinn" or n.startswith("balti jaam")}
    wanted = kohila | vaike | tallinn

    # trip_id -> [(stop_sequence, kind, departure, arrival)]
    calls = {}
    for row in read_csv(archive, "stop_times.txt"):
        if row["trip_id"] not in trips or row["stop_id"] not in wanted:
            continue
        stop = row["stop_id"]
        kind = "k" if stop in kohila else "w" if stop in vaike else "t"
        calls.setdefault(row["trip_id"], []).append(
            (int(row["stop_sequence"]), kind, to_minutes(row["departure_time"]), to_minutes(row["arrival_time"]))
        )

    # Per trip: the Kohila departure, and the arrivals that come after it.
    departures = []  # (service_id, entry)
    for trip_id, stop_calls in calls.items():
        stop_calls.sort()
        origin = next((c for c in stop_calls if c[1] == "k"), None)
        if origin is None:
            continue
        entry = {"d": origin[2], "l": trips[trip_id][1]}
        for sequence, kind, _, arrival in stop_calls:
            if kind != "k" and sequence > origin[0]:
                entry[kind] = arrival  # the last call at a station is where the train ends up
        if "w" in entry or "t" in entry:
            departures.append((trips[trip_id][0], entry))

    days = {}
    for offset in range(day_count):
        day = today + datetime.timedelta(days=offset)
        services = running_services(archive, day)
        todays = [entry for service, entry in departures if service in services]
        todays.sort(key=lambda e: (e["d"], e["l"]))
        days[day.isoformat()] = todays
    return {"v": 1, "generated": today.isoformat(), "stop": "Kohila", "days": days}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--zip", help="use this GTFS zip instead of downloading the national feed")
    parser.add_argument("--out", required=True, help="where to write the JSON timetable")
    parser.add_argument("--days", type=int, default=8, help="how many days to include, starting today")
    args = parser.parse_args()

    if args.zip:
        source = args.zip
    else:
        print("downloading", FEED_URL, file=sys.stderr)
        source = io.BytesIO(urllib.request.urlopen(FEED_URL, timeout=300).read())

    today = tallinn_today()
    with zipfile.ZipFile(source) as archive:
        data = build(archive, today, args.days)

    total = sum(len(v) for v in data["days"].values())
    if total == 0:
        sys.exit("no Kohila departures found; refusing to write an empty timetable")
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(data, f, ensure_ascii=False, separators=(",", ":"))
    print(f"wrote {args.out}: {len(data['days'])} days, {total} departures", file=sys.stderr)


if __name__ == "__main__":
    main()
