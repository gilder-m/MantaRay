# Changelog

## Unreleased

**The bench check, run (2026-08-06).** The preset work below was written
against doubles and marked as not yet run on the 926; it has been now, and it
holds: `probe` finds the adapter, `SHOW_PRESETS` read back the very 300-second
live preset the instrument was found holding, `SET_PRESET_LIVE 2` round-tripped
and stopped the count at exactly LT=2.00, and `START` against that satisfied
preset was accepted with the clocks frozen - the silent refusal, reproduced.
Pinning by `--device` opens the named adapter and refuses a serial that is not
there; a detector opened twice learns its serial and then matches it. The bench
check is kept as `crates/ortseam-device/tests/bench_926.rs`, ignored by default
because it needs the instrument. Three things it found:

- *Dead time could read below zero.* The two clocks are separate commands, so
  they are sampled a round trip apart, and the one read second has run on: at
  a low count rate the bench 926 reported `RT=1.00 LT=1.02 DT=-2.00%`. Real
  time is now read second, which puts the skew in the direction the arithmetic
  survives, and the result is clamped - a negative dead time is a sampling
  artefact, not a measurement.
- *A helper that refuses to start said why to nobody.* Its reason goes to
  standard error, which a window with no console behind it never shows, so the
  operator saw only "the bridge stopped". Standard error is now relayed line by
  line as before *and* kept, so the failure is reported as "the bridge stopped:
  no adapter with "08134079" in its serial number".
- Two doc comments had been separated from what they document by code inserted
  between them, leaving `spawn_program` and `close_window` undocumented and
  their text attached to a test module and to `remember_serial`.

**A detector can be named, and cannot be confused with another one
(2026-08-06).** Three related things, all about knowing which instrument you
are talking to.

*Detectors can be renamed.* The Detector List's name column is an edit box:
type over it and the new name reaches the saved pick list, the open
instrument, its window title, and the detector fields of every spectrum saved
from it afterwards. A name that means something to whoever reads the file next
year - "Bench HPGe" - beats the model and serial the scan happened to print.

*A local instrument is pinned to its adapter, not to a position on the bus.*
Away from ORTEC's configured detector numbers, `serve N` means the Nth adapter
the bus enumerates, which is a position and not an instrument: plug in a
second adapter, or replug the one there, and the same entry can lead somewhere
else. An entry now remembers what the instrument called itself and hands that
to the helper as `--device`, which both the Windows and the libusb bridge
already understood. The one case where that would be wrong is handled: through
ORTEC's library an instrument reports the detector number that selected it,
and a number is not an adapter serial, so an identity that is only the number
already used is not treated as a pin.

*And the identity is checked on every open.* Whatever the instrument reports
in its configuration reply is compared against what the entry remembers -
learned on the first successful open - and a mismatch refuses the connection
by name ("this is instrument \"11217584\", not \"08134079\" - the adapters may
have been swapped or replugged") instead of quietly opening the wrong detector
and labelling its spectra with the wrong name. An instrument that reports no
serial is nothing to check against rather than evidence of a mix-up, so those
still open.

**A preset the instrument is already holding is visible, and cannot silently
refuse a count (2026-08-06).** Found on the bench: a 926 was still holding a
300-second live preset from an earlier session, with its live clock stopped
at exactly that value. ortseam could only ever *write* presets - it opened
with an empty Presets tab, because nothing ever asked the instrument what it
was holding - and then `START` was accepted and the instrument did not count,
because the preset was already satisfied. The instrument says nothing about
either: a valid `START` and an ignored one look identical on the wire, and no
reply carries the preset registers unless they are asked for.

Both halves are closed. The bridge answers a new `SHOW_PRESETS`, reading all
four registers back (time presets in ticks, reported in seconds; counts as
counts), and a served simulator answers it the same way, so an instrument and
a simulator behave alike. Connecting asks the question and shows what comes
back, so the Presets tab reflects the instrument rather than only this
session's edits; an older bridge that does not know the verb still connects,
with no presets shown, exactly as before. And starting an acquisition whose
real-time, live-time or ROI preset is already reached is now refused by name -
"the Live time preset is already reached - clear the spectrum, or change the
preset" - instead of appearing to work. Uncertainty and MDA presets are not
part of that check: they are host-side, and `advance` stops and reports them
on the next poll rather than the instrument quietly declining.

**Question and answer stay together on the bridge (2026-08-06).** Reviewing
the work below found that giving the bridge transport a ten-second timeout
had introduced a way for it to answer the wrong question. A timed-out
exchange left its late reply in the channel, so the next command read that
reply as its own and every reading afterwards was the previous one's - for as
long as the connection lasted, and silently: a status poll that lands on a
`DATA` line parses as no counts and not counting, which is exactly what an
idle instrument looks like. A stall that long needs nothing exotic; a
suspended laptop is past ten seconds on its own. The transport now counts the
replies still owed by commands it gave up on and discards that many before
believing an answer, so it either recovers alignment or keeps failing - never
returns a reply belonging to something else. Alongside it, the bridge reduces
each reply to the one line the protocol allows: passthrough relays the
instrument's own words now, a reply is only cut at a carriage return or a
NUL, and a lone line feed in one would have arrived as two answers and put
every later question with the wrong one.

**The audit's open list, worked through (2026-08-06).** Everything the
2026-08-04 audit left in TODO.md is fixed except what needs the bench or a
product decision; TODO.md now holds only those. In severity order:

- *Global shortcuts stand down while typing.* Every unmodified key binding -
  Delete, Insert, Home/End, the arrows, `+ - / =`, `A`, `5` - now yields to
  a focused text field, as Escape always did. Pressing Delete to fix a typo
  in the Calibrate dialog's energy box no longer silently clears the region
  under the marker, and typing "511" into an energy box no longer recentres
  the view.
- *Job `WAIT`s wait, and job presets reach the wire.* On a real instrument,
  `WAIT n` now spends n wall-clock seconds and `WAIT` polls a few times a
  second until the preset stops the count - previously both returned in
  milliseconds, so the manual's own `START / WAIT 300 / STOP` pattern saved
  a near-empty spectrum; in the desktop application the job now parks
  between frames, so the interface stays alive for the whole count instead
  of freezing while hammering the wire. Job `SET_PRESET_*` goes through
  `set_presets` and reaches the instrument, where before it wrote only a
  client-side mirror that died with the process. The simulator keeps its
  faster-than-real-time fast-forward throughout.
- *A quoted job argument followed by a space.* `LOCK "pw" "owner"` parsed as
  password `pwowner` with an empty owner - a password the operator never
  typed, on a detector now locked with it. Whitespace now ends a quoted
  argument exactly as a comma does.
- *One corrupt reply cannot take down the client.* A malformed `SHOW_DATA`
  line - a huge declared count, a word that is not a number, fewer words
  than declared - is now a reported error that leaves the last good
  spectrum standing. Previously a huge count aborted the whole process
  inside an allocation, and a garbled word quietly became zero counts.
- *File readers refuse corrupt lengths instead of allocating them.* A
  `.Spe` whose `$DATA:` range declares nine hundred million channels, a
  list-mode file whose channel-count field was rewritten, an N42 whose
  runs of zeros are individually modest but cumulatively gigabytes, and a
  crafted `.Lib` whose 65535 nuclides all share one 65535-peak chain are
  all errors now, not aborts or hangs. The N42 element walk is a loop
  rather than recursion, so a crafted file cannot overflow the stack.
- *The Windows IOCTL path is sound on 64-bit.* OVERLAPPED was modelled as
  five pointer-sized words, which on x86_64 leaves the real `hEvent` null -
  the kernel signalled the file handle instead, ambiguous the moment two
  requests ever overlap. It is now a proper `#[repr(C)]` struct, correct on
  both widths. When a timed-out request cannot be withdrawn from the
  driver, the transfer buffers - now heap-owned for exactly this reason -
  are leaked rather than freed under a kernel that may still write through
  them, and the device is retired until reopened. Registry enumeration
  skips an unreadable key instead of hiding every device after it.
- *The bridge cannot hang the application.* A wedged helper process used to
  block the pipe read forever; answers now arrive through a reader thread
  with a ten-second bound, and closing a wedged bridge kills it after two
  seconds rather than waiting indefinitely.
- *Dialog state ends with its dialog.* The strip dialog no longer reports
  "stripped" over a failure; removing a calibration row no longer smears
  the deleted row's typing onto the row below, and half-typed calibration
  edits die when the dialog closes; the Report viewer and the Exit and
  Unsaved-changes questions stay above a maximized plot; the regions
  sidebar refreshes when the calibration, library or settings change, not
  only when counts do.
- *Windows are addressed by identity.* Closing a background window no
  longer silently retargets commands at whatever window happens to be last;
  the Window menu and double-click focus by window id, so a list that
  shifted mid-frame cannot activate the wrong one. A view that showed the
  whole spectrum keeps showing all of it when the conversion gain rises.
- *The first frame shows before the instrument scan runs*, so launching
  with no detector configured shows a window immediately instead of
  appearing to hang for the length of a synchronous probe.
- *The serve dialect answers with the instrument's own words.* A `$`- or
  `_`-spelled command passed through the bridge now relays the reply
  (SEND_MESSAGE can finally see its answer); record parsing checks the
  record letter, so a desynchronised version record can no longer read as a
  plausible clock; a UMCBI model name with a space no longer shifts the
  configuration line's fields.
- *Simulator edges.* Storing on a locked instrument no longer half-happens;
  clearing list mode keeps the file's description and calibration;
  `SEND_MESSAGE("SET_PRESET_…")` respects the same no-change-while-counting
  rule as the dialog; a served instrument's pole-zero completes while idle;
  the TCP server survives a poisoned lock and drops a silent client after
  a minute instead of blocking every later connection.
- *Job engine edges.* The line number reported after a job ends is the last
  command's, not the command count; the million-step runaway guard says
  what it is; the chaos corpus's bare `BEEP` (no such form - the manual
  gives `BEEP <freq>,<duration>`, `BEEP ID` and `BEEP "String"`) is fixed
  and the corpus now asserts it parses. UMCBI loading no longer leaks a
  module reference per failed attempt, unpins its DLL directory on failure,
  and resolves `mcbloc32.dll` to the copy already in the process.

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
stopped the instrument by itself at exactly LT=10.00 s. Uncertainty and MDA
presets are host-side calculations no instrument carries, and this was
recorded here as not working on a remote instrument at all; that was wrong.
They are evaluated from the mirrored spectrum on every frame and STOP is sent
when one is satisfied. What is really different from a time preset is where
the stop lives: the instrument's own registers hold a time preset and stop it
whether or not anything is watching, while these hold only for as long as
ortseam is running.

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
