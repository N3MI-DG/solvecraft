# 3D mouse

SolveCraft reads a 3Dconnexion 3D mouse (SpaceMouse) directly, with no driver to install. It's off
until you turn it on: tick **Use a 3D mouse** in Preferences › Navigation. Slide the cap to pan,
push or pull it to zoom, tilt it to turn the view up or down and twist it to spin; its buttons Fit
and go Home. Once it's on, its settings appear under it: speeds, smoothing, response, dead zone,
directions and what each button does.

While it's on, SolveCraft looks for a 3D mouse every couple of seconds, so you can plug one in,
or wake a wireless one, while it's running; turned off, it leaves the device alone. It uses the first one it finds. The desktop app reads 3D mice;
the web app doesn't.

## Windows

Nothing to install: plug it in, pair it over Bluetooth or connect the Universal Receiver, and
turn it on in Preferences. 3Dconnexion's driver (3DxWare) isn't needed, and can stay installed.
For programs it doesn't know, the driver turns cap movement into mouse-wheel scrolling;
SolveCraft ignores the wheel while the cap moves, so the two don't fight.

## macOS

Nothing to install either: turn it on in Preferences. SolveCraft reads the device directly,
alongside 3Dconnexion's driver if it's installed, without taking it over. 3D mouse support hasn't
been tried on a Mac yet; if yours does nothing, please [open an issue](https://github.com/bherbruck/solvecraft/issues) with
the model and how it's connected.

## Linux

On Linux the device belongs to root until a udev rule lets you read it. Without the rule the 3D
mouse does nothing. Install the rule once, from a source checkout (or copy a downloaded
[`71-solvecraft-3dconnexion.rules`](https://github.com/bherbruck/solvecraft/blob/main/packaging/linux/71-solvecraft-3dconnexion.rules)
from wherever you saved it):

```sh
sudo cp packaging/linux/71-solvecraft-3dconnexion.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger --subsystem-match=hidraw
```

SolveCraft finds the device within a couple of seconds, without a restart. The rule covers every
3D mouse with 3Dconnexion's own vendor id, over USB, Bluetooth and the Universal Receiver:
SpaceMouse Wireless and Pro Wireless, Enterprise, Compact and the current SpaceMouse.

Models made before 2011, and the wired SpaceMouse Pro, carry Logitech's vendor id instead, which
the rule doesn't match (matching all of it would open every Logitech keyboard and mouse too):
SpaceNavigator, SpaceNavigator for Notebooks, SpaceExplorer, SpacePilot, SpacePilot Pro,
SpaceMouse Pro (wired), SpaceTraveler, Spaceball 5000, SpaceMouse Classic, SpaceMouse Plus XT and
CADman. The rule file has a commented-out line for each, matching that exact product: uncomment
yours in `/etc/udev/rules.d/71-solvecraft-3dconnexion.rules` and reload the rules as above.

To check a device, run `ls -l /dev/hidraw*` with it connected: a `+` after its permissions means
the rule matched. The Flatpak also needs access to the device:
`flatpak override --user --device=all ai.storyteller.solvecraft`.
