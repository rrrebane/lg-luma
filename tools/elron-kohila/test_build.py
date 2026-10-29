"""Tests for build.py against a tiny hand-made GTFS feed (no network)."""

import datetime
import io
import unittest
import zipfile

import build


def make_feed(files):
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        for name, rows in files.items():
            archive.writestr(name, "\n".join(rows) + "\n")
    buffer.seek(0)
    return zipfile.ZipFile(buffer)


FEED = {
    "agency.txt": ["agency_id,agency_name", "E,ELRON", "B,Some Bus Company"],
    "routes.txt": [
        "route_id,agency_id,route_short_name,route_type",
        "r21,E,R21,2",
        "bus1,B,100,3",
        "elbus,E,EB,3",
    ],
    "stops.txt": [
        "stop_id,stop_name",
        "k1,Kohila",
        "k2,Kohila raudteejaam",
        "v1,Tallinn-Väike",
        "t1,Tallinn",
        "t2,Balti jaam (train station)",
        "x1,Rapla",
    ],
    "trips.txt": [
        "trip_id,route_id,service_id",
        "toward,r21,weekday",
        "away,r21,weekday",  # runs Tallinn -> Kohila: must not be listed
        "late,r21,weekday",  # after midnight
        "holiday,r21,sunday",
        "bus,bus1,weekday",
        "elbus,elbus,weekday",  # an Elron bus is not a train
    ],
    "stop_times.txt": [
        "trip_id,stop_id,stop_sequence,arrival_time,departure_time",
        "toward,x1,1,05:30:00,05:31:00",
        "toward,k1,2,06:09:00,06:10:00",
        "toward,v1,3,06:43:00,06:43:00",
        "toward,t1,4,06:49:00,06:49:00",
        "away,t1,1,07:00:00,07:00:00",
        "away,v1,2,07:06:00,07:06:00",
        "away,k1,3,07:40:00,07:41:00",
        "late,k2,1,24:10:00,24:11:00",
        "late,t2,2,24:50:00,24:50:00",
        "holiday,k1,1,09:00:00,09:00:00",
        "holiday,v1,2,09:33:00,09:33:00",
        "bus,k1,1,06:00:00,06:00:00",
        "bus,t1,2,07:00:00,07:00:00",
        "elbus,k1,1,06:30:00,06:30:00",
        "elbus,t1,2,07:30:00,07:30:00",
    ],
    "calendar.txt": [
        "service_id,monday,tuesday,wednesday,thursday,friday,saturday,sunday,start_date,end_date",
        "weekday,1,1,1,1,1,0,0,20260101,20261231",
        "sunday,0,0,0,0,0,0,1,20260101,20261231",
    ],
    "calendar_dates.txt": [
        "service_id,date,exception_type",
        "weekday,20261102,2",  # a weekday with no service
        "sunday,20261103,1",  # an extra service day
    ],
}


class BuildTest(unittest.TestCase):
    def run_build(self, start="2026-10-29", days=8):
        return build.build(make_feed(FEED), datetime.date.fromisoformat(start), days)

    def test_toward_trip_has_both_arrivals(self):
        weekday = self.run_build()["days"]["2026-10-29"]  # a Thursday
        self.assertEqual(weekday[0], {"d": 370, "l": "R21", "w": 403, "t": 409})

    def test_trip_that_leaves_tallinn_is_not_listed(self):
        for entry in self.run_build()["days"]["2026-10-29"]:
            self.assertNotEqual(entry["d"], 7 * 60 + 41)

    def test_after_midnight_times_pass_1440(self):
        late = self.run_build()["days"]["2026-10-29"][-1]
        self.assertEqual(late, {"d": 1451, "l": "R21", "t": 1490})  # no Tallinn-Väike stop

    def test_only_elron_trains_are_included(self):
        lines = {e["l"] for day in self.run_build()["days"].values() for e in day}
        self.assertEqual(lines, {"R21"})

    def test_calendar_rules_and_exceptions(self):
        days = self.run_build()["days"]
        self.assertEqual(len(days["2026-10-29"]), 2)  # Thursday: toward + late
        self.assertEqual(len(days["2026-11-01"]), 1)  # Sunday: holiday trip only
        self.assertEqual(days["2026-11-01"][0]["w"], 573)
        self.assertEqual(days["2026-11-02"], [])  # weekday service removed
        self.assertEqual(len(days["2026-11-03"]), 3)  # a Tuesday: its two trains plus the extra service

    def test_day_count(self):
        self.assertEqual(len(self.run_build(days=3)["days"]), 3)


if __name__ == "__main__":
    unittest.main()
