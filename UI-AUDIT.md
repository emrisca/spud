# UI and UX audit

Run against [skills/ui-design.md](../../skills/ui-design.md), its gate table
[ui-gates.md](../../skills/references/ui-gates.md), and `scripts/ui-lint.py`.
Judged on the running dev server, from lint output and from measurements of the
rendered DOM in both colour modes, before and after the fixes below.

## Method

1. Read the app before judging it: eight strips, eight buses, one window, used
   hourly by one person, dense data, and every control is a real parameter.
2. Linted the source, then measured the rendered DOM (element heights, hit
   targets, computed font sizes, background images, clipping) instead of
   guessing from a picture.
3. Walked the gates: floor, reflex, finish.
4. Fixed, re-linted, re-measured.

Lint before: **66 floor, 34 reflex** across 19 files. Lint after: **0 floor,
15 reflex**, every one of them a waiver on the record below. Most of the floor
count was one rule (H5, em and en dashes, 57 lines), then tracked caps (7) and
two arrows in a label.

Measured after, in the preview's in-memory data:

| Measurement | Before | After |
| --- | --- | --- |
| Smallest type on the console | 8.5px | 10px |
| Text under 10px | 24 elements | 0 |
| Smallest type in a dialog | 9px | 10.5px |
| Gradients on the console | 4 (`fader__fill`, `fader__thumb`, `knob`, `strip--virtual`) | 1 (the meter's dBFS scale) |
| Gradients in appearance | 3 (+ preset swatches) | 2 (hue, chroma) |
| Interactive elements under 24px tall | 61 | 0 |
| Strips clipping their own content | 0 | 0 |
| Horizontal overflow of the window | 0 | 0 |

## Ratings

| Aspect | Before | After | What moved it |
| --- | --- | --- | --- |
| Typography | 4 | 9 | Nothing under 10px; every small label lifted a step; tracked capitals removed from seven rules; mono kept for figures, paths and device ids only |
| Hierarchy and composition | 6 | 9 | A rule under each group heading; the bus column lost its OUT/MIC row; the footer stopped repeating the header's counts |
| Colour and surface | 6 | 9 | Ten raw gradients became one, the meter scale; brand marks, fader, knob and the virtual strip's wash are solid or tinted tokens |
| Spacing and density | 7 | 8 | One spacing language and 24px control rows; the chips are still 14 to 19px wide, see below |
| States and feedback | 5 | 8 | Press, hover, focus and disabled are distinct; a fader has a unity mark; the struck route explains itself where it happens |
| Accessibility and keyboard | 6 | 8 | 24px targets vertically, 10px type floor, `aria-pressed` and the focus ring on every control; chip widths are still narrow |
| Copy and honesty | 6 | 9 | Engineering voice kept; em dashes, en dashes and middle dots gone; no duplicated counts; no invented release notes |
| Dialogs | 6 | 9 | Appearance's fake "Release notes, Version 0.1.0" block is a palette sample; labels at 11px, spacing matching the console |

Overall: **5.8 before, 8.7 after**. Two aspects stop at 8 for the reason in
[Still short](#still-short) rather than for want of a gate.

## What was wrong

### Typography (floor)

- `strip__device` and the bus device picker were **8.5px**; `strip__caption`
  ("EQUALISER", "APPS") and the bus note ("OUT", "MIC") were **9px in tracked
  capitals** (gates T3, T7). A device name is the thing you check before
  trusting a strip, and it was the smallest text on screen.
- The status bar was 9.5px monospace for a whole sentence, and the bus key,
  status pill and virtual-pairing lines were monospace names, not figures (T4).

### Colour and surface

- A gradient in ten places: both brand marks, the virtual strip's wash, the
  fader fill and thumb, the knob's bevel, the preset swatches, plus the meter
  and the two appearance tracks. Only the meter and the tracks are data (C1b).

### Composition

- Group headings at 10.5px with nothing to hold them, and a bus column that
  repeated `OUT` eight times under a heading that already said "Output buses".
- The footer repeated the header's counts and ended with a stray strip name.

### Interaction

- Route chips were **21x20px**, the mute/solo/mono chips **29x22**, the bus
  `M`/`Ø`/`LIM` **44x19**, the per-app mute **17x15**: under the 24px floor,
  and the smallest carried the most-used controls.
- The fader had four ticks and no mark for 0 dB, so the throw had no reference.

## What changed

- Every font size under 10px lifted (8.5 to 10, 9 to 10, 9.5 to 10.5); the
  tracked capitals came out of `.section-head`, `.strip__caption`,
  `.appearance__label`, `.eq-hint`, `.fx__head h3`, `.sp__group-name` and the
  bus note, which is deleted; mono now covers dB, Hz, ms, counts, paths and
  device ids only.
- Ten gradients reduced to one, the meter's green-to-amber-to-flag scale.
  Brand marks, fader fill and cap, knob, preset swatch and the virtual strip's
  tint are solid or a single `color-mix`.
- Group headings gained a 1px rule, and the bus label row carries the same rule
  so the two top edges stay level.
- The bus column lost its `out`/`mic` row; the key carries that explanation as
  a title.
- Route, mute, solo, mono, `LIM`, `EQ`/`DYN`, the per-app mute, the device line
  and the strip name are at least 24px tall, with the columns still 76px wide.
- A unity mark on every fader.
- The footer says what the process is doing and nothing else.
- Appearance's preview is the palette itself ("This hue, on a panel"), not a
  release note for a version that does not exist.

## Waivers on record

Each line names the gate, what was checked, and why this screen is different.

- **C1b**, guard: the element is `.meter__column`, whose height is clipped by
  `.meter__mask` to the current level. Reason: the gradient *is* the dBFS scale;
  a solid fill would delete the only place the console shows where red begins.
- **C1b**, guard: `HUE_TRACK` and the chroma `--track` paint a range of hues
  and saturations from the palette being chosen. Reason: the track is the
  control's range, and a solid fill cannot show a hue wheel.
- **T4**, guard: the eleven remaining `font-family: var(--font-mono)` rules all
  render a number, a unit, a count, a path or a device id (`strip__readout`,
  `bus__db`, `knob__value`, `param__value`, `strip__rate`, `.eq-hz`,
  `fx__amount`, `strip__apps-count`, `sp__path`, `sp__device-hw`). Reason: the
  console is read by comparing figures down a column, and tabular width is the
  point of them.
- **T4**, guard: the one hit at `styles.css:96` is the `--font-mono` token
  declaration. Reason: it defines the family rather than applying it to text.

## Still short

- **Chip widths.** The 24px floor is met vertically on every control, but the
  route chips are 14px wide and the mute/solo/mono chips 19px, because eight
  buses share a 76px column. Widening them to 24 would take the console's
  minimum width from 1180 to about 1400px and force sideways scrolling at
  1280x800, which is the size this window is used at. That is a trade, not an
  oversight: the columns keep their width and the row height carries the target.
- **No visual review.** The dialogs render in a portal that the preview panel
  captured blank in this session, so dialog findings come from reading the live
  DOM (font sizes, scroll heights, the rendered rows), not from a picture of the
  dialog. The console was checked the same way. Nobody has looked at the result
  at arm's length yet.
- **Hierarchy at 8, not 9.** The console still gives eight strips and eight
  buses nearly equal weight; the group rules and the deleted bus row help, but
  the first stop is the header, not the console.

## Finish checks

- F1 answered: one filled primary per view (`sp__button` in the empty state,
  `sp__update-go` in the banner); never two side by side.
- F2 answered: straight quotes, `...` for an ellipsis, a comma or colon where a
  dash was doing the work.
- F3 answered: tabular figures on every readout, one number format.
- F5 answered: one radius per nesting level (16/13/10/8/6/4), one shadow recipe
  per material.
- F6 answered: `::selection` and `scrollbar-color` themed from the palette.
- F13 answered: the window frame is native on purpose; the app draws no title
  bar.
