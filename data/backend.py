"""Read the configured khal collection once, returning a day-grouped agenda."""
import datetime as dt
import json
import locale
import sys


def snapshot(request):
    from khal.settings import get_config
    from khal.cli_utils import build_collection
    from khal.controllers import get_events_between
    config = get_config(request.get("khal_config"))
    calendars = [{"id": name, "name": cal.get("displayname") or name}
                 for name, cal in config["calendars"].items()]
    selected = {cal["id"] for cal in calendars} - set(request.get("excluded", []))
    start = dt.date.fromisoformat(request["start"]) if request.get("start") else dt.datetime.now(config["locale"]["local_timezone"]).date()
    ahead = request.get("days_ahead", 7)
    if not isinstance(ahead, int) or not 0 <= ahead <= 90:
        raise ValueError("Days ahead must be between 0 and 90")
    collection = build_collection(config, selected) if selected else None
    try:
        locale.setlocale(locale.LC_TIME, "")
    except locale.Error:
        pass
    days = []
    for offset in range(ahead + 1):
        day = start + dt.timedelta(days=offset)
        rows = [] if collection is None else get_events_between(
            collection=collection, locale=config["locale"],
            start=dt.datetime.combine(day, dt.time.min),
            end=dt.datetime.combine(day, dt.time.max),
            formatter=lambda rows: rows, notstarted=False, env={"calendars": config["calendars"]},
            original_start=None, colors=False)
        events = []
        for row in rows:
            if row.get("cancelled", "").strip():
                continue
            events.append({"title": row.get("title", "Untitled event"),
                           "time": "All day" if row.get("all-day") == "True" else row.get("start-end-time-style", "") or (row["start-time"] + "–" + row["end-time"]),
                           "calendar": row.get("calendar", ""),
                           "description": row.get("description", ""),
                           "location": row.get("location", "")})
        days.append({"date": day.isoformat(), "label": day.strftime("%A, %-d %B"), "events": events})
    return {"calendars": calendars, "days": days, "start": start.isoformat(),
            "timezone": str(config["locale"]["local_timezone"])}


if __name__ == "__main__":
    try:
        print(json.dumps(snapshot(json.loads(sys.argv[1])), ensure_ascii=False))
    except (Exception, SystemExit) as error:
        print(f"Could not read calendars: {error}. Check that khal is installed for this Python interpreter and that khal list works.", file=sys.stderr)
        sys.exit(1)
