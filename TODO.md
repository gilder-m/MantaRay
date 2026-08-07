# TODO

The open findings, seeded by the 2026-08-04 full-codebase audit (five
independent review passes, one per subsystem) and updated as work lands -
pruned 2026-08-06 after the audit-fix pass closed most of the list. Each
entry says where, what goes wrong, why it matters, and what the fix should
look like. What has been fixed is in the changelog, not here.

Severity: **H** = wrong results/crash/hang on realistic use, **M** = on
unusual-but-possible input, **L** = practice/robustness.

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
drawn tight); revisit if MDAs read high against MAESTRO on the bench.

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
  polls; the GUI parks the job and checks once per frame) and sends job
  presets through `set_presets` so they reach the wire. Proven against a
  scripted transport; worth one `START / WAIT 10 / STOP / SAVE` job against
  the real 926.
