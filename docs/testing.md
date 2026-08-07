# What each suite tests

`cargo test --workspace` runs all of it. Every suite here exists because
something went wrong once; the header says what.

## mantaray-core — the arithmetic

Everything a number in a report is computed from. These are pure functions, so
the tests are the specification.

| Suite | What it holds to account |
|---|---|
| `spectrum.rs` | channels, live and real time, dead time, totals |
| `roi.rs` | region arithmetic, overlap, merging, clearing across a span |
| `peak_search.rs` | finding peaks in a real continuum without inventing them |
| `peak_info.rs` | centroid, FWHM, net and gross area, the background under a peak |
| `calibration.rs` | energy and FWHM fits, and refusing an ill-conditioned one |
| `auto_calibrate.rs` | recovering a calibration from a known nuclide's lines |
| `efficiency.rs` | the efficiency curve, fitted in log-log |
| `library.rs` | nuclide lookup, the name parser, the element table |
| `quant.rs` | identification and activity, including decay correction |
| `transforms.rs` | smoothing, stripping, summing — and that they are reversible |
| `undo.rs` | the history stack, and that undoing restores what was there |

## mantaray-formats — reading and writing files

The formats are somebody else's, so these are mostly tests about being
suspicious of them.

| Suite | What it holds to account |
|---|---|
| `chn.rs`, `spe.rs` | the two common ORTEC formats, both directions |
| `spc_cross_check.rs` | `.Spc` read back against the same data in another format |
| `sniff.rs`, `dispatch.rs` | choosing a reader by content rather than by extension |
| `roi_ascii_native.rs` | region files, ASCII spectra, and the native JSON |
| `library_files.rs` | binary `.Lib` files, chain-walked rather than trusted in order |
| `ortec_examples.rs`, `real_data.rs`, `fixtures.rs` | real files off a real instrument, not synthesised ones |
| `chaos.rs` | truncated, padded and corrupted files, which must be refused rather than half-read |

The NNDC converter's own tests live beside it in `src/nndc.rs`, because they
turn on the shape of a particular export.

## mantaray-device — instruments

| Suite | What it holds to account |
|---|---|
| `simulator.rs` | a simulated MCB, so the rest can be driven without hardware |
| `remote.rs` | the client half of the bridge, including refusing a serial it did not ask for |
| `bridge.rs` | the protocol between the program and the helper |
| `presets.rs` | live, real, area and count limits, and stopping on them |
| `workflow.rs` | start, stop, clear and copy in the orders an operator uses them |
| `bench_926.rs` | **a real ORTEC 926.** Ignored by default; run it where the instrument is |
| `bridge_hardware.rs` | the bridge against real hardware. Also ignored by default |

The two hardware suites need the instrument on the bus:

```sh
cargo test -p mantaray-device --test bench_926 -- --ignored --nocapture
```

## mantaray-gui — the interface, drawn headless

egui renders without a window, so these draw the real interface and fail if any
of it panics, draws nothing, or draws the wrong thing.

| Suite | What it holds to account |
|---|---|
| `frames.rs` | whole frames: menus, toolbar, sidebar, dialogs, every theme, every scale. Also the tab strip, the workspaces, and that no dialog can grow off the screen |
| `pointer.rs` | clicking, dragging and the wheel, as synthetic pointer events |
| `session.rs` | the crash snapshot, what survives a restart, and settings written by earlier versions |
| `recall.rs` | opening files of every supported kind, end to end |
| `workflows.rs` | whole jobs in the order a person does them |
| `calibration_flow.rs` | calibrating from two known lines and saving it |
| `timing.rs` | the clock while a count runs |
| `print_sample.rs`, `svg_sample.rs` | write a page and a plot out, for eyeballing |

Some of these drive the interface the way a hand does — finding a menu item by
the text painted on screen and clicking where it is — because a button wired to
nothing looks identical to a working one in a test that calls the action
directly.

## The rest

- **`mantaray-jobs`** — `parse.rs` and `execute.rs` for the JOB language, and
  `chaos.rs` for malformed scripts.
- **`mantaray-report`** — `reports.rs` for the written output.
- **`mantaray-cli`** — `cli.rs` runs the built binary and checks what it prints
  and what it exits with.

## What the tests cannot cover

Said plainly, because a gap nobody names is a gap somebody assumes is covered.

- **Native file dialogs.** `rfd` opens the operating system's picker, which
  cannot be driven headlessly. Importing and exporting a colour scheme is
  tested as far as the JSON and the error paths; the button-to-dialog wiring is
  not.
- **Anything on screen that is not text or geometry.** Overlap, contrast and
  crowding do not show up in an assertion about painted text. `tools/screenshot.sh`
  renders a demo state to a PNG for that, and the states live in `settle_layout`
  in `crates/mantaray-gui/src/app.rs`.
- **macOS.** CI builds and tests it. Nobody has run it against an instrument.
