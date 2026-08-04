# Changelog

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
