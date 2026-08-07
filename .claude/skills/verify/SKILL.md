---
name: verify
description: Verify a change to MantaRay end to end - format, lint, test, rustdoc, and look at the interface in a real screenshot. Use before committing any change, and always for a change that draws something.
---

# Verify a change

Five gates. A change is not done until all five pass, and a change that draws
anything is not done until somebody has **looked at it**.

## 1. Build settings

Artefacts go to `/home/windows/cargo-target` (ext4), never the NTFS mount the
working copy sits on — building there put 41 GB behind a userspace FUSE process
and the kernel OOM killer took the terminal with it. This is already set in
`~/.cargo/config.toml` along with `jobs = 4`. Do not override either.

A full workspace build still dips to a few hundred MB of headroom. Prefer
building one crate at a time when that is enough, and run long builds with
`run_in_background: true` so an OOM cannot take the session.

## 2. Format, lint, test, document

```sh
cargo fmt --all
cargo clippy --workspace --all-targets     # must be silent
cargo test --workspace                      # count the passes, do not eyeball
cargo doc --workspace --no-deps             # must be silent
```

Sum the test results rather than trusting the last line:

```sh
cargo test --workspace 2>&1 | awk '/test result:/ {p+=$4; f+=$6} END {print "passed:", p, "failed:", f}'
```

## 3. Look at it

```sh
cargo build --release -p mantaray-gui
tools/screenshot.sh <demo-state> /tmp/shot.png
```

Then **Read the PNG**. Crop to inspect a detail:

```sh
convert /tmp/shot.png -crop 900x300+0+150 +repage /tmp/crop.png
```

Demo states live in `settle_layout()` in `crates/mantaray-gui/src/app.rs`:

`tile` `rows` `help` `analyse` `dialogs` `charts` `hardware` `roi` `nolabels`
`empty` `insight` `select` `isotope` `cal` `job`

Prefix with a theme to check a palette: `paper:isotope`, `amber:roi`.
Some take extra variables, e.g. `MANTARAY_DEMO_ISOTOPE=eu152`.

**If the change has no demo state that shows it, add one.** A state that cannot
be captured cannot be checked, and the state is useful documentation afterwards.

This gate is not optional garnish. The first capture ever taken on this machine
found a label collision that thirty-seven passing frame tests could not see:
assertions on painted text are blind to overlap, contrast, crowding and colour.

## 4. Test through the path a person uses

A button wired to nothing looks identical to a working one in any test that
calls the action directly. `crates/mantaray-gui/tests/frames.rs` has
`click_menu_item(app, ctx, "Display", "Peak Labels")`, which finds the item by
the text actually painted and clicks where it is. Use it for anything reached
from a menu.

Watch for assertions that cannot fail. `assert!(off.len() < on.len())` passes
when a single label disappears out of sixty. Assert the specific thing:
name the nuclide that must be gone, count what must remain.

## 5. Before committing

- Commit messages: a plain descriptive sentence, then a body explaining *why*.
  Never mention AI or Claude, and never add a co-author trailer.
- The program is **MantaRay**; the crates and binaries are `mantaray-*`. Do not
  write the former name into anything new — history keeps it, prose does not.
- Branch, push, open a pull request, let CI go green. `gh pr edit` and
  `gh pr merge` fail on this repository (a GraphQL deprecation); use
  `gh api repos/gilder-m/MantaRay/pulls/N/merge` and `gh api -X PATCH` instead.
