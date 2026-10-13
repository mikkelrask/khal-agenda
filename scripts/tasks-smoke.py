#!/usr/bin/env python3
"""Verify task writes using only a temporary file on a live Wayland session."""
import datetime as dt
import os
from pathlib import Path
import subprocess
import tempfile
import time
import tomllib
import gi
gi.require_version('Atspi', '2.0')
from gi.repository import Atspi, GLib
ROOT = Path(__file__).resolve().parents[1]
def walk(n):
    if n is None: return
    yield n
    for i in range(n.get_child_count()): yield from walk(n.get_child_at_index(i))
def nodes():
    for app in walk(Atspi.get_desktop(0)):
        if app.get_role_name() == 'application' and app.get_name() == 'khal-agenda':
            for n in walk(app):
                if n.get_state_set().contains(Atspi.StateType.SHOWING): yield n

def wait(f):
    end = time.monotonic()+8
    while time.monotonic()<end:
        try: v=f()
        except GLib.Error: v=None
        if v: return v
        time.sleep(.1)
    raise AssertionError('Timed out')
def find(name,role=None): return next((n for n in nodes() if n.get_name()==name and (role is None or n.get_role_name()==role)),None)
def click(name): assert wait(lambda:find(name,'push button') or find(name,'button')).get_action_iface().do_action(0)
def entries():return [n for n in nodes() if n.get_role_name() in ('entry','text') and n.get_editable_text_iface() is not None]
def task(title):
    node=find(title,'label')
    while node is not None and node.get_role_name()!='check box':node=node.get_parent()
    return node
def toggle_task(title):
    checkbox=wait(lambda:task(title))
    action=checkbox.get_action_iface()
    if action.get_n_actions():
        assert action.do_action(0)
        return
    # Recent GTK versions expose no AT-SPI checkbox action; exercise native keyboard activation.
    for _ in range(20):
        if task(title).get_state_set().contains(Atspi.StateType.FOCUSED):break
        subprocess.run(['wlrctl','keyboard','type','\t'],check=True)
        time.sleep(.05)
    assert task(title).get_state_set().contains(Atspi.StateType.FOCUSED)
    subprocess.run(['wlrctl','keyboard','type',' '],check=True)
def toggle_completed():
    text=wait(lambda:find('Show completed tasks','label'))
    switch=next(n for n in walk(text.get_parent()) if n.get_role_name()=='switch')
    assert switch.get_action_iface().do_action(0)
with tempfile.TemporaryDirectory(prefix='khal-tasks-smoke-') as tmp:
    root=Path(tmp);todo=root/'todo.txt'
    original='(A) 2026-10-01 Plan weekend trip @home +travel\n(B) Book bike service @town +errands\nBuy coffee beans for the weekend @town +shopping\nx 2026-10-09 Finished\n'
    todo.write_text(original)
    config=root/'khal-agenda/config.toml';config.parent.mkdir();config.write_text(f'todo_file = "{todo}"\ntheme = "{os.environ.get("KHAL_AGENDA_THEME", "system")}"\n')
    env=dict(os.environ,XDG_CONFIG_HOME=tmp)
    with (root/'app.log').open('w') as log:
        p=subprocess.Popen([str(ROOT/'target/release/khal-agenda'),'--tasks'],env=env,stdout=log,stderr=log)
        try:
            wait(lambda:task('Plan weekend trip'))
            assert not find('Finished')
            assert not entries(), 'Task view must have no text inputs'
            for name in ('Add','Save task','Complete task','Reopen task'):assert not find(name)
            assert find('A','label') and find('B','label')
            assert find('Created 2026-10-01 · @home · +travel','label')
            assert task('Plan weekend trip').get_name(), 'Checkbox must have an accessible name'
            if os.environ.get('KHAL_TASKS_SCREENSHOT'):
                import json
                title = find('Your tasks.', 'label')
                panel = title.get_parent().get_parent().get_parent()
                rect = panel.get_component_iface().get_extents(Atspi.CoordType.SCREEN)
                monitor = next(m for m in json.loads(subprocess.check_output(['mmsg','get','all-monitors'],text=True))['monitors'] if m['active'])
                x = monitor['x']+monitor['width']-16-rect.width
                subprocess.run(['grim','-g',f"{x},{monitor['y']+24} {rect.width}x{rect.height}",os.environ['KHAL_TASKS_SCREENSHOT']],check=True)
            toggle_task('Plan weekend trip')
            wait(lambda:todo.read_text()==f'x {dt.date.today()} {original}')
            wait(lambda:find('2 remaining','label'))
            assert not task('Plan weekend trip')
            toggle_completed()
            completed=wait(lambda:task('Plan weekend trip'))
            assert completed.get_state_set().contains(Atspi.StateType.CHECKED)
            toggle_task('Plan weekend trip');wait(lambda:todo.read_text()==original)
            toggle_completed()
            for title in ('Plan weekend trip','Book bike service','Buy coffee beans for the weekend'):
                toggle_task(title)
                wait(lambda title=title:task(title) is None)
            wait(lambda:find('All caught up.','label'))
            toggle_completed()
            toggle_task('Plan weekend trip');wait(lambda:todo.read_text().startswith('(A)'))
            # A concurrent edit must never be silently overwritten, and the checkbox must reset.
            todo.write_text('External edit @sync\n')
            toggle_task('Plan weekend trip')
            wait(lambda:any('changed elsewhere' in n.get_name() for n in nodes()))
            assert todo.read_text()=='External edit @sync\n'
            assert not task('Plan weekend trip').get_state_set().contains(Atspi.StateType.CHECKED)
            click('Refresh');wait(lambda:task('External edit'))
            click('Settings');wait(lambda:find('Enable tasks'))
            text=find('Enable tasks');switch=next(n for n in walk(text.get_parent()) if n.get_role_name()=='switch');assert switch.get_action_iface().do_action(0)
            wait(lambda:tomllib.loads(config.read_text()).get('tasks_enabled'))
            click('Browse')
            def chooser_control(name): return find(name,'button')
            cancel=wait(lambda:chooser_control('Cancel'));assert p.poll() is None
            assert cancel.get_action_iface().do_action(0)
            wait(lambda:find('Browse'));assert p.poll() is None
            alternate=root/'alternate.txt';alternate.write_text('External edit @sync\n')
            click('Browse')
            file=wait(lambda:find('alternate.txt','table cell'))
            row=file.get_parent()
            assert row.get_parent().get_selection_iface().select_child(row.get_index_in_parent())
            choose=wait(lambda:(b:=chooser_control('Choose')) and b.get_state_set().contains(Atspi.StateType.SENSITIVE) and b)
            assert choose.get_action_iface().do_action(0)
            wait(lambda:find('Browse'));assert p.poll() is None
            assert tomllib.loads(config.read_text())['todo_file']==str(alternate)
            click('Done');click('Tasks');wait(lambda:task('External edit'))
            subprocess.run([str(ROOT/'target/release/khal-agenda'),'--tasks'],env=env,check=True)
            click('Close');p.wait(timeout=5);assert p.returncode==0
        finally:
            if p.poll() is None:p.terminate();p.wait(timeout=5)
    errors=(root/'app.log').read_text();assert 'panicked' not in errors and 'CRITICAL' not in errors,errors
print('PASS: read-only task cards, metadata, accessible checkboxes, completion/reopen, empty state, external-change protection, refresh, toggle, file chooser cancel/accept and --tasks activation')
