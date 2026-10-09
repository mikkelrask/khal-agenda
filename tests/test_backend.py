import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("backend", Path(__file__).resolve().parents[1] / "data/backend.py")
backend = importlib.util.module_from_spec(spec)
spec.loader.exec_module(backend)


class Calendars(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        cal = root / "events"
        cal.mkdir()
        (cal / "events.ics").write_text("""BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Khal Agenda Test//EN
BEGIN:VEVENT
UID:daily
DTSTAMP:20260101T000000Z
DTSTART:20261010T080000Z
DTEND:20261010T090000Z
RRULE:FREQ=DAILY;COUNT=3
SUMMARY:Recurring meeting
END:VEVENT
BEGIN:VEVENT
UID:all-day
DTSTAMP:20260101T000000Z
DTSTART;VALUE=DATE:20261010
DTEND;VALUE=DATE:20261012
SUMMARY:Two-day event
END:VEVENT
END:VCALENDAR
""")
        combined = (cal / "events.ics").read_text()
        (cal / "events.ics").unlink()
        for i, event in enumerate(combined.split("BEGIN:VEVENT")[1:]):
            (cal / f"event-{i}.ics").write_text("BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//Khal Agenda Test//EN\nBEGIN:VEVENT" + event.split("END:VCALENDAR")[0] + "END:VCALENDAR\n")
        config = root / "khal.conf"
        config.write_text(f"""[calendars]
[[test]]
path = {cal}
[locale]
local_timezone = Europe/Copenhagen
timeformat = %H:%M
dateformat = %d/%m/%Y
[sqlite]
path = {root / 'cache.db'}
""")
        self.request = {"khal_config": str(config), "start": "2026-10-10", "days_ahead": 2}

    def tearDown(self):
        self.tmp.cleanup()

    def test_recurrence_and_exclusive_all_day_end(self):
        result = backend.snapshot(self.request)
        self.assertEqual([len(d["events"]) for d in result["days"]], [2, 2, 1])
        event = next(e for e in result["days"][0]["events"] if e["title"] == "Recurring meeting")
        self.assertIn("10:00", event["time"])
        self.assertEqual(result["calendars"], [{"id": "test", "name": "test"}])
        self.assertEqual(result["days"][0]["date"], "2026-10-10")

    def test_deselecting_every_calendar_really_hides_everything(self):
        result = backend.snapshot(dict(self.request, excluded=["test"]))
        self.assertTrue(all(not d["events"] for d in result["days"]))
        self.assertEqual(len(result["calendars"]), 1)

    def test_zero_days_ahead_includes_only_selected_day(self):
        result = backend.snapshot(dict(self.request, days_ahead=0))
        self.assertEqual(len(result["days"]), 1)
        self.assertEqual(len(result["days"][0]["events"]), 2)


if __name__ == "__main__":
    unittest.main()
