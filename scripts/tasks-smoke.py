#!/usr/bin/env python3
"""Verify task writes using only a temporary file on a live Wayland session."""
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
def input_with(text):return next((n for n in entries() if Atspi.Text.get_text(n,0,-1)==text),None)
with tempfile.TemporaryDirectory(prefix='khal-tasks-smoke-') as tmp:
    root=Path(tmp);todo=root/'todo.txt';todo.write_text('(A) Test task @home +demo\nx 2026-10-09 Finished\n')
    config=root/'khal-agenda/config.toml';config.parent.mkdir();config.write_text(f'todo_file = "{todo}"\n')
    env=dict(os.environ,XDG_CONFIG_HOME=tmp)
    with (root/'app.log').open('w') as log:
        p=subprocess.Popen([str(ROOT/'target/release/khal-agenda'),'--tasks'],env=env,stdout=log,stderr=log)
        try:
            wait(lambda:find('Complete task'))
            assert not find('Reopen task')
            if os.environ.get('KHAL_TASKS_SCREENSHOT'):
                import json
                title = find('Your tasks.', 'label')
                panel = title.get_parent().get_parent().get_parent()
                rect = panel.get_component_iface().get_extents(Atspi.CoordType.SCREEN)
                monitor = next(m for m in json.loads(subprocess.check_output(['mmsg','get','all-monitors'],text=True))['monitors'] if m['active'])
                x = monitor['x']+monitor['width']-16-rect.width
                subprocess.run(['grim','-g',f"{x},{monitor['y']+24} {rect.width}x{rect.height}",os.environ['KHAL_TASKS_SCREENSHOT']],check=True)
            edit=wait(lambda:input_with('(A) Test task @home +demo'))
            assert edit.get_editable_text_iface().set_text_contents('(B) Edited task @work +demo')
            click('Save task');wait(lambda:'(B) Edited task' in todo.read_text())
            click('Complete task');wait(lambda:todo.read_text().startswith('x '))
            done=wait(lambda:find('Show completed tasks','label'));switch=next(n for n in walk(done.get_parent()) if n.get_role_name()=='switch');assert switch.get_action_iface().do_action(0)
            wait(lambda:find('Reopen task'))
            reopen=[n for n in nodes() if n.get_name()=='Reopen task'][0];assert reopen.get_action_iface().do_action(0)
            wait(lambda:todo.read_text().startswith('(B) Edited task'))
            empty=[n for n in entries() if Atspi.Text.get_text(n,0,-1)==''];assert empty[-1].get_editable_text_iface().set_text_contents('Added @home')
            click('Add');wait(lambda:'Added @home' in todo.read_text())
            # A concurrent edit must never be silently overwritten.
            todo.write_text('External edit @sync\n')
            saves=[n for n in nodes() if n.get_name()=='Save task'];assert saves[0].get_action_iface().do_action(0)
            wait(lambda:any('changed elsewhere' in n.get_name() for n in nodes()))
            assert todo.read_text()=='External edit @sync\n'
            click('Refresh');wait(lambda:input_with('External edit @sync'))
            click('Settings');wait(lambda:find('Enable tasks'))
            text=find('Enable tasks');switch=next(n for n in walk(text.get_parent()) if n.get_role_name()=='switch');assert switch.get_action_iface().do_action(0)
            wait(lambda:tomllib.loads(config.read_text()).get('tasks_enabled'))
            click('Done');click('Tasks');wait(lambda:input_with('External edit @sync'))
            subprocess.run([str(ROOT/'target/release/khal-agenda'),'--tasks'],env=env,check=True)
            click('Close');p.wait(timeout=5);assert p.returncode==0
        finally:
            if p.poll() is None:p.terminate();p.wait(timeout=5)
    errors=(root/'app.log').read_text();assert 'panicked' not in errors and 'CRITICAL' not in errors,errors
print('PASS: task edit, completion, reopen, add, external-change protection, refresh, toggle and --tasks activation')
