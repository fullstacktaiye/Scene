# The Copilot key on Linux

Findings from a Lenovo Yoga 7 2-in-1 14IPH11 running Fedora 44, KDE Plasma
6.7.4 on Wayland. This is the evidence behind Milestone 5's "best effort"
framing, and it is worth reading before promising Copilot-key support.

## What the key actually sends

The Copilot key is not a key. It is a chord: **Shift + Super + F23**. Nothing
in the hardware identifies itself as an assistant key — the machine here
declares no `KEY_ASSISTANT` and no `KEY_ALL_APPLICATIONS` on any input device.

## Why it cannot be bound in KDE as shipped

`xkeyboard-config` handles the chord deliberately, in `symbols/inet`:

```
key <FK23> { [ XF86TouchpadOff, XF86Assistant ], type[Group1] = "PC_SHIFT_SUPER_LEVEL2" };
```

The key type is `PC_SHIFT_SUPER_LEVEL2`, so Shift+Super selects level 2 and the
key emits `XF86Assistant`.

Qt has no key code for that keysym — there is no `Qt::Key_Assistant` in Qt
6.11's `qnamespace.h`. So the press reaches KDE as `Key_unknown`, and KDE's own
shortcut recorder can only display the modifiers: `Meta+Shift`. The key is
unbindable through `kglobalaccel`, and no amount of shortcut configuration
changes that.

Confirmed directly against libxkbcommon, the same library KWin uses:

| Configuration | Keysym for Shift+Super+F23 |
| --- | --- |
| Stock | `XF86Assistant` |
| With the override below | `F23` |

## Working around it

Override the key to a single level of plain `F23`, so the chord arrives as
`Meta+Shift+F23`, which Qt does understand.

`~/.config/xkb/symbols/custom`:

```
partial xkb_symbols "copilot_f23" {
    key <FK23> { type[Group1] = "ONE_LEVEL", [ F23 ] };
};
```

`~/.config/xkb/rules/evdev`:

```
! include %S/evdev

! option = symbols
  custom:copilot = +custom(copilot_f23)
```

The include must come **first**. Options are appended in rule order, and the
system rules pull in `inet(evdev)` — which redefines `<FK23>` and silently
undoes the override if it lands afterwards.

Then in `~/.config/kxkbrc`:

```
[Layout]
Use=true
Model=pc105
LayoutList=us
Options=custom:copilot
ResetOldOptions=true
```

KWin reads this at session start. There is no reload path: `org.kde.KWin
reconfigure` does not touch the keymap, and `/Layouts` only switches between
already-configured layouts. **A logout and login is required.**

To undo: `rm -rf ~/.config/xkb ~/.config/kxkbrc`.

## What this means for Scene

Scene must not claim Copilot-key support on the strength of a keycode being
declared. `KEY_F23` is declared by essentially every AT keyboard whether or not
the physical key exists, so capability bits prove nothing here. Only an
observed event does.

It also cannot assume the chord is bindable: on a stock KDE session it is not.
Detection needs to distinguish three states — no such key, a key that emits an
unbindable keysym, and a key that can be bound — and say which one applies.
The fallback shortcut has to remain the supported path in all three.
