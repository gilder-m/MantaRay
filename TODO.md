# TODO

The open findings, seeded by the 2026-08-04 full-codebase audit (five
independent review passes, one per subsystem) and updated as work lands -
pruned 2026-08-05 after the first Linux hardware run (PR #1) fixed some and
found others. Each entry says where, what goes wrong, why it matters, and
what the fix should look like. What has been fixed is in the changelog, not
here.

Severity: **H** = wrong results/crash/hang on realistic use, **M** = on
unusual-but-possible input, **L** = practice/robustness.

## GUI (`crates/ortseam-gui`)

### H: Global shortcuts fire while typing in text fields
`app.rs` `keyboard()` (~3351-3519) clones raw input at the top of the frame
and only guards Escape and Ctrl+C with `egui_wants_keyboard_input()`. Every
other binding is unguarded: Delete→ClearRoi, Insert→MarkPeak, Home/End→marker
jump, arrows→marker moves, PgUp/PgDn→scroll, `+ - / =`→zoom/log, `A`→auto
scale, `5`→center. **Why it matters:** pressing Delete to fix a typo in the
Calibrate dialog's energy box silently clears the ROI under the marker, so
Enter then records the bare marker channel instead of the ROI centroid —
defeating the centroid-calibration feature; typing "511" in a text box
recenters the view; typing `a` in Sample Description flips the vertical
scale. **Fix:** gate every single-key binding on the same
`egui_wants_keyboard_input()` check the Escape handler already uses (arrows
included; keep Escape's special handling).

### M: A debug-build freeze on Linux, seen once, remains unexplained
During the 2026-08-05 bench run the GUI froze once under a **debug** build
while a live detector window was polling. It did not recur under a release
build (CPU fell 93% → 19%), and no root cause was established — a debugger
could not attach without relaxing `ptrace_scope`. **Why it matters:** an
unreproduced freeze is an undiagnosed bug, not a fixed one; if it is a real
deadlock it may only be *slower* to appear in release. **Next step:** try to
reproduce under a debug build with a live instrument; if it recurs, attach
with `ptrace_scope` relaxed (or run under `gdb` from the start) and get a
stack.

### M: Stale `calibration_edits` after removing a table row
`dialogs.rs` (~2497-2564): edits are keyed by row index and only `resize`d;
removing row k leaves the deleted point's text on the rows below, and since
`EditCalibrationPoint` fires on any focus loss where text ≠ point, clicking
into and out of such a box rewrites a surviving point with the deleted
point's values. **Fix:** `remove(k)` on the edit vector when a row is
removed, and clear the vector when the dialog closes (the comment at ~2511
already claims this happens; nothing does it).

### M: Strip dialog reports success on failure
`dialogs.rs` (~3666-3671) pushes `Action::Status("stripped; Edit/Undo takes
it back")` unconditionally after `strip_from()`; queued actions are applied
after dialogs draw, so the success status overwrites the error status a
failed strip just set. A mistyped path yields "stripped" with nothing
stripped. **Fix:** make `strip_from` return success/failure and only queue
the success status on success.

### M: Three hand-rolled windows miss the z-order fix
`dialogs.rs` ~2620 (Report viewer), ~4298 ("Exit ortseam"), ~4345 ("Unsaved
changes") build their own `egui::Window` without
`.order(egui::Order::Foreground)`, which `dialog_window()` got. With a
maximized spectrum window, clicking the plot buries the Exit/ConfirmClose
question; the title-bar X then appears dead (each close request re-opens the
already-buried dialog). **Fix:** add the same `.order(...)` to all three.

### M: First-frame instrument scan runs while the window is hidden
`app.rs` ~1165-1170/~3174-3178 + `main.rs` ~85: the synchronous bridge probe
(`run_bridge` → `Command::output()`) runs during the first frame, and the
new hidden-startup logic only sends `ViewportCommand::Visible(true)` after
that frame completes. On a shipped build with no detector, launching shows
no window at all until the probe returns — the "appears to hang" state the
deferred scan was built to avoid, reintroduced by the startup-flash fix.
**Fix:** draw and show the first frame, then scan on the second (e.g. gate
the scan on `shown`), or make the probe async.

### M: Regions sidebar cache misses calibration/library/settings changes
`dialogs.rs` (~1396-1407): the cache fingerprint hashes active-window index,
total counts, live time and ROI bounds only. Calibrating changes none of
those, so rows keep their uncalibrated "ch NNN" text under the freshly
drawn "keV nuclide net" header until a count or ROI changes. **Fix:** fold
the calibration coefficients (and whatever library/settings the rows render)
into the fingerprint.

### M: Job WAITs block the UI thread; on real hardware the GUI freezes
`jobs.rs` (~166-180, ~329-339): no background thread — `wait_for_stop`
busy-loops `advance(detector, 1.0)` up to 2,000,000 iterations with no
sleep, and each iteration is instrument IO for a bridge-connected MCB, so a
`WAIT` with a real-time preset freezes the interface for the whole count
while hammering the instrument; `wait_program` blocks the frame on
`child.wait()`. Fine for the simulator; wrong for the bench 926. **Fix:**
run jobs on a worker thread with a channel back to the UI (or at minimum
poll once per frame instead of looping to completion inside one frame).

### L: `close_window` retargets `active` unconditionally
`app.rs` (~887-903): closing any background window reassigns `active` to the
last window, silently retargeting subsequent commands; also runs for a
`CloseWindow` with a nonexistent id. **Fix:** only move `active` when the
active window itself closed; ignore unknown ids.

### L: `set_length` keeps a stale narrow view when a spectrum grows
`viewmodel.rs` (~114-126): raising conversion gain 1024→8192 leaves the view
showing the first eighth. **Fix:** if the old view covered the whole old
range, cover the whole new range too.

### L: Index-based actions within one frame batch can shift
`app.rs` draw_windows/apply: `Activate(index)` after a `CloseWindow` earlier
in the same frame's batch can target a shifted index. Needs two interactions
in one frame. **Fix:** resolve actions by window id, not index.

## Windows IOCTL path (`crates/ortseam-mcb/src/usb.rs`)

### M: OVERLAPPED modeled as `[usize; 5]` is wrong on x86_64
Lines ~261, ~310, ~689: on 64-bit, OVERLAPPED is 32 bytes with `hEvent` at
byte 24 (index 3), not index 4; the comment "five words either way" is
false. Writing the event at index 4 leaves the real `hEvent` NULL, so the
kernel signals the *file handle* instead — ambiguous the moment two
operations are ever in flight, and the 64-bit path runs routinely in a dev
tree because `ortseam-device/src/bridge.rs` (~98) finds the sibling x86_64
`ortseam-mcb.exe` before the i686 candidates. **Fix:** a proper
`#[repr(C)]` struct (`Internal: usize, InternalHigh: usize, Offset: u32,
OffsetHigh: u32, hEvent: HANDLE`), correct on both widths.

### M: If cancellation fails, buffers are freed while the IRP may live
Lines ~357-381: on timeout, `CancelIoEx` and the follow-up
`GetOverlappedResultEx(..., 1000, ...)` results are both ignored; if the
driver never completes the request, `finish` returns and the local packet
Vec + OVERLAPPED are freed while the kernel can still write through them —
UB as METHOD_BUFFERED completion copies into freed memory. **Fix:** check
both results; if the request is still live after the grace wait,
`std::mem::forget` the buffers (leak, don't free) and mark the device
wedged.

### L: Registry enumeration stops at the first error
`usb.rs` ~149: any `RegEnumKeyExA` error (e.g. a >511-char key name) breaks
enumeration entirely, hiding later devices; skip the bad key instead.

### L: `milliseconds + 1000` can overflow u32
`usb.rs` ~712 — saturate instead (no current caller passes near-MAX).

## Serve/bridge/UMCBI (`crates/ortseam-mcb`)

### L: `record_number` never checks the record letter
`serve.rs` ~396: under any reply desync a `$F` (version) record parses as a
plausible clock/gain number. The test at ~435 is vacuous — it passes on its
second operand alone, and `record_number("$F0926-001")` actually returns
`Ok(926.0)`. **Fix:** take the expected record letter as a parameter and
refuse others; rewrite the test to pin that.

### L: Verbatim passthrough discards instrument replies
`serve.rs` ~113/~123: a `$…` or `_`-containing verb reaches the instrument
but the client is answered `OK`, so a SHOW sent via SEND_MESSAGE can never
see its answer. **Fix:** relay the instrument's reply.

### L: UMCBI `MODEL=` unsanitized in CONFIGURATION line
`serve.rs` ~162-167: the USB path applies `record_text`, the UMCBI path
embeds the model string raw into a space-delimited line; a model with a
space breaks field parsing. **Fix:** sanitize both paths the same way.

### L: umcbi.rs practice items
`load_from` leaks a module handle per failed `resolve` and never restores
`SetDllDirectoryA`; `local_paths`/`discover` call
`LoadLibraryA("mcbloc32.dll")` fresh (loads from an unpinned search path if
called before `Umcbi::load`); `start_time`'s `time == now` heuristic
(~188) discards a genuine start time equal to the current second.

## Jobs (`crates/ortseam-jobs`, `crates/ortseam-cli`)

### H: `split_args` corrupts space-separated quoted arguments
`parse.rs` ~254-269: `pushed_quoted` is only reset by a comma, never by
whitespace, so after a closing quote later characters append to the
*previous* argument and a later quote pushes an empty string. Verified:
`LOCK "pw" "owner"` parses as password `"pwowner"`, owner `""` — a job
locks the detector with a password the operator doesn't know;
`SET_RANGE "6/29/2012" "14:05:00" 900` fails on the mangled value. Comma
forms work, which is why every existing test passes. **Fix:** reset the
quoted flag on whitespace; add tests for space-separated quoted arguments.

### H: `WAIT <seconds>` never waits wall-clock time on real hardware
`ortseam-cli/src/job_host.rs` ~198-203: `wait_seconds` calls
`advance(mcb, seconds)`, which for `RemoteMcb` only bumps a poll counter —
it returns in milliseconds. The manual's own `START / WAIT 300 / STOP`
pattern stops acquisition almost immediately, producing a near-empty
spectrum. **Fix:** for non-simulator instruments, sleep wall-clock time in
increments while polling; keep fast-forward for the simulator.

### M: `wait_for_stop` busy-polls and swallows its own timeout
`job_host.rs` ~178-196: no sleep between iterations (hammers a remote
instrument with SHOW_STATUS/SHOW_DATA continuously), and after 10M
iterations it returns `Ok` while the instrument is still counting. **Fix:**
sleep between polls; on cap exhaustion return an error.

### M: Job `SET_PRESET_*` writes only the client-side mirror
`job_host.rs` ~360-397 use `presets_mut()`, which for `RemoteMcb` never
transmits; the instrument never has the preset, the stop is enforced purely
client-side (if the CLI dies mid-run the instrument counts forever), and
the "no preset change while counting" rule is bypassed. **Fix:** go through
`set_presets` so presets hit the wire.

### M: Chaos-test corpus disagrees with the parser over bare `BEEP`
`parse.rs` ~105 rejects bare `BEEP`; `tests/chaos.rs` ~35 writes it inside
its "valid" job, so the corpus job fails to parse at line 8 and the chaos
sweeps exercise less than intended (masked because the test only asserts
no-panic). **Fix:** check MAESTRO's manual for BEEP's real argument form;
align parser or corpus, and make the chaos test assert the pristine corpus
parses.

### L: `exec.rs` ~75: `position()` after the job ends returns the command
count posing as a line number; ~190: the hard 1,000,000-step cap kills a
legitimately large `LOOP` as "the job was stopped" with no hint why.

## Device layer (`crates/ortseam-device`)

### H: Malformed `SHOW_DATA` reply aborts the process
`remote.rs` ~139: `Vec::with_capacity(count)` with the channel count taken
straight from the reply line; `DATA 1000000000000000` kills the client with
an uncatchable allocation abort. One corrupt line over TCP/pipe takes down
the application. **Fix:** bound the count (largest real MCB is 16k
channels; refuse or clamp beyond, e.g. 65536) before allocating, and add
malformed-reply tests (`tests/remote.rs` currently has none).

### M: Unparseable `SHOW_DATA` channel words silently become 0 counts
`remote.rs` (~poll): a corrupt word in a `DATA` line parses as 0 counts and
the rest of the spectrum is accepted, so one garbled reply quietly zeroes
channels. **Fix:** treat an unparseable word as a malformed reply and reject
the whole line. (The other half of this finding — a length change discarding
the calibration and detector identity — was fixed 2026-08-05 by
`copy_descriptors_from` on resize.)

### M: Uncertainty and MDA presets never stop a remote instrument
The presets dialog accepts uncertainty and MDA presets for any detector, but
they are host-side calculations no instrument carries, and nothing evaluates
them for a bridged or network detector — only the simulator honours them.
**Why it matters:** an operator who sets "stop at 1% uncertainty" on the
bench 926 gets an acquisition that never stops, with no warning that the
preset is inert. **Fix:** evaluate these presets host-side in the poll loop
for remote instruments (the poll already carries the spectrum needed to
compute them), or grey them out with a "not supported for this instrument"
note until then.

### M: Bridge exchange has no timeout; Drop can hang
`bridge.rs` ~127-135/~151: `read_line` on the helper's stdout blocks forever
if the helper wedges (TCP transport sets timeouts; the pipe transport
doesn't), and `Drop` does `child.wait()` after closing stdin — correct
against zombies but unbounded. **Fix:** read with a timeout (thread +
channel, or make the helper's protocol line-timeout), and bound the Drop
wait before killing the child.

### L: Simulator: `store_spectrum` on a locked instrument pushes the
snapshot before `clear()` fails (`simulator.rs` ~726-738, partial state);
list-mode `CLEAR` rebuilds `ListModeFile` losing calibration/description
(~465-469 vs ~643-649); `SEND_MESSAGE("SET_PRESET_…")` skips the
busy-while-counting check `set_presets` enforces (~813-836).

### L: Served instrument's pole-zero never completes while idle
`ortseam-cli/src/main.rs` ~349-356: the serve clock thread only calls
`advance` when active, but Optimize/pole-zero run on the instrument clock
while idle (`simulator.rs` ~511-531), so `START_PZ` on a served simulator
never finishes until an acquisition starts. **Fix:** advance idle time too
(real time only).

### L: `server.rs` ~34 panics the serving thread on a poisoned mutex;
serve (~47-53) is single-client with no read timeout, so one silent client
blocks all future connections. Arguably by design; document or add a
timeout.

## Formats (`crates/ortseam-formats`)

### H: `.Spe` `$DATA:` channel count allocates unbounded
`spe.rs` ~42-53: `count = last.saturating_sub(first) + 1` feeds
`Vec::with_capacity` + `resize` with no cap — `0 900000000` in an otherwise
ordinary file forces a ~7 GB allocation and an uncatchable abort; with
`usize::MAX` the `+ 1` overflows (panic in debug, wraps to an empty
spectrum in release). Every other reader caps this (chn/spc at u16). This
is the most commonly exchanged format here. **Fix:** cap the declared count
(e.g. 1<<20 channels), error beyond; add valid-header-huge-length tests.

### M: List-mode channel count allocates unbounded
`list_mode.rs` ~222 (+ `to_spectrum` ~133/~172): the u32 channel-count field
at offset 12 is stored unchecked; ~4.29e9 channels → ~34 GB abort at
histogramming. Same fix: cap at read time.

### M: N42 CountedZeroes total is unbounded across runs
`n42.rs` ~262-268: each zero-run is capped at 1<<24 but runs accumulate —
~64 repeats (~1.3 KB of XML) force an 8 GB allocation. **Fix:** cap the
cumulative `counts.len()` too.

### M: `next_element` recursion can overflow the stack
`n42.rs` ~114-128: one recursive call per non-delimited tag-name prefix
occurrence (`<SpectrumX…` while scanning for "Spectrum"); Rust does not
guarantee tail calls in debug, so a sub-megabyte crafted N42 can overflow
Windows' 1 MB main-thread stack. **Fix:** loop instead of recursing.

### M: `.Lib` cross-nuclide peak-chain amplification
`library.rs` ~293-317: `seen_peaks` is per-nuclide, so a crafted ~5 MB .Lib
whose 65535 nuclide records all point at one shared 65535-long peak chain
does ~4.3e9 pushes — an effective hang. The per-walk cycle protection is
correct; the aggregate isn't bounded. **Fix:** cap total peaks read per
file (65535 — the format cannot address more).

### L: `spe.rs` ~56 truncates a >65535 first-channel to u16 silently;
`roi.rs` ~39-40 clamps regions above channel 32767 to i16::MAX on write
(inherent to the 16-bit .Roi layout — document at the call site);
`chn.rs` ~71 clamps negative i32 channel counts to 0 (harmless for
round-trips).

## Core (`crates/ortseam-core`) — deferred judgment calls

### M: `auto_calibrate` scoring still validates candidates linearly
The one-peak-per-line rule and linear-refit-at-3 are in (2026-08-04), but a
winner with 4+ matches is still refitted as a quadratic that the linear
scoring never validated; fine in practice, worth a second look if
auto-calibration ever misbehaves on curved detectors.

### L: `analyse` feeds whole-ROI background into Currie's B
`quant.rs` (~309): `peak.info.background` is background over the whole ROI,
where Currie's B is defined over the peak integration region; modestly
inflates per-line MDA for wide ROIs. Defensible as-is (ROIs are normally
drawn tight); revisit if MDAs read high against MAESTRO on the bench.

### L: Overflow/edge theoreticals
`analysis.rs` ~608 `6*get(i)` in u64 can overflow near u64::MAX/16 counts;
`roi.rs` ~32/~57 `len`/`center` overflow at `end = usize::MAX` (all in-crate
callers construct clamped ROIs). Guard if ROI construction is ever exposed
raw.

## Hardware verification (needs the bench, instrument idle)

- **Run the full-chain hardware test** after the current acquisition
  finishes: build i686 release `ortseam-mcb`, run
  `ORTSEAM_MCB=<abs path> cargo test -p ortseam-device --test
  bridge_hardware -- --nocapture`. It was skipped this session (adapters
  unplugged, then the instrument was live-counting and must not be
  disturbed).
- ~~**Prove the libusb path**~~ **Done on Linux, 2026-08-05** (PR #1):
  `direct.rs` drove the real 926 end to end — enumeration, commands, clocks,
  a whole spectrum, presets, and the desktop application via
  `ortseam-mcb serve`. Still open: **macOS**, which only type-checks, and
  **multiple adapters on one Linux bus** (the bench has one).
- **The exact-multiple/ZLP question** documented in `dpm.rs::read_in` now
  has bench evidence on *both* backends: Windows read 8192-channel spectra
  (32768 bytes ≡ 0 mod 4096) byte-identical with follow-up commands in sync,
  and the 2026-08-05 Linux run read 4096-channel spectra (16384 bytes)
  repeatedly through `transfer_blocking` with the live window polling in
  sync throughout. No stranded terminator has ever been observed. Keep the
  loop as it is; do not "fix" it on theory alone.
- **Optionally bind Windows to WinUSB** so the in-house path needs no ORTEC
  driver at all (today direct.rs is non-Windows because ORTEC's driver owns
  the device).
