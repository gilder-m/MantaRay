# TODO

The open findings, seeded by the 2026-08-04 full-codebase audit (five
independent review passes, one per subsystem) and updated as work lands -
pruned 2026-08-06 after the audit-fix pass closed most of the list. Each
entry says where, what goes wrong, why it matters, and what the fix should
look like. What has been fixed is in the changelog, not here.

Severity: **H** = wrong results/crash/hang on realistic use, **M** = on
unusual-but-possible input, **L** = practice/robustness.

## From the 2026-11-05 review of `main`

A whole-codebase review of d3f5686. Most entries were proven with a failing
test or a CLI run against that commit; entries marked *(unverified)* came
from an audit pass with a concrete scenario but were not reproduced. The
review's first five fixes - invented bias writes, the GammaVision `.Spc`
calibration, NaN and "detected, 0 Bq" activities, and the pretend sample
changer - are in the changelog, not here.

### Analysis (`crates/mantaray-core`)

#### H: Energy to channel is up to 138 channels wrong after a fit through collinear points
`EnergyCalibration::channel` (calibration.rs ~84) solves the quadratic as
`(-b ± √disc) / 2c` whenever `c != 0`; a round-off `c` of 1e-19 loses every
digit. Fitting 512→256, 1536→768, 3584→1792 keV puts 661.657 keV at channel
1184.9 instead of 1323.3, while `energy()` stays right, so nothing on screen
looks wrong. MDA regions, efficiency-from-source, library markers and
jump-to-line all use it. **Fix:** the stable root `2(E - a) / (b + √disc)`.

#### H: Nuclides flagged "not in total" are counted in the total
`NuclideFlags::not_in_total` is read from the `.Lib` but
`QuantitativeReport::total_activity` never looks at it: Cs-137 1175 Bq plus a
flagged K-40 at 3752 Bq totals 4927. **Fix:** filter on the flag.

#### H: The activity uncertainty leaves out the efficiency calibration
`EfficiencyPoint::uncertainty` is never read. The ± is counting statistics
plus the library term only: 50,000 counts against a ±3 % certificate shows
about ±0.45 %. **Fix:** carry the curve's uncertainty at each line energy.

#### M: One NaN line energy in a library crashes the GUI on every frame
`EnergyIndex::ensure` sorts with `partial_cmp().unwrap_or(Equal)`, which can
panic since Rust 1.81; the GUI calls it from `draw`. The CSV reader accepts
`nan` and the `.Lib` reader has no finite check. Same comparator in
calibration.rs ~212 and library.rs ~400/~420. **Fix:** `f64::total_cmp`, and
refuse non-finite energies when reading.

#### M: A marked ROI that `peak_info` rejects vanishes without a note
`analyse` skips it with `continue`: a marked 659..664 region of thousands of
counts gives no peak, no unidentified entry and no note, and the report can
say "not detected" beside it. **Fix:** a note naming the region and why.

#### M: A sample quantity of 0 or less silently becomes 1 while the unit still says per kg
**Fix:** refuse it with a note, and range the GUI field.

#### M: Without an efficiency, the MDA is divided by the quantity but the activity beside it is not
Both are labelled cps. **Fix:** treat both the same way.

#### M: A QA chart whose history has zero spread calls any new value "in control"
qa.rs ~249. Recording one spectrum twice makes exactly that history, and
"Freeze limits" then fixes sigma at 0. **Fix:** no limits until the spread is
non-zero.

#### M: A reversed ROI in a native `.json` or session file crashes analysis
`Roi` derives `Deserialize`, skipping `Roi::new`'s ordering; `peak_info`
overflows (roi.rs ~32) and the sidebar calls it every frame. **Fix:** validate
on deserialise.

#### M: The absent-nuclide MDA assumes a 4-channel FWHM without a shape calibration
`mda_only_result`'s `fwhm_at(channel).unwrap_or(4.0)`: on a 16k HPGe with a
15-channel FWHM the MDA comes out about 46 % low. **Fix:** say so in a note,
or refuse without a shape calibration.

#### M: Efficiency is extrapolated outside the calibrated range with no warning *(unverified)*
An interpolated curve is clamped flat, a log-log one extrapolates freely, and
`range()` is never called. **Fix:** a note per line outside the range.

#### M: Peak search reports a peak outside the spectrum
On a real 8192-channel GammaVision spectrum `mantaray peaks` listed a peak at
channel -130.65 (-27 keV, FWHM 27.6). Not traced yet; likely a centroid fit
extrapolated off the low edge and never checked against the range.

### Instrument and wire (`crates/mantaray-device`, `crates/mantaray-mcb`)

#### H: After one slow reply, every later reply on a network instrument belongs to the previous command
`TcpTransport::exchange` has no late-reply guard (the bridge has `owed`).
After one timeout, `exchange("TWO")` returned the reply to ONE - for good.
STOP then "succeeds" on a DATA line and the mirror freezes.

#### H: A network instrument's count rate is always 0
`RemoteMcb::integrate` never sets `total_count_rate`, and the bridge hardcodes
ICR=0. A minimum-rate alarm trips on every remote detector; a maximum-rate one
never can.

#### H: `advance()` never reports a preset the instrument enforced itself
When the instrument reports ACTIVE=0 at its live preset, `advance` returns
`None`, so the Batch queue shows "counting" forever on real hardware and "Run
next sample" re-runs and clears the same sample.

#### M: On the ORTEC-library route the "serial" is the detector number
`ViaUmcbi::identity()` returns `self.number`, and remembered calibrations are
restored by serial alone with no channel-count check, outranking the
instrument's own. Another instrument enumerated as detector 1 inherits its
calibration; an 8k→4k change keeps the old keV per channel.

#### M: A network instrument's `CHANNELS=` is unbounded and allocates on the GUI thread
The DATA count is capped at 65536; the configuration value is not.
`CHANNELS=10000000000000` aborts the application.

#### M: Unbounded `read_line` on TCP
A peer that never sends a newline grows memory without limit; one that
trickles a byte every 2 s keeps resetting the 3 s timeout and freezes connect.

#### M: A blank command wedges the bridge *(unverified)*
The bridge skips blank lines without answering, but the client expects one
reply per line, so `SEND_MESSAGE ""` leaves every later exchange a reply
behind. Non-Windows route.

#### M: The UI thread waits on the instrument *(unverified)*
The bridge scan runs synchronously at launch with no timeout, connects take
up to 4 × 10 s, and every Start/Stop/Clear blocks on `Courier::exchange` with
no timeout.

### File formats (`crates/mantaray-formats`)

#### M: Saves overwrite in place, so a failed save destroys the previous file
Every spectrum, ROI, library and `.Clb` save uses `std::fs::write`; a
truncated `.Spe` reloads zero-filled and uncalibrated. The session snapshot
already writes a temporary and renames; do the same here.

#### M: `.Chn` drops calibration units
chn.rs hardcodes "keV" and skips trailer +28, where MAESTRO writes the units:
MeV comes back as keV.

#### M: `.Spe` drops channels when the start channel is not 0
The writer puts `"{offset} {len-1}"`, which the reader takes as first and last
channel: 1024 channels at offset 100 read back as 924.

#### M: Native `.json` spectra from v0.1 (ORTSEAM) no longer open
`missing field 'mantaray'`. **Fix:** `#[serde(alias = "ortseam")]`.

#### M: The ASCII reader accepts nearly any text
An `.Spe` saved as `.txt` reads as 80 shifted channels with no error, and the
two-column "channel counts" layout `sniff` recognises reads interleaved.

#### M: A missing live or real time is silently copied from the other
N42, `.Spe` and ASCII: an N42 with no LiveTime reads live = real, 0 % dead
time. N42 also parses only `PTnS`, so `PT59M50S` fails *(unverified)*.

#### M: Library CSV misreads ordinary spreadsheet output
`I-131,8.02d,…` gives half-life 0 (no decay correction) without a word.
*(unverified:)* quoted commas split, the layout is chosen by column count, and
an unparsable yield becomes 0.

#### L: The `.Spc` writer is offered though `can_write()` says no, and writes real time as ACQTIM
The calibration now goes where GammaVision keeps it; whether GammaVision opens
the file has not been tried.

#### L: Little of the `.Chn` trailer and format sniffing is pinned by tests
Mutation testing: a `-101` footer (calibration without a quadratic term) is
never read by a test, nor a `.Chn` with descriptions after the calibration,
and 18 mutants in `sniff` survive every test in the workspace.

### GUI (`crates/mantaray-gui`)

#### H: A running job's commands go to whichever window the operator activates
`job_detector()` is re-derived from `self.active` on every call and falls back
to detector 0 on a buffer. Clicking a buffer tab during a job's WAIT points the
job's SAVE and CLEAR at that buffer. **Fix:** fix the detector when the job
starts or at `SET_DETECTOR`.

#### H: GUI job steps that fail do not stop the job
START, STOP, CLEAR, FILL, SMOOTH, MARK, RECALL and REPORT call `apply_action`
and return `Ok(())` whatever happened, so an overnight LOOP writes wrong files
and ends "job finished".

#### H: GUI job `SET_PRESET_*` never reach a network or bridged instrument
They write through `presets_mut()`, the local mirror only, and Start does not
push them. An instrument holding LIVE=60 stops a job's 300 s count at 60 s,
saved as a 300 s count. The CLI host does send them.

#### H: A count recovered by Undo is marked unmodified, so Close and Exit do not ask
Exit then removes the session snapshot too. `restore_session`'s doc says
recovered windows come back modified; they do not.

#### H: Save overwrites the spectrum's own sample description with the Sample text
A background saved as "Background 24 h" is written as whatever the Sample
dialog holds.

#### H: Analysis results are not tied to their spectrum
`app.analysis` is never cleared: Print, the Analysis dialog and CSV export
show spectrum A's activities under spectrum B's title.

#### H: The Efficiency dialog comes pre-filled with invented numbers
(59.5, 0.08), (661.7, 0.02), (1332.5, 0.01): one press of Fit or Interpolate
gives becquerels from made-up data. *(unverified:)* rows with a decimal comma
are dropped without a word.

#### M: Baseline Zoom on an empty buffer kills the application
File/New buffer, then Display/Baseline Zoom: slice index out of range.

#### M: A right-click menu on an inactive window acts on the active one
Right-click → Clear on detector 2 clears detector 1.

#### M: Linear count-axis labels ignore Baseline Zoom
On screen, in SVG and in print: with baseline 4000 the "2.5k" gridline sits
at 5.5k.

#### M: The MDA preset region is fixed at channels 0-100 with no editor
The count stops on the MDA of noise and X-rays.

#### M: Errors swallowed so the status line says success
Stop (`let _ = detector.stop()`), "Run next sample"'s stop and clear (no
undo either), and *(unverified)* Clear presets, Start/Stop all, report save
and CSV export.

#### M: `MANTARAY_DEMO` ships in default builds and loads test nuclide data
`settle_layout` has no feature gate; it loads
`NuclideLibrary::sample_for_tests()` over the operator's library, and
"charts" fabricates an efficiency curve.

#### L: The Calibrate dialog's Units field is never applied
`calibration_units` is written and never read; maestro-parity.md says it
works.

#### L: The pointer and readout are up to half a channel off the bars at deep zoom
Bars span `end - start + 1` columns, `channel_at` uses `end - start`
intervals.

#### L: Smaller faults *(unverified)*
Calibration edit boxes go stale after auto-calibrate and a focus-out writes
the old points back; a detector rename is lost with two or more entries; job
`RECALL_CALIBRATION` skips `remember_active_calibration`.

### Jobs and CLI (`crates/mantaray-jobs`, `crates/mantaray-cli`)

#### M: A relative job path doubles the directory in `$(FullBase)` and `$(FullPath)`
`mantaray job jobs/recount.job` then fails with "No such file or directory"
and no path; an empty `$(CurDir)` points at the root.

#### M: With no instrument, SAVE, REPORT and EXPORT write empty output and exit 0
`SAVE "NOW.CHN"` wrote a 0-channel `.Chn`.

#### M: CLI `SET_DETECTOR n` treats every number as the one `--detector` instrument
A two-detector job counts both on one instrument; the second CLEAR wipes the
first.

#### M: `SET_RANGE` slices claim their full duration past the end of the recording
3000 + 900 s on a 3600 s file claims 900 s for 600 s of events.
*(unverified:)* `SAVE "*.lis"` after `SET_RANGE` writes the whole recording.

#### M: A huge `WAIT` panics both GUI and CLI
`Duration::from_secs_f64` on `WAIT 1e20`.

#### M: The CLI starts jobs it cannot finish *(unverified)*
CHANGE_SAMPLE, ZOOM, RUN and others fail only when reached, after counting.
**Fix:** check every command before line 1.

#### L: CLI `CLOSEMCBS` stops the acquisition; the GUI only closes windows
Against the command's own doc.

#### L: Job writes overwrite silently, and nothing makes a name unique per run
Re-running `examples/nightly-background.job` replaces last night's files.

#### L: The README shows the CLI on `--detector 192.168.0.40:2000` *(unverified)*
`--detector` is TCP-only, and the only TCP server is simulator-only.

### Security and release

#### M: Dropping a `.job` on the window, or passing one on the command line, runs it at once
Including `RUN` (any program) and `SEND_MESSAGE` (any instrument command); the
Job dialog's preview is skipped. "Opening" a received file runs it.

#### M: Windows loads the ORTEC DLL by bare name, before ORTEC's directories
`SetDllDirectoryA(NULL)` then `LoadLibraryA("Mcbcio32.dll")`, and the bridge
inherits the GUI's working directory. **Fix:** `SetDefaultDllDirectories` and
absolute candidate paths. Untested on Windows.

#### M: CI's "no simulator in shipped binaries" check can never fail
`cargo tree … | grep -q simulator` finds nothing even with the feature on, so
the negated check always passes.

#### L: Calibration units go unescaped into SVG and print HTML
`.Spe`, N42 and ASCII cut units to 4 characters, but a native `.json` does
not, so a crafted file can script the printed page.

#### L: Release workflow hardening
`contents: write` reaches the build jobs; actions are pinned by tag, not SHA;
no `--locked`, no tests and no CI gate before a tag is released; ci.yml has no
`permissions:` block.

### Tests

Entries marked confirmed come from mutation testing against the whole
workspace at dc01d73. Each named change was made to the code, every test in
the workspace ran, and all of them still passed.

#### M: About 30 "real file" tests pass having read nothing
`tests/fixtures/` holds only a README, and the skip message is swallowed, so
CI shows `ok`. No test reads a byte written by MAESTRO or GammaVision. Decide
which redistributable files can be committed.

#### M: `MockTransport` cannot see a command that was never sent
It answers "OK" when its script runs out and never checks the script was used
up; `a_job_preset_reaches_the_wire_not_just_the_mirror` still passes with its
bug reverted.

#### M: No test checks a number Peak Info shows
Confirmed. Peak Info's activity, its ±, the net-area uncertainty % and the
fitted Gaussian's FWHM and area can each be replaced by 0 or 1
(analysis.rs ~29-117). So can the sloped background line (~184-194), the
minimum-width check `2n + 1` (~142), the moment centroid (~288-300), the
half-maximum crossings (~250-280) and the Gaussian fit's own arithmetic
(~320-351). Both the report and the GUI's Peak Info show these values. Peak
search is barely pinned either: region edges (~413-431), `shape_consistent`
(can always say yes), the second-difference kernel and `describe_peak`'s
arithmetic. **Fix:** one test per number against a value worked by hand, on a
sloped background and a known Gaussian.

#### M: Presets and the network detector's lock are untested
Confirmed. In presets.rs:
- Flipping the MDA preset's `current <= target` (~152) makes it stop a count
  at once.
- The `limit > 0` guards (~123-150) can let a preset of 0 stop a count at once.
- `progress` can multiply where it divides (~167-191).
- `mda_of` can return 0.

On a network detector (remote.rs):
- `lock` can store nothing, `unlock` can accept any password, `is_locked` can
  return a constant, and `ensure_unlocked` can always pass (~483, ~722-742).
- `start`'s already-satisfied guard is tested for the live-time preset only:
  real time, ROI peak and ROI integral can be dropped from it (~603-606).
- The reconstructed start time (~466-475) and the `CAL=` hand-over at connect
  (~170-174) are untested.

`AnyMcb::set_presets` and `AnyMcb::lock` (any.rs ~102, ~202), the wrapper the
GUI and CLI call, can do nothing. So can the dashboard tile's status text and
its 90 % dead-time flag (lib.rs ~110-135), and the low-count-rate alarm's
"only once counting" condition (alarm.rs ~85), which the GUI evaluates every
frame.

#### M: CLI job commands are not tested to do anything
Confirmed: no other crate's tests reach this code. Even with
`--features simulator`, which the CLI's end-to-end tests need, each of these
in job_host.rs can be replaced by "do nothing, succeed":
- `RECALL`, `RECALL_ROIS`, `SAVE_ROIS`
- `STRIP`, `SMOOTH`
- `LOAD_LIBRARY`, `EXPORT`, `IMPORT`
- `STOP`, `CLEAR`, `LOCK`, `UNLOCK`
- `CLEAR_PRESETS`, `SET_PRESET_REAL`, `SET_PRESET_PEAK`,
  `SET_PRESET_INTEGRAL`, `SET_PRESET_UNCERTAINTY`

So can `mantaray serve`, `build-library` and `auto-calibrate`, and dropping
`acquire`'s live or real preset (run on d3f5686).

#### M: File readers: N42 calibration, the binary `.Lib`, list-mode slices
Confirmed:
- N42-2011's `<EnergyCalibration>` can be dropped (n42.rs ~187), as can the
  joining of split `<Data>` blocks (~173).
- The binary ORTEC `.Lib` reader is barely checked. Its minutes, hours and
  days half-life units can each be dropped to seconds (library.rs ~168-170),
  which makes those nuclides' half-lives 60 or more times too short.
- `ListModeFile::slice` and `duration` and `ListRange::is_valid` are untested.
- So are the `.Chn` start-time parse and much of the ASCII reader and writer.

#### M: Activities, calibration and efficiency are barely pinned by core's own tests *(unverified)*
Core's own tests miss these, but the whole-workspace check stalled on them,
so another crate's tests may catch some:
- quant.rs: `is_detected` can always say yes; `total_activity` and
  `bq_to_uci` can return 0; the decay-during-counting correction, the
  per-line decay corrections, the weighted mean and the absent-nuclide MDA can
  be changed.
- calibration.rs: energy to channel, `auto_calibrate`'s matching and the
  table's units.
- efficiency.rs: interpolation between points, `range()`, and a certified
  source's decayed activity and efficiency point.
- library.rs: `decay_constant` and `best_match`.

#### L: Finish the mutation run *(stopped 2026-11-06)*
In the whole-workspace check, 184 core mutants (quant, calibration,
efficiency, library, roi, qa, spectrum) timed out. Every one stalled just
after a fresh test binary launched, spread across 49 binaries, while macOS's
`syspolicyd` and XProtect were busy checking new binaries. They say nothing
about the code. Adding the terminal under Developer Tools in Privacy &
Security should stop those checks for binaries it launches; then re-run with
`--iterate`.

Also never run against the workspace:
- about 30 formats survivors (spc, spe, n42, nndc)
- the report (9) and jobs (85) survivors
- about 138 device survivors outside the simulator

The GUI (2,892 mutants) was not mutated, for time. mcb never is: only
hardware exercises it.

Run with `MANTARAY_MCB` unset:
`cargo mutants -p <crate> --test-workspace=true -j 2 --minimum-test-timeout 120`.

#### L: Where the suite's trust is thin
Every quantitative test runs at 1 keV per channel with a constant efficiency;
every Peak Info test has the same background on both sides; MDA values are
never asserted through `analyse`; the log-log interpolation test also passes
linear interpolation; about nine frame tests assert a shape count the empty
interface already passes; crash recovery is tested in memory only; fixed temp
paths collide between concurrent runs, and 15 of the 18 `temp_dir()` uses in
tests, in nine files, have no cleanup (library_files.rs:178 leaves a
`mantaray-binlib-<pid>` folder per run).

Mutation testing on `main` (d3f5686), each crate against its own tests only,
missed:

| Crate | Missed | Notes |
|---|---|---|
| core | 563 of 1,717 | 47 in the test-only `sample_for_tests` |
| device | 288 of 717 | 88 in the simulator or its random numbers |
| formats | 187 of 796 | |
| jobs | 110 of 234 | |
| cli | 49 of 94 | with `--features simulator`; 64 without |
| report | 15 of 49 | |

## Features

### Updater tool
Releases ship as an archive per platform with a `SHA256SUMS.txt`, and
updating today means downloading, checking and unpacking by hand. An updater
would check this repository's releases for a newer version, download the
operator's archive, check it against the release's sums and replace the
three binaries together. **Open questions:** whether it lives in the
application, the CLI or both; how a bench machine without internet is
handled; and that it must never replace anything mid-count. It makes the
release workflow a path onto every installation, so the release hardening
above comes first, and checksums from the same release prove integrity, not
origin: signing is worth deciding at the same time.

## GUI (`crates/mantaray-gui`)

### M: A freeze on Linux, seen twice including under release, remains unexplained
The GUI has stalled twice with a live detector window polling: once on
2026-08-05 under a **debug** build, and again on 2026-08-06 under a **release**
one. The second occurrence rules out the first explanation — a debug build's
saturated event loop — which was written here as though it settled the matter.
It did not.

What the second stall showed, while it was happening: the main thread was `R`
in userspace rather than blocked, the `mantaray-mcb` helper was idle at
`anon_pipe_read`, and the process held about half a core. It recovered on its
own after some minutes without being signalled. No stack was captured, so
there is still no root cause.

**Why it matters:** a spinning main thread that is not blocked is a live loop
somewhere in the frame, not a deadlock on the bridge — and the helper sitting
idle says the instrument is not what is holding it. Under release, with real
counting, this is a stall an operator meets mid-measurement.

**Next step:** the obstacle is capture, not reproduction. `ptrace_scope=1` on
the bench machine means gdb can only attach to its own descendant, so a
debugger cannot be brought to a stall already in progress — the program has to
be launched *from* gdb and left running, driven through a FIFO so the terminal
stays free. Keep such a session up during bench work and take `thread apply all
bt` the next time it stalls.

### L: Uncertainty and MDA presets stop a remote instrument only while the host runs
These are host-side calculations no instrument carries. They *are* evaluated
for bridged and network detectors — `advance` computes them from the mirrored
spectrum on every frame of the desktop application, and on every step of the
command line's `WAIT`, and sends STOP when one is satisfied (pinned by
`an_uncertainty_preset_does_stop_a_remote_instrument` in
`crates/mantaray-device/tests/remote.rs`). What is genuinely different from a
time preset is where the stop lives: a real- or live-time preset is set in the
instrument's own registers and stops it whether or not anything is watching,
while these stop it only for as long as mantaray is running and polling.
**Why it matters:** close the application mid-count, or lose the USB link, and
an uncertainty-preset acquisition keeps going on the instrument. **Fix:** say
so in the Presets tab — the preset is honoured, but by the host.

## Core (`crates/mantaray-core`) — deferred judgment calls

### M: `auto_calibrate` scoring still validates candidates linearly
The one-peak-per-line rule and linear-refit-at-3 are in (2026-08-04), but a
winner with 4+ matches is still refitted as a quadratic that the linear
scoring never validated; fine in practice, worth a second look if
auto-calibration ever misbehaves on curved detectors.

### L: `analyse` feeds whole-ROI background into Currie's B
`quant.rs` (~309): `peak.info.background` is background over the whole ROI,
where Currie's B is defined over the peak integration region; modestly
inflates per-line MDA for wide ROIs. Defensible as-is (ROIs are normally
drawn tight); revisit if MDAs read high against MAESTRO on the bench. The
2026-11-05 review put the difference from Gilmore's form at about 2× high
for narrow ROIs and 2× low for wide NaI ones *(unverified)*; no test pins
either choice.

### L: Overflow/edge theoreticals
`analysis.rs` ~608 `6*get(i)` in u64 can overflow near u64::MAX/16 counts;
`roi.rs` ~32/~57 `len`/`center` overflow at `end = usize::MAX` (all in-crate
callers construct clamped ROIs). Guard if ROI construction is ever exposed
raw.

## Formats — accepted format limits, documented in code

`.Roi` clamps regions above channel 32767 (a sixteen-bit signed format;
noted at the write site); `chn.rs` clamps negative i32 channel counts to 0
(harmless for round-trips). Neither is worth breaking the formats over.

## Hardware verification (needs the bench)

- **Re-verify the Windows IOCTL path against the 926** after the 2026-08-06
  rework: `usb.rs` gained the correct 64-bit OVERLAPPED layout, heap-owned
  transfer buffers that are leaked (never freed) when a cancel fails, and a
  wedged-device latch. It compiles under CI's Windows job, but the driver
  has not seen the new code. `mantaray-mcb usb`, `usbtalk SHOW_VERSION` and
  `usbspectrum` on the Windows side is the whole check.
- **Run the full-chain hardware test** when an instrument is idle: build
  i686 release `mantaray-mcb`, run `MANTARAY_MCB=<abs path> cargo test -p
  mantaray-device --test bridge_hardware -- --nocapture`.
- **macOS remains a type-check only**; the libusb path is proven on Linux
  (2026-08-05, PR #1) but has never run on a Mac. **Multiple adapters on
  one Linux bus** are likewise untried (the bench has one).
- **The exact-multiple/ZLP question** in `dpm.rs::read_in` has bench
  evidence on *both* backends: Windows read 8192-channel spectra (32768
  bytes ≡ 0 mod 4096) byte-identical with follow-up commands in sync, and
  the 2026-08-05 Linux run read 4096-channel spectra (16384 bytes)
  repeatedly through `transfer_blocking` with the live window polling in
  sync throughout. No stranded terminator has ever been observed. Keep the
  loop as it is; do not "fix" it on theory alone.
- **Optionally bind Windows to WinUSB** so the in-house path needs no ORTEC
  driver at all (today direct.rs is non-Windows because ORTEC's driver owns
  the device).
- **Job WAIT semantics against the bench**: the 2026-08-06 rework makes
  `WAIT`/`WAIT n` wall-clock for real instruments (CLI sleeps in 250 ms
  polls; the GUI parks the job and checks once per frame) and sends the
  CLI's job presets through `set_presets` so they reach the wire - the
  GUI's still do not (see the 2026-11-05 entries above). Proven against a
  scripted transport; worth one `START / WAIT 10 / STOP / SAVE` job against
  the real 926.
