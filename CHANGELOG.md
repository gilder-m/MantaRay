# Changelog

## Unreleased

**Linux drives real hardware (2026-08-05).** The libusb path met an ORTEC 926
for the first time - it had compiled for months and never moved a byte - and
every recorded wire-format assumption held on first contact: the `$F`/`$G`
records, the tick arithmetic, and a whole 4096-channel spectrum with its ROI
bits in about a fifth of a second. What changed to get there:

- *`ortseam-mcb serve` exists away from Windows.* The dialect-translation
  layer (`Session`) is compiled on every platform and a new `ViaDirect`
  backend carries it over libusb, so the desktop application drives a local
  instrument on Linux exactly as it does on Windows - Scan, Open all, and a
  live detector window, with no vendor software of any kind.
- *`probe` and `configure` answer on Linux* in the same block shape the
  Windows bridge prints, so the application's scan parses both without caring
  which platform answered. `usb` keeps the plain serial listing.
- *The application looks for `ortseam-mcb`* (no `.exe`) beside itself away
  from Windows.
- *The translation layer is now tested everywhere*: a bench double answering
  as the real 926 answered pins the status clocks, preset tick conversion,
  data reads and refusals on every platform, not only where the hardware is.

Opening the adapter needs a one-line udev rule (see the README); the first
failure was `errno 13` and nothing else, exactly as predicted in
[docs/ortec-hardware.md](docs/ortec-hardware.md). macOS still type-checks and
nothing more.

**File dialogs hid the very files they were for.** On Linux, Recall with the
`.Spe` filter showed nothing: ORTEC names its files capitalised - `.Spe`,
`.Chn`, `.Spc` - and GTK and the XDG portal match filter patterns
case-sensitively where Windows does not, so a lowercase-only filter hid every
real instrument file. Filters now carry each practical spelling, at the one
place the platform is spoken to, so every dialog in the application is fixed
at once.

Presets could not be typed in at all: ticking a preset's box un-ticked it on
the next frame. The Presets tab re-read the instrument's own presets every
frame, so a half-finished edit - ticked, but not yet applied - was erased
before the number could be typed. The edit now lives in the dialog until
Apply sends it (or Clear discards it), and switching detectors seeds it
afresh so one instrument's presets are never typed into another's.

Testing MCB Properties against the bench found the count presets never
arrived: the bridge passed `SET_PRESET_COUNT`/`SET_PRESET_INTEG` through
untranslated, and the 926 ignores an unknown verb with the same empty reply
it gives a valid one - so the bridge answered OK while setting nothing. The
real verbs, established on the bench by writing values and reading them back
exactly, are `SET_PEAK_PRESET` and `SET_INTEGRAL_PRESET`, in counts;
`SET_PRESET_CLEAR` now zeroes all four preset registers. The time presets
were verified whole: a 10-second live preset set through ortseam's dialect
stopped the instrument by itself at exactly LT=10.00 s. What still does not
stop a remote instrument: uncertainty and MDA presets, which are host-side
calculations no instrument carries - the dialog accepts them but nothing
evaluates them for a bridged or network detector yet.

The first double-click on the bench's own Cs-137 peak found another gap:
without a region or a shape calibration, Peak Info guessed the peak to be
eight channels wide and fitted a sliver of anything broader - and a NaI(Tl)
line can be a hundred channels wide. The peak search now measures the peak
under the marker from the counts themselves, and the fit takes the whole
peak, centred where the peak is. The eight-channel guess remains only as the
last resort when no peak can be found at all.

The first spectrum saved off the bench found a gap that was never
Linux-specific: a spectrum from **any** remote instrument - a network MCB, or
either bridge - saved with `DET# 0`, no detector name and a measurement date
of 01/01/1970, because the remote mirror never carried them. The detector's
number and name from the pick list now travel with its spectrum, the moment
START is accepted is recorded as the measurement date (a resumed count keeps
its original date; Clear resets it, as the simulator always did), and none of
it is lost if the instrument's conversion gain changes between polls.

**Audit fixes (2026-08-04).** A five-pass review of the whole workspace; what
it found and what was fixed:

- *Energy and shape calibration fitting.* A duplicated calibration point no
  longer breaks (or silently skews) the fit - repeated channels average, and
  the fit is least-squares over every point. The quadratic fit is solved in a
  centred, scaled coordinate, so 16k-channel spectra no longer shred the
  curvature coefficient. Auto-calibration refits exactly-three matches as the
  line the search validated instead of an exact parabola through
  tolerance-sized misses, and one reference line can no longer be claimed by
  two peaks (an unresolved doublet could previously stand in for two of the
  required three agreeing lines).
- *The uncertainty preset* now stops on the 1-sigma uncertainty of the **net
  peak area**, as the manual defines it ("calculated in the same manner as
  for the Peak Info command") - not on gross counts, which on a
  continuum-dominated region would stop acquisition far too early.
- *Quantitative analysis* reports why a requested thing could not be done
  instead of silently omitting it: a decay correction with no recorded start
  time, and each absent-nuclide MDA row that could not be built, now land in
  the report's notes.
- *QA control charts* flag a not-a-number measurement as Action instead of
  letting it sail through as in-control.
- *DPM protocol (in-house USB).* Partial frame sends, empty write
  acknowledgements and replies whose length word promises more than arrived
  are now errors instead of silent desynchronisation; `read_all` refuses
  reads that would wrap the sixteen-bit address space and return duplicated
  memory. The libusb backend uses cancel-on-timeout transfers throughout, so
  a timed-out transfer can no longer hand its stale completion to the next
  request as if it were the answer.
- *Report output.* CSV fields are quoted when they need it (a comma in a
  nuclide name no longer shifts every later column), and analysis notes are
  printed with the nuclide report.
- Half-life display no longer renders whole values with a trailing dot
  ("1248000000. y"); FW(1/x)M clamps its `x` to the documented 2..=99 on
  input as well as in the settings dialog.

The findings not yet fixed - with file, line, failure scenario and suggested
fix for each - are recorded in [TODO.md](TODO.md).

## 0.1.0

The first release. Everything in the MAESTRO v7 (A65-B32) manual is
implemented or deliberately improved - [docs/maestro-parity.md](docs/maestro-parity.md)
is the section-by-section accounting.

**The workbench.** Acquisition against a physics-based simulator or a network
instrument; PHA, list-mode and zero-dead-time modes; presets on time, counts,
uncertainty and MDA; two-view spectrum windows with marker, regions,
comparison traces, isotope markers and an in-plot peak-information card;
energy, peak-shape and efficiency calibration; peak search, peak info, sum,
smooth, strip; ROI and nuclide reports; the complete `.JOB` automation
language including `RUN`/`WAIT`, `LOOP SPECTRA`/`VIEW` and `ZOOM`.

**File formats.** `.Chn`, `.Spc`, `.Spe`, `.Roi`, TRANSLT-style ASCII, a
lossless native JSON and a time-sliceable list-mode container - with files
recognised by their contents when the name says nothing, and readers that
survive arbitrary corruption. Validated against genuine MAESTRO Pro spectra.

**Beyond the manual.** Undo that recovers cleared instrument data into a
buffer window; automatic energy calibration from a known source's peaks;
list-mode replay; SVG plot export and browser-based printing with the plot
and reports on one page; QA control charts; a multi-detector dashboard,
alarms and a batch queue; four colour schemes held to measured contrast
rules; window tiling; a keyboard reference on F1; settings that survive a
restart; and a headless test suite that drives the real application - about
550 tests in all, including a monkey that hammers the session with hostile
actions and has already caught a crash.

**Care in the corners.** The analysis table links every nuclide to its
strongest line in the spectrum; detection limits stay meaningful without an
efficiency curve (count-rate units, like the rest of the table); an idle
empty detector says what to do next; dragging a file over the window
announces "drop to open"; the title-bar close button asks the same
unsaved-work question File/Exit does; and the toolbar, header and sidebar
degrade gracefully at narrow widths instead of clipping.

**Real hardware.** Instruments are reached through a transport carrying the
ASCII `SET_`/`SHOW_` dialect. TCP is built in and proven against a served
simulator (`ortseam serve`).

Beyond that, ORTSEAM talks to a real ORTEC 926 over USB **using none of ORTEC's
user-mode software** - no `Mcbcio32.dll`, no `mcbloc32.dll`, no
`DpmUsbAddIn.dll`, only the kernel driver. Commands, clocks, configuration,
gain, mode, integrals, the dual-port memory and whole spectra all work, and the
readout was checked channel-for-channel against ORTEC's own library on the same
instrument: 8192 of 8192 identical, clocks matching to the millisecond, and the
totals agreeing with the instrument's own arithmetic. `ortseam-mcb` is the
32-bit bridge, and it can also drive ORTEC's libraries where they are
installed. The wire format is written down in
[docs/ortec-hardware.md](docs/ortec-hardware.md).

**ORTEC's own files.** Binary `.Lib` nuclide libraries are read as GammaVision
writes them, chain-walked rather than trusted in file order, and every library
that ships with MAESTRO loads.
