#!/usr/bin/env python3
"""Auto-approve GNOME portal dialogs for `spacing` (Capture Input and
Remote Desktop) via AT-SPI — no manual clicking needed.

GNOME's portal backend (xdg-desktop-portal-gnome >= 45) shows a GTK
permission dialog for both portals:

  * serve (InputCapture CreateSession): "An application wants to
    capture input events" + "Allow Remote Input Capturing" switch
  * client (RemoteDesktop Start): "Allow Remote Interaction" switch

The request only resolves after the switch is toggled on and "Share"
is clicked.

Ladder per pass (works across GNOME 45–50):

  1. Read the switch's real state — only toggle if it is off.
  2. Try the AT-SPI "Toggle" action first (works on <= GNOME 49). On
     GNOME 50 the action exists but is inert — if the state did not
     change, fall back to the dialog's "default.activate", which
     toggles the default widget (the switch) there.
  3. Click "Share" via AT-SPI. On GNOME 50 the button's "Click"
     action is likewise inert — if the dialog is still up, fall back
     to real keyboard input through org.gnome.Mutter.RemoteDesktop
     (the internal session API, no extra portal approval): Tab until
     the "Share" button reports FOCUSED, then Return.

Usage:
    python3 contrib/portal-approve.py [--timeout 60]

Run it in a second terminal while `spacing serve` / `spacing client`
is waiting on the dialog, or leave it looping in the background for a
hands-free smoke test. Requires: dbus-python, an AT-SPI bus
(org.a11y.Bus — present on any standard GNOME session). The keyboard
fallback additionally needs the mutter RemoteDesktop bus, which is
always present on GNOME/Wayland.
"""
import argparse
import sys
import time

import dbus

ACC = "org.a11y.atspi.Accessible"
ACTION = "org.a11y.atspi.Action"
PROPS = "org.freedesktop.DBus.Properties"
ROOT = "/org/a11y/atspi/accessible/root"

ROLE_PUSH_BUTTON = 43
ROLE_DIALOG = 23
ROLE_SWITCH = 130  # GTK4 exposes GtkSwitch/AdwSwitchRow under this role

STATE_CHECKED = 4
STATE_FOCUSED = 12

DIALOG_TITLES = ("remote desktop", "capture input")
SWITCH_HINTS = ("allow remote input capturing", "allow remote interaction",
                "allow remote")
BUTTON_LABELS = ("share", "allow", "accept", "connect")

KEYSYM_TAB = 0xFF09
KEYSYM_RETURN = 0xFF0D


def a11y_bus():
    sb = dbus.SessionBus()
    addr = sb.get_object("org.a11y.Bus", "/org/a11y/bus",
                         introspect=False).GetAddress(
                             dbus_interface="org.a11y.Bus")
    return dbus.bus.BusConnection(addr)


def acc(bus, name, path):
    return bus.get_object(name, path, introspect=False)


def name_of(o):
    try:
        return str(o.Get(ACC, "Name", dbus_interface=PROPS))
    except Exception:
        return ""


def role_of(o):
    try:
        return int(o.GetRole(dbus_interface=ACC))
    except Exception:
        return -1


def actions_of(o):
    try:
        return [str(a[0]) for a in o.GetActions(dbus_interface=ACTION)]
    except Exception:
        return []


def state_bits(o):
    """AT-SPI StateSet → set of state indexes that are on."""
    try:
        lo, hi = (int(x) for x in o.GetState(dbus_interface=ACC))
        out = set()
        for i in range(32):
            if lo & (1 << i):
                out.add(i)
            if hi & (1 << i):
                out.add(32 + i)
        return out
    except Exception:
        return set()


def do(o, verb):
    acts = actions_of(o)
    for i, a in enumerate(acts):
        if a.lower() == verb:
            o.DoAction(dbus.Int32(i), dbus_interface=ACTION)
            return True
    return False


def walk(bus, name, pred, path=ROOT, depth=0, out=None):
    if out is None:
        out = []
    if depth > 15:
        return out
    o = acc(bus, name, path)
    try:
        if pred(o):
            out.append(o)
    except Exception:
        return out
    try:
        children = o.GetChildren(dbus_interface=ACC)
    except Exception:
        return out
    for ch in children:
        walk(bus, name, pred, str(ch[1]), depth + 1, out)
    return out


def portal_app(bus):
    for n in bus.list_names():
        if not str(n).startswith(":"):
            continue
        try:
            root = acc(bus, n, ROOT)
            if name_of(root) == "xdg-desktop-portal-gnome":
                return n
        except Exception:
            continue
    return None


def dialogs_of(bus, app):
    return walk(bus, app, lambda o: role_of(o) == ROLE_DIALOG
                and name_of(o).lower() in DIALOG_TITLES)


def allow_switches(bus, app):
    return walk(
        bus, app,
        lambda o: role_of(o) == ROLE_SWITCH and "Toggle" in actions_of(o)
        and any(h in name_of(o).lower() for h in SWITCH_HINTS))


def share_buttons(bus, app):
    out = []
    for label in BUTTON_LABELS:
        out += walk(bus, app, lambda o, L=label:
                    role_of(o) == ROLE_PUSH_BUTTON
                    and name_of(o).lower() == L)
    return out


class MutterKeys:
    """Real keypresses via org.gnome.Mutter.RemoteDesktop — the internal
    session API gnome-remote-desktop uses. Keys land on the focused
    window, which is the portal dialog while it is up. Lazily opened
    because it is only needed when AT-SPI actions prove inert."""

    IF = "org.gnome.Mutter.RemoteDesktop.Session"

    def __init__(self):
        self.ses = None

    def _start(self):
        if self.ses is not None:
            return True
        try:
            sb = dbus.SessionBus()
            rd = sb.get_object("org.gnome.Mutter.RemoteDesktop",
                               "/org/gnome/Mutter/RemoteDesktop")
            path = rd.CreateSession(
                dbus_interface="org.gnome.Mutter.RemoteDesktop")
            self.ses = sb.get_object("org.gnome.Mutter.RemoteDesktop", path)
            self.ses.Start(dbus_interface=self.IF)
            time.sleep(0.2)
            return True
        except Exception:
            self.ses = None
            return False

    def tap(self, keysym):
        if self.ses is None:
            return
        for state in (True, False):
            try:
                self.ses.NotifyKeyboardKeysym(
                    dbus.UInt32(keysym), dbus.Boolean(state),
                    dbus_interface=self.IF)
            except Exception:
                pass
            time.sleep(0.05)

    def stop(self):
        if self.ses is not None:
            try:
                self.ses.Stop(dbus_interface=self.IF)
            except Exception:
                pass
            self.ses = None


def approve_once(bus, kb, verbose):
    """Returns True if a dialog was found and handled this pass."""
    app = portal_app(bus)
    if not app:
        return False
    dialogs = dialogs_of(bus, app)
    if not dialogs:
        return False
    title = name_of(dialogs[0])
    if verbose:
        print(f'dialog: "{title}"', flush=True)

    # 1) Turn every "Allow …" switch ON — verified against live state.
    for s in allow_switches(bus, app):
        if STATE_CHECKED in state_bits(s):
            continue
        do(s, "Toggle")
        time.sleep(0.4)
        # Re-walk: GTK may rebuild the row, so fetch a fresh object.
        s2 = next((x for x in allow_switches(bus, app)
                   if name_of(x) == name_of(s)), s)
        if STATE_CHECKED in state_bits(s2):
            if verbose:
                print(f'  toggle "{name_of(s)}" -> on', flush=True)
            continue
        # GNOME 50: Toggle action is inert — the dialog's default
        # widget is the switch, so default.activate flips it instead.
        if do(dialogs[0], "default.activate"):
            if verbose:
                print('  default.activate on dialog (switch fallback)',
                      flush=True)
            time.sleep(0.4)

    # 2) Affirmative button: AT-SPI Click first, keyboard fallback.
    for b in share_buttons(bus, app):
        do(b, "Click")
        if verbose:
            print(f'  click "{name_of(b)}"', flush=True)
        break
    time.sleep(0.8)
    if not dialogs_of(bus, app):
        return True

    # GNOME 50: button Click can be inert too — drive real keys.
    # The dialog owns focus, so Tab cycles switch → Cancel → Share.
    if kb._start():
        for _ in range(6):
            kb.tap(KEYSYM_TAB)
            time.sleep(0.3)
            shares = share_buttons(bus, app)
            if any(STATE_FOCUSED in state_bits(b) for b in shares):
                if verbose:
                    print('  keyboard: Return on focused Share', flush=True)
                kb.tap(KEYSYM_RETURN)
                break
    return True


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--timeout", type=float, default=60.0,
                    help="seconds to keep watching (default 60)")
    ap.add_argument("--once", action="store_true",
                    help="handle one dialog then exit")
    ap.add_argument("-q", "--quiet", action="store_true")
    args = ap.parse_args()
    verbose = not args.quiet

    bus = a11y_bus()
    kb = MutterKeys()
    deadline = time.time() + args.timeout
    seen = False
    try:
        while time.time() < deadline:
            if approve_once(bus, kb, verbose):
                seen = True
                if args.once:
                    break
                time.sleep(0.5)
            else:
                if seen:
                    break  # dialog handled and dismissed
                time.sleep(0.5)
    finally:
        kb.stop()
    if verbose and not seen:
        print("no portal dialog seen before timeout", flush=True)
    return 0 if seen else 1


if __name__ == "__main__":
    sys.exit(main())
