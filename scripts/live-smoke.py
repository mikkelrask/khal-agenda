#!/usr/bin/env python3
"""Exercise the popup on a live Wayland session, with synthetic calendar data."""
import datetime as dt
import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import time
import tomllib
import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("fixtures", ROOT / "tests/test_backend.py")
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)
fixture = fixtures.Calendars()
fixture.setUp()
for path in (Path(fixture.tmp.name) / "events").glob("*.ics"):
    text = path.read_text().replace("20261010", dt.date.today().strftime("%Y%m%d")).replace("20261012", (dt.date.today() + dt.timedelta(days=2)).strftime("%Y%m%d"))
    path.write_text(text)


def walk(node):
    if node is None:
        return
    yield node
    for i in range(node.get_child_count()):
        yield from walk(node.get_child_at_index(i))


def find(name=None, role=None):
    apps = [n for n in walk(Atspi.get_desktop(0)) if n.get_role_name() == "application" and n.get_name() == "khal-agenda"]
    for app in apps:
        for n in walk(app):
            if (name is None or n.get_name() == name) and (role is None or n.get_role_name() == role) and n.get_state_set().contains(Atspi.StateType.SHOWING):
                return n
    return None


def wait(predicate, message):
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(.1)
    raise AssertionError(message)


def click(name):
    assert wait(lambda: find(name, "button"), "Missing " + name).get_action_iface().do_action(0)


with tempfile.TemporaryDirectory(prefix="khal-agenda-smoke-") as directory:
    config = Path(directory) / "khal-agenda/config.toml"
    config.parent.mkdir()
    config.write_text(f'khal_config = "{fixture.request["khal_config"]}"\ntheme = "{os.environ.get("KHAL_AGENDA_THEME", "system")}"\n')
    log = open(Path(directory) / "app.log", "w")
    process = subprocess.Popen([str(ROOT / "target/release/khal-agenda")], env=dict(os.environ, XDG_CONFIG_HOME=directory), stdout=log, stderr=log)
    try:
        wait(lambda: find("Your days ahead."), "Popup did not open")
        click("Settings")
        wait(lambda: find("test", "label"), "Initial calendar discovery did not reach Settings")
        click("Done")
        wait(lambda: find("Recurring meeting"), "Agenda did not load")
        click("Settings")
        wait(lambda: find("Make it yours."), "Settings did not open")
        name = wait(lambda: find("test", "label"), "Calendar toggle did not appear")
        switch = next(n for n in walk(name.get_parent()) if n.get_role_name() == "switch")
        assert switch.get_action_iface().do_action(0)
        wait(lambda: tomllib.loads(config.read_text()).get("excluded") == ["test"], "Calendar exclusion did not persist")
        click("Done")
        wait(lambda: find("No calendars selected. Choose calendars in Settings."), "All deselected calendars should be empty")
        click("Settings")
        name = wait(lambda: find("test", "label"), "Calendar toggle did not appear")
        switch = next(n for n in walk(name.get_parent()) if n.get_role_name() == "switch")
        assert switch.get_action_iface().do_action(0)
        name = wait(lambda: find("Show month calendar", "label"), "Month toggle did not appear")
        switch = next(n for n in walk(name.get_parent()) if n.get_role_name() == "switch")
        assert switch.get_action_iface().do_action(0)
        wait(lambda: tomllib.loads(config.read_text()).get("month_view"), "Month setting did not persist")
        margin_label = wait(lambda: find("Top margin (px)", "label"), "Margin control did not appear")
        margin = next(n for n in walk(margin_label.get_parent()) if n.get_role_name() == "spin button")
        assert margin.get_value_iface().set_current_value(72)
        wait(lambda: tomllib.loads(config.read_text()).get("top_margin") == 72, "Margin did not persist")
        days_label = find("Days ahead", "label")
        spin = next(n for n in walk(days_label.get_parent()) if n.get_role_name() == "spin button")
        assert spin.get_value_iface().set_current_value(0)
        wait(lambda: tomllib.loads(config.read_text()).get("days_ahead") == 0, "Days ahead did not persist")
        click("Done")
        wait(lambda: find("Recurring meeting"), "Re-enabled calendar did not load")
        click("Today")
        click("Refresh")
        wait(lambda: find("Recurring meeting"), "Refresh did not finish")
        if os.environ.get("KHAL_AGENDA_SCREENSHOT"):
            # Only capture this app's panel, positioned by its documented anchors.
            title = find("Your days ahead.", "label")
            panel = title.get_parent().get_parent().get_parent()
            rect = panel.get_component_iface().get_extents(Atspi.CoordType.SCREEN)
            import json
            monitor = next(m for m in json.loads(subprocess.check_output(["mmsg", "get", "all-monitors"], text=True))["monitors"] if m["active"])
            x = monitor["x"] + monitor["width"] - rect.width - 16
            subprocess.run(["grim", "-g", f'{x},{monitor["y"]+72} {rect.width}x{rect.height}', os.environ["KHAL_AGENDA_SCREENSHOT"]], check=True)
        backdrop = find("", "button")
        assert backdrop.get_action_iface().do_action(0)
        process.wait(timeout=5)
        assert process.returncode == 0
        contents = Path(directory, "app.log").read_text()
        assert "panicked" not in contents and "CRITICAL" not in contents, contents
        print("PASS: agenda, calendar toggles, month view, days ahead, Today, Refresh, and outside dismissal/process exit.")
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=5)
        fixture.tearDown()
        log.close()
