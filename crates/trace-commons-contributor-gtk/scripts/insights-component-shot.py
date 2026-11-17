#!/usr/bin/env python3
"""Linux-only synthetic component smoke; this does not validate app wiring/data.

Set INSIGHTS_TEXT_SCALE=1.5 for enlarged text.
Run under xvfb-run with python3-gi, GTK4, libadwaita and ImageMagick installed:
  xvfb-run -a python3 insights-component-shot.py brand_tokens.rs out.png light 440
Uses generated production CSS verbatim; fails on GTK CSS parser diagnostics.
"""
import os
import pathlib
import re
import subprocess
import sys
import gi
gi.require_version('Gtk', '4.0')
gi.require_version('Adw', '1')
from gi.repository import Adw, Gtk, Gdk, GLib

source, shot, scheme, width = sys.argv[1:]
width = int(width)
Adw.init()
scale = float(os.environ.get('INSIGHTS_TEXT_SCALE', '1'))
Gtk.Settings.get_default().set_property('gtk-font-name', f'Sans {10 * scale:g}')
Adw.StyleManager.get_default().set_color_scheme(Adw.ColorScheme.FORCE_DARK if scheme == 'dark' else Adw.ColorScheme.FORCE_LIGHT)
source = pathlib.Path(source)
def constant(path, name):
    return re.search(r'const ' + name + r': &str = r#"(.*?)"#;', path.read_text(), re.S).group(1)
css = '\n'.join([
    constant(source, scheme.upper() + '_CSS'),
    constant(source.with_name('style.rs'), scheme.upper() + '_TOKENS'),
    source.with_name('style.css').read_text(),
    constant(source, 'INSIGHTS_' + scheme.upper() + '_CSS'),
])
errors = []
provider = Gtk.CssProvider()
provider.connect('parsing-error', lambda _, section, error: errors.append(str(error)))
provider.load_from_data(css.encode())
assert not errors, errors
Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
win = Gtk.Window(title='Synthetic Insights components')
win.set_default_size(width, 650)
root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=16)
root.add_css_class('tc-insights')
win.set_child(root)
body = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=16)
for side in ('top', 'bottom', 'start', 'end'):
    getattr(body, 'set_margin_' + side)(16)
root.append(body)
def label(text, cls):
    w = Gtk.Label(label=text, xalign=0)
    w.set_wrap(True)
    w.add_css_class(cls)
    return w
stack = Gtk.Stack()
for name in ['Overview', 'Patterns', 'Sessions', 'Analyze']:
    stack.add_titled(Gtk.Box(), name, name)
switcher = Gtk.StackSwitcher(stack=stack)
nav = Gtk.ScrolledWindow(hscrollbar_policy=Gtk.PolicyType.AUTOMATIC, vscrollbar_policy=Gtk.PolicyType.NEVER)
nav.set_child(switcher)
body.append(nav)
body.append(label('This week', 'title-2'))
body.append(label('Nov 9–15 · 24 saved sessions', 'caption'))
body.append(label('Saved snapshots · Weeks are not compared', 'caption'))
flow = Gtk.FlowBox(selection_mode=Gtk.SelectionMode.NONE, homogeneous=True, min_children_per_line=1, max_children_per_line=3, row_spacing=8, column_spacing=8)
flow.add_css_class('tc-insights-summary-grid')
buttons=[]
activated=[]
for name, value, detail in [('Tokens', '128,400', 'Claude Code'), ('Cache share', '64%', 'Known input tokens'), ('Sessions', '24', 'Saved snapshots')]:
    content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
    content.append(label(name, 'heading'))
    content.append(label(value, 'title-3'))
    content.append(label(detail, 'caption'))
    button = Gtk.Button(child=content)
    button.add_css_class('tc-insights-summary')
    button.connect('clicked', lambda _, name=name: activated.append(name))
    buttons.append(button)
    flow.insert(button, -1)
body.append(flow)
card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
card.add_css_class('tc-insights-card')
card.append(label('Tokens by day', 'heading'))
for day, fraction in [('Mon', .35), ('Tue', .75), ('Wed', .5)]:
    row = Gtk.Box(spacing=8)
    row.append(label(day, 'caption'))
    bar = Gtk.LevelBar(min_value=0, max_value=1, value=fraction)
    bar.set_hexpand(True)
    row.append(bar)
    card.append(row)
body.append(card)
body.append(label('Synthetic component fixture — not application data', 'caption'))
loop = GLib.MainLoop()
def capture():
    try:
        assert not errors, errors
        assert activated == ['Tokens', 'Cache share', 'Sessions'], activated
        assert win.get_width() == width, (win.get_width(), width)
        for b in buttons:
            assert b.get_width() > 0 and b.get_width() < width
        subprocess.run(['import', '-window', 'root', shot], check=True)
        deviation = float(subprocess.check_output(['convert', shot, '-colorspace', 'Gray', '-format', '%[standard-deviation]', 'info:']))
        assert deviation > 1, 'Blank screenshot'
        print(f'{scheme} {width}px: CSS parsed; allocated summary widths {[b.get_width() for b in buttons]}; {shot}')
    except Exception as error:
        errors.append(str(error))
    finally:
        loop.quit()
    return GLib.SOURCE_REMOVE
win.present()
def exercise():
    try:
        for button in buttons:
            assert button.grab_focus()
            assert button.has_focus()
            assert button.activate()
        assert flow.child_focus(Gtk.DirectionType.TAB_BACKWARD)
    except Exception as error:
        errors.append(str(error))
    GLib.timeout_add(500, capture)
    return GLib.SOURCE_REMOVE
GLib.timeout_add(1000, exercise)
loop.run()
assert not errors, errors
