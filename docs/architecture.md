# Architecture

```text
mantaray-gui ──┐                        mantaray-cli ──┐
              ├── mantaray-report ──┐                 │
              ├── mantaray-jobs ────┤                 │
              ├── mantaray-device ──┼── mantaray-core ─┘
              └── mantaray-formats ─┘
```

Dependencies point one way: nothing below knows about anything above it, and
`mantaray-core` depends on nothing but `chrono`, `serde` and `thiserror`.

## The crates

**`mantaray-core`** is the model and the mathematics, with no input or output. A
`Spectrum` is channel counts plus the descriptors that file formats carry (times,
start, descriptions, calibrations, regions, mode). Around it sit energy and
peak-shape calibration, region bookkeeping, peak information, peak search,
smoothing, stripping, nuclide libraries, efficiency curves, quantitative analysis
with decay correction and detection limits, quality-assurance charts, and the undo
stack. Channels are zero-based everywhere.

**`mantaray-formats`** turns files into `Spectrum` values and back. Each format is
its own module with its layout documented at the top; `load_spectrum` and
`save_spectrum` dispatch on the extension. A shared bounds-checked `Cursor` keeps
the binary readers honest about truncated files.

**`mantaray-device`** hides instruments behind the `Mcb` trait: identity and
capabilities, the settings of the MCB Properties dialog, presets and their
evaluation, start/stop/clear, status, list mode and zero-dead-time spectra, and raw
messages. `SimulatedMcb` implements it with a seeded physics model, so a run is
reproducible. `advance` is the acquisition loop: poll, then stop when a preset is
satisfied. Around that sit the detector list, the sample changer and batch queue,
the alarm monitor and the dashboard tiles.

**`mantaray-jobs`** parses `.JOB` files into a `Job` of `Command`s and runs them
against a `JobHost`. Every host method has a default that reports the command as
unsupported *with its line number*, so a host implements what it can and a job
fails loudly rather than silently skipping work. `run` executes a whole job;
`Runner` executes one command at a time, which is what the desktop application uses
so the display keeps drawing.

**`mantaray-report`** formats results as text: the two ROI report layouts, channel
printouts, nuclide reports and CSV.

**`mantaray-cli`** is `mantaray`, a clap application over the libraries, with its own
small `JobHost`.

**`mantaray-gui`** is the desktop application, built on egui. It is a library with a
binary on top: `main.rs` only builds the window and hands over, so everything the
application does can be driven from a test.

- `viewmodel.rs` - what part of the spectrum is on screen, where the marker is and
  how the vertical axis is scaled. No drawing, so it is unit tested.
- `view.rs` - draws one spectrum window and returns what the user did as events.
- `theme.rs` - the palettes, and the colour measurements the tests hold them to.
- `dialogs.rs` - the menu bar, toolbar, status sidebar and every dialog.
- `app.rs` - state, the acquisition tick, the undo history and one `apply` for
  every command. `App::headless` builds it without egui, `App::draw` renders one
  frame into any `Ui`, and `App::advance_by` moves simulated time on by hand.
- `jobs.rs` - `impl JobHost for App`, so a job drives the application exactly as
  the menus do.

Two rules keep the interface testable. Drawing decisions that matter are functions
with names and return values rather than conditions buried in a paint call -
`App::visible_windows` reports which windows will be drawn, `place_info_box` decides
where the peak-information card goes - and the drawing code asks them, so a test can
too. And gestures that span frames keep their own state: the display remembers where
a drag began, because egui has already forgotten by the frame the button comes up.

## How a frame works

1. `tick` advances every running detector by the elapsed wall-clock time, stops
   those that have met a preset, moves the sample changer on and checks the alarm
   limits.
2. `jobs::step` carries out a few commands of any running job.
3. The panels and windows draw. Drawing never mutates the model: menus, the
   toolbar, the sidebar and the spectrum views all return `Action` values.
4. `apply` carries the actions out, which is the only place the model changes.

That split is what makes the borrow checker easy to live with here - drawing needs
a shared borrow of a spectrum that may live inside a detector, while the display
state needs a mutable one - and it means every command has exactly one
implementation, whether it came from a menu, a keystroke, the right-mouse menu or a
`.JOB` file.

## Undo

Commands that change or discard data call `push_undo(label)` first, which snapshots
the active spectrum into a bounded `UndoStack`. Undoing a buffer restores it in
place. Undoing a detector command opens a buffer window holding the recovered data,
because instrument memory is not written back to - the same route a recalled file
takes.

## Testing

Around 530 tests, of the kinds worth having:

- **Worked examples** for the manual's equations, with the arithmetic in the test
  so a reader can check it.
- **Round trips** for every writable format, plus a fixture directory that, when
  real instrument files are dropped into it, requires them to load, survive a
  native round trip, and show the expected lines at the expected energies.
- **Cross-format agreement**: a `.Spc` file and its `.Spe` conversion must give the
  same channels, times, date and calibration.
- **Determinism** for the simulator: the same seed gives the same spectrum.
- **Behaviour under test for the job engine**, against a recording host.
- **End-to-end command-line runs**, including a job that acquires, saves and
  reports twice.
- **The application itself, headless**: opening spectra and checking that what
  opened is visible, a whole working session of commands, real frames rendered
  through egui with no window, and synthetic pointer input for clicking and
  dragging. See the table in the README.
- **Whole jobs**: `workflows.rs` runs the sequences a person actually performs -
  calibrate from two lines and save and reopen; smooth, look, undo; search,
  calibrate, analyse, report; the same job in two orders. These catch what
  per-command tests cannot, which is one step quietly undoing another.
- **ORTEC's own example files**, read as they ship: every spectrum loads, every
  channel count is a power of two, and the peaks land on real lines - Cs-137 at
  661.7 keV, K-40 at 1460.8. Skipped with a message when the examples are not
  installed, so a machine without them still passes.
- **A monkey**: three deterministic seeds firing a thousand hostile actions each
  - NaN energies, reversed ranges, channels past the end, windows closed by an
  index that never existed - painting real frames along the way. A failing run
  replays exactly from its seed.

The interface suites are written against symptoms rather than functions. "A
spectrum recalled while one window fills the screen is the one shown" failed when
the maximise feature hid new windows; "the finished selection is reported when the
button comes up" failed because egui forgets the press position on that frame.
Both are the sort of fault only the person using the program would otherwise find.
