# MAESTRO parity

Every feature in the MAESTRO v7 user manual (A65-B32), where it lives in ORTSEAM,
and where ORTSEAM deliberately differs. Section numbers refer to the manual.

Legend: **yes** implemented · **different** implemented another way · **no** not
implemented, with the reason.

## 3. Display features

| Feature | § | State | Notes |
|---|---|---|---|
| Title bar, menu bar, toolbar | 3.2 | yes | `gui/dialogs.rs` |
| Detector pick list on the toolbar | 3.2 | yes | opens detector or buffer windows |
| ROI status area (Mark / UnMark) | 3.2 | yes | right of the menu bar, F2 cycles |
| Multiple spectrum windows | 3.2 | different | any number, not eight; windows float and are kept inside the spectrum area |
| Expanded spectrum view | 3.3 | yes | `gui/view.rs` |
| Full spectrum view, inset, always logarithmic, with the expanded region boxed | 3.3 | yes | click it to jump |
| Status sidebar: start, real, live, dead, presets | 3.6 | yes | plus total counts and preset progress |
| Marker information line | 3.2 | yes | channel, energy, contents, selection |
| Supplementary information line | 3.2 | yes | the bottom bar |
| Marker with mouse and arrow keys | 3.5.1 | yes | Shift moves ten channels |
| Right-mouse-button menu | 3.5.2, 4.8 | yes | same commands as the manual lists |
| Rubber rectangle | 3.5.3 | yes | drag selects a range; the menus act on it |
| Resizing and moving the full spectrum view | 3.5.4 | different | the inset scales with the window instead of being dragged |
| ROI, Peak and Library index buttons | 3.6 | yes | ROI and Library stepping, Ins/Del/Info |
| Tool tips | 3.4 | yes | on every toolbar button |

## 4.1 File

| Feature | § | State | Notes |
|---|---|---|---|
| Recall | 4.1.2 | yes | any supported format, into a buffer window; also File/Recent, File/Open path, and dropping a file on the window |
| Save, Save As | 4.1.3 | yes | format chosen by extension |
| Export | 4.1.4 | different | writes ASCII or CSV directly instead of calling an external program |
| Import | 4.1.5 | different | reads any supported format directly |
| Print | 4.1.6 | yes | File/Print builds a printable page - the plot on the Paper palette, the region report, the seven-a-line channel printout - and opens it in the browser, whose print dialog does the spooling to paper or PDF |
| ROI Report, paragraph and column | 4.1.7 | yes | `report/src/lib.rs`, same fields and order |
| Report to file or on screen, displayed regions only | 4.1.7 | yes | |
| Compare | 4.1.8 | yes | Shift with up and down offsets the trace, Esc clears it |
| Settings: default save format, ask on save | 4.1.1 | different | the format follows the extension; no ask-on-save prompt |
| Settings: export and import programs | 4.1.1 | no | not needed, the formats are built in |
| Settings: directories | 4.1.1 | different | the file dialogs remember where you were |
| About | 4.1.10 | yes | Help/About |

## 4.2 Acquire

| Feature | § | State | Notes |
|---|---|---|---|
| Start, Stop, Clear, Copy to Buffer | 4.2.1-4.2.4 | yes | Alt+1, Alt+2, Alt+3, Alt+5 |
| List Mode | 4.2.5 | yes | events recorded and histogrammed together |
| View ZDT Corrected | 4.2.7 | yes | F3; corrected and error spectra |
| Download Spectra | 4.2.6 | yes | Acquire/Download Spectra pulls every stored spectrum into buffer windows; Acquire/Store to Instrument Memory fills the store (field mode) |
| MCB Properties: Amplifier | 4.2.8 | yes | gains, rise time, flat top, pole zero, pile-up rejection |
| MCB Properties: ADC | 4.2.8 | yes | conversion gain and discriminators |
| MCB Properties: Presets | 4.2.8 | yes | real, live, ROI peak, ROI integral, uncertainty, MDA |
| MCB Properties: High Voltage | 4.2.8 | yes | target, enable, measured |
| MCB Properties: Stabilizer | 4.2.8 | yes | settings are stored and reported |
| MCB Properties: About and Status | 4.2.8 | yes | identity, capabilities, live status |
| Optimize, pole zero | 4.2.8 | yes | the routines run on the simulated instrument's clock; each simulated preamp has its own right answer that Optimize finds |
| InSight virtual oscilloscope | 4.2.9 | yes | Acquire/InSight: the shaped pulse from the live rise time and flat top, the pole-zero tail from the instrument's own error, knobs bound to the amplifier, and the auto routines a button away |

## 4.3 Calculate

| Feature | § | State | Notes |
|---|---|---|---|
| Settings: FW(1/x)M, sensitivity, background points | 4.3.1 | yes | `core/settings.rs` |
| Calibration, 2 points linear, 3+ quadratic, up to 96 | 4.3.2 | yes | with residuals and Destroy Calibration |
| Calibration units, up to four characters | 4.3.2 | yes | |
| List Data Range | 4.3.3 | yes | start, duration, Increment, Restore |
| Peak Search | 4.3.4 | yes | multi-scale Mariscotti, sensitivity 1-5 |
| Peak Info | 4.3.5 | yes | equations 17-21, with a Gaussian fit and library match |
| Input Count Rate | 4.3.6 | yes | |
| Sum | 4.3.7 | yes | selection, region or whole spectrum |
| Smooth | 4.3.8 | yes | equation 23, undoable |
| Strip | 4.3.9 | yes | factor or live-time ratio, negative adds, undoable |

## 4.4 Services

| Feature | § | State | Notes |
|---|---|---|---|
| JOB Control | 4.4.1 | yes | browse, view, run, terminate |
| Library file: select file and peak | 4.4.2 | yes | with an editor for nuclides and lines |
| Sample Description | 4.4.3 | yes | saved with the spectrum |
| Lock / Unlock Detector | 4.4.4 | yes | password and owner |
| Edit Detector List | 4.4.5 | yes | add, open, numbers 1-999 |

## 4.5 ROI

| Feature | § | State | Notes |
|---|---|---|---|
| Off, Mark, UnMark | 4.5.1-4.5.3 | yes | F2 cycles; dragging marks or unmarks |
| Mark Peak | 4.5.4 | yes | Insert, three FWHM about the marker |
| Clear, Clear All | 4.5.5 | yes | Delete; both undoable |
| Auto Clear | 4.5.6 | no | peak search adds to the existing regions, as the manual describes; clear them yourself |
| Save File, Recall File | 4.5.7 | yes | `.Roi` channel pairs |

## 4.6 Display

| Feature | § | State | Notes |
|---|---|---|---|
| Detector, Buffer | 4.6.1 | yes | F4 opens a buffer |
| Logarithmic, Automatic | 4.6.3 | yes | keypad `/`, and `A` for automatic |
| Baseline Zoom | 4.6.4 | yes | |
| Zoom In, Zoom Out, Center | 4.6.5-4.6.7 | yes | keypad `+`, `-`, `5`, and the wheel |
| Full View | 4.6.8 | yes | |
| Isotope Markers | 4.6.9 | yes | library lines drawn across the display |
| Preferences: Points, Fill ROI, Fill All | 4.6.11 | yes | Fill All is the default |
| Preferences: Spectrum Colors | 4.6.11 | yes | every colour, plus a light scheme |
| Preferences: Peak Info font and colour | 4.6.11 | no | the peak-info window follows the application style |

## 4.7 Window

| Feature | § | State | Notes |
|---|---|---|---|
| Cascade, Tile Horizontally, Tile Vertically | 4.7 | yes | arranged once, then free to move and resize |
| Arrange Icons | 4.7 | no | windows do not minimise to icons |
| Multiple Windows | 4.7 | different | always on |
| Window list | 4.7 | yes | |

## 5. Keyboard commands

Implemented: arrows and Shift+arrows, PageUp and PageDown, keypad `+`, `-`, `/`,
`5`, Insert, Delete, F2, F3, F4, Esc, Alt+1/2/3/5, Ctrl+O, Ctrl+S, Ctrl+Shift+S,
Ctrl+Z, Ctrl+Shift+Z, Ctrl+Y. F1, or Help/Keyboard Commands, lists them all in
the application.

Different: keypad `*` (automatic scaling) is `A`, because egui does not report a
separate keypad asterisk. Ctrl+F1..F12 detector selection is the toolbar pick
list and the Display/Detector menu.

## 6. JOB files

The whole command set of §6.5 parses and runs: acquisition, presets, files,
regions, reports, library, locking, sample changing, loops, list-mode ranges and
raw instrument messages. Variables (`$(FullPath)`, `$(FullBase)`, `$(FileExt)`,
`$(FileDir)`, `$(McaDir)`, `$(CurDir)`, `$(Loop)`, `$(Loop1)`, `$(ShortPath)`,
`$(ShortBase)`, `$(Password)`, `$(Owner)`, `$(CR)`, `$(LF)`, `$(FF)`, `$(ESC)`,
`$(Bel)`) and the `???` loop counter behave as documented, including "a value of
0 executes once" and "a LOOP with no END_LOOP executes once".

`LOOP SPECTRA` and `VIEW` run against the instrument's stored spectra: `VIEW n`
opens stored spectrum `n` in a buffer window (`VIEW 0` shows the live data), and
`LOOP SPECTRA` makes one pass per stored spectrum. `SEND_MESSAGE "STORE"` fills
the store from a job.

`START_OPTIMIZE`, `START_PZ`, `STOP_PZ`, `WAIT_AUTO` and `WAIT_PZ` run the
automatic tuning routines on the instrument and wait for them, as on hardware.
`RUN` and `RUN_MINIMIZED` launch real programs; `WAIT "program"` launches one
and waits, failing the job with the exit status when the program fails. `ZOOM 3`
fills the spectrum area with the active window, other states restore it, and
`ZOOM: x y w h` places the window at a rectangle.

The whole §6.5 command set now runs.

## 7. Utilities

| Utility | State | Notes |
|---|---|---|
| WinPlots | different | the display itself, plus Export and the report viewer |
| Nuclide Library Editor | yes | built into Services/Library file |
| TRANSLT | yes | `ortseam convert`, with `--columns`, `--no-channels`, `--no-header` |

## Real instruments

The pick list holds two kinds of detector side by side: the built-in simulator,
and a **network instrument** - an MCB at the far end of a TCP connection,
speaking the ASCII `SET_`/`SHOW_` dotted-command dialect that MAESTRO documents
for `SEND_MESSAGE` and the simulator answers. `ortseam serve` exposes a
simulator that way, so one ORTSEAM can drive another's instrument across the
room; the Detector List's "Add a network instrument" connects to it, or to
anything else speaking the dialect. The protocol layer is exercised against a
scripted transport (every byte checked) and over real TCP against a served
instrument; the transport is pluggable, so a vendor-library backend (ORTEC's
CONNECTIONS/UMCBI) can be added without touching the protocol. Validation
against physical ORTEC hardware is pending bench time - the code is arranged so
that day is configuration, not surgery.

## Deliberate departures from the manual

**Equations 20 and 21.** The printed net-area expressions scale the background by
`h - l - (n - 1)` channels, which disagrees with equation 19 - it sums
`h - l + 1 - 2n` channels - and over-subtracts. On a test peak holding exactly 370
counts above a flat background the printed form gives 340, and a flat region gives
a net area of -30 instead of 0. ORTSEAM uses the number of channels equation 19
actually covers, which reproduces both known answers, and the same width goes into
the uncertainty. See `core/src/analysis.rs`.

**Peak-shape calibration.** Treated as a polynomial in channel, which is what real
files hold: `[2.4946, 1.745e-3, -1.9356e-7]` at 0.361 keV per channel gives 1.80
keV at 662 keV and 2.27 keV at 1332 keV. A square-root form would give 0.93 keV,
which no germanium detector achieves.

**Peak search width.** An uncalibrated search marks three FWHM rather than the
"width of the peak as determined by Peak Search", because the background points of
equations 17-21 have to fall outside the peak for the net area to be right. The
region is never narrower than `2n + 1` channels.

## Beyond MAESTRO

- **Undo and redo** for Clear, Smooth, Strip, region edits and peak marking.
  Instrument memory is never written back to: undoing a detector command recovers
  the data into a buffer window.
- **Peak information drawn in the plot** where MAESTRO opens a separate dialog: the
  region shaded, the straight-line background dashed between its anchor points, the
  fitted Gaussian over the peak, the width marked at half maximum, and a card
  carrying the numbers, with a leader line to the peak it describes.
- **Peak carets** over every marked region, named from the working library where
  the energy is recognised and by energy where it is not, with labels dropped
  rather than overlapped.
- **One window filling the spectrum area** (Ctrl+M, the header button, or the
  Window menu). Opening a spectrum while the area is filled shows the new one, so
  a recall is never invisible.
- **Four colour schemes** with the palette rules checked by tests rather than by
  eye: the data has the highest contrast of anything drawn, the five data hues sit
  far apart, warm pairs also differ in lightness, and saturated red is reserved for
  alarms.
- **Finer zoom and modern navigation**: twenty percent a step rather than halving
  and doubling, on the buttons, the keyboard, the wheel and a trackpad pinch.
  The wheel zooms about the channel under the pointer, so the peak being looked
  at stays put; a middle-button drag pans the view; and the overview inset
  answers a click by jumping, a drag by panning, and a double-click by showing
  the whole spectrum. Escape peels the topmost layer - dialogs first, then the
  plot's overlays.
- **A keyboard reference in the application** (F1), and a Regions list that
  highlights and follows the region the marker is in.
- **The overview inset carries the regions and the marker**, so it answers
  "where are my regions?" and "where am I?" even when the expanded view is
  zoomed somewhere else entirely.
- **Drag a file onto the window** to open it, and a recent-files list, so a
  spectrum can always be opened even where a system file dialog will not appear.
- **Files opened by their contents** when the name says nothing - `SPECTRUM.001`,
  `run3.dat`, or no extension at all.
- **File/Export Plot** writes the plot as it looks - zoom, scale, regions, named
  peaks, palette - as a self-contained SVG picture for reports.
- **Go to energy** (a toolbar box), **Ctrl+Tab window cycling** that pages
  through maximised spectra, and a hover readout that names the library line
  under the pointer.
- **Settings that survive a restart**: theme, hand-edited colours, recent files
  and simulation speed.
- **Automatic energy calibration**: pick a source in the Calibrate dialog and the
  application searches for peaks, tries every pairing against the nuclide's line
  energies, and fits the calibration that lines at least three of them up -
  refusing to guess when nothing agrees. On the real Eu-152 spectrum in the test
  fixtures, the automatic result lands within 1 keV of the instrument's own
  calibration at every major line.
- **Efficiency calibration** and activities in becquerel, with decay correction to
  a reference date and for decay during the count, weighted means across a
  nuclide's lines, and detection limits. The curve can be **measured from a
  certified source** in one press: with the source's spectrum open, the
  certificate's activity, uncertainty and age typed in, every library line found
  becomes a decay-corrected efficiency point and the curve is fitted - on the
  real Eu-152 fixture the measured curve has germanium's true falling shape.
- **Quality-assurance control charts** for centroid, width and count rate, with
  warning and action limits.
- **Multi-detector dashboard**, alarm limits on dead time and count rate, and a
  **batch queue** driving the sample changer.
- **A command-line workbench** for everything that does not need a display.
- **Cross-platform**: Windows, Linux and macOS.
- **A lossless native format**, CSV export, and a list-mode container that can be
  sliced by time - and **replayed**: the List Data Range dialog scrubs and plays
  the acquisition back, the window's spectrum following the moving slice.
- **Real control charts and a drawn efficiency curve**: the QA dialog shades the
  warning and action bands in the palette's status colours and judges each
  point; the efficiency dialog draws the fitted log-log curve through its
  points, so the classic HPGe knee - or a bad fit - is visible at a glance.
- **Honest numbers without an efficiency curve**: the nuclide analysis reports
  count rates (summed per nuclide) under its "cps" heading rather than zeros,
  and detection limits follow suit - Currie's limit as a count rate instead of
  a silent zero.
- **An unsaved-changes question** before a modified buffer's window closes, and
  dialogs that cascade rather than stack. The title-bar close button asks the
  same question File/Exit does when spectra are unsaved or a count is running.
- **The analysis table navigates**: click a nuclide's name and the display
  jumps to its strongest line with the peak card open; the Regions list and the
  plot cross-link the same way.
- **`calibrate --auto Eu-152` on the command line**: the automatic energy
  calibration is scriptable, so a folder of source spectra calibrates in a
  shell loop.
- **Explorer integration on Windows** (Services menu): per-user file-type
  registration so double-clicking a `.Spe`, `.Chn` or `.Spc` opens it here,
  with an unregister that removes only ORTSEAM's claim.
- **A crash report** written to the temporary directory if the application
  ever panics - the message, location, platform and backtrace in one file
  worth attaching to a bug report.
