# Themes

A scheme is not only a palette. It carries thirteen colours *and* how the
program draws: how spectra are arranged, what the toolbar's buttons carry, how
the area under the trace is filled, and whether there are gridlines, a
background wash, a glow under the trace, shadows and rounded corners.

**Theme & Colours** (Display menu) edits all of it. A scheme can be named,
kept, and written to a file to send somebody.

## The rules a palette has to satisfy

A spectrum display carries several meanings at once — data, marked regions, a
comparison trace, library lines, the cursor — on one plot. Three rules, checked
by test for every built-in scheme rather than by eye:

1. **The data wins.** The spectrum has the highest contrast against the plot
   background of anything drawn, better than 7:1. Everything else is quieter.
2. **Hues are spread, not crowded.** The data roles sit far apart on the wheel,
   and the spectrum and the regions are near opposites so a marked peak reads
   against the trace it sits on.
3. **Never hue alone.** Each role also differs in lightness, or in saturation,
   or in how it is drawn — so the display survives any colour-vision
   deficiency, and print.

Saturated red is kept out of the data palette and reserved for alarms, so a red
thing on screen always means "look at me".

The contrast figures are WCAG: relative luminance at 0.2126/0.7152/0.0722, and
a ratio of `(L1 + 0.05) / (L2 + 0.05)`. **Theme & Colours** reports them for a
hand-edited palette, and warns when two roles are too close to tell apart.

## The scheme file

```json
{
  "name": "Bench",
  "colors": { "background": "#000040", "foreground": "#00ffff", "...": "..." },
  "style": { "layout": "Windows", "fill": "Solid", "grid": false, "wheel_zoom": 100 }
}
```

Colours are written as hex because the point of the format is that a person can
read and edit it. Both that and the older `[0, 0, 64]` form are accepted when
reading, so a palette tuned by hand under an earlier version still loads.

Every field has a default, and so does the whole `style` block, so a file need
only say what it means to change.

The style carries feel as well as looks. `wheel_zoom` is how much one notch of
the scroll wheel zooms, as a percentage of the keyboard's step: 100 means a
notch and Display/Zoom In move the same amount, 50 calms a twitchy wheel to
half that, 200 hurries a stiff one to double. The range is 25 to 400, and the
curve is the same at every setting - only its pace changes.

## Conductor, and where its colours came from

Conductor reproduces the look of the software these instruments have
traditionally shipped with. Its values were **sampled from a screenshot of that
software running on the bench machine**, not reconstructed from memory — an
earlier attempt done from memory had a green trace and orange regions, which is
not what it looks like at all.

| Role | Sampled | Notes |
|---|---|---|
| Plot background | `#000040` | navy, not black |
| Spectrum | `#00ffff` | cyan, filled solid to the baseline |
| Regions | `#ff0000` | |
| Cursor | `#ffffff` | a thin line, one pixel |
| Overview | `#c0c0c0` | the whole spectrum drawn small, in silver |
| Inset title strip | `#008080` | |
| Chrome | `#f0f0f0` | as it renders on Windows 10 |

Sampled on 2026-08-07 from `Screenshot 2026-07-31 140810.png` in the operator's
own `ORTEC-reference` folder, beside the manuals and the runtime — a capture of
Maestro-PRO driving a 926 on this bench. **The screenshot itself is not in this
repository and should not be**: it is a picture of somebody else's proprietary
interface, and this project has no licence to redistribute it. The measured
values above are recorded instead, which is what makes the claim checkable.

Two deliberate departures from the original:

- **The alarm is crimson, not red.** The original leaves red meaning both
  "region" and "alarm". Two reds meaning two things is exactly the collision
  rule 3 exists to prevent, so the alarm is taken far enough round the wheel to
  be told apart while still reading as an alarm.
- **The overview box is not yellow.** The original's yellow is a thin cursor
  inside the overview. This role is the rectangle covering whatever the
  expanded view shows, which for a spectrum viewed whole is the entire inset —
  and in yellow it flooded it.

Two things still differ and are not colours: the original's overview is flat
silver where this one shows marked regions in it, and the original draws no
grid where this is a per-scheme setting.

## Workspaces

Separate from schemes, and chosen from the corner of the status bar. A scheme
is how the program looks and is what travels between people; a workspace is
what it *shows*, and follows the job in hand.

- **Acquisition** — the clock, the dead time, the rate, and the preset that
  will stop the run. The region list and the nuclide lookup are put away.
- **Analysis** — the regions and the nuclide lookup. The presets and the
  stability trace go away; both describe a run that has already finished.
- **Everything** — the default, deliberately. Both of the others hide sections,
  and hiding a panel from somebody who has not asked for it is how a program
  earns a reputation for losing things.

A workspace decides the sidebar and nothing else. Whether a spectrum has been
pulled out of the tab strip into its own window is a property of that spectrum,
not of the workspace: it is remembered across a change of workspace and across
a restart, and it is not per-workspace.
