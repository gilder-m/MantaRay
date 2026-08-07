#!/usr/bin/env bash
# Captures a demo state of the application into a PNG, for the documentation
# and for looking at changes before they are committed.
#
# The Windows counterpart is tools/screenshot.ps1, which captures a window by
# handle with PrintWindow. There is no equivalent under Wayland: a client cannot
# read another client's surface, by design. So this runs the application against
# a nested X server instead, which is more reliable than fighting the compositor
# and works the same whether the session is X11, Wayland or neither - including
# over SSH with no display at all.
#
#   tools/screenshot.sh analyse docs/screenshots/analyse.png
#   ORTSEAM_DEMO_ISOTOPE=eu152 tools/screenshot.sh isotope /tmp/isotope.png
#
# Needs Xvfb and ImageMagick. Build first: cargo build --release -p ortseam-gui
set -euo pipefail

demo="${1:-1}"
out="${2:-shot.png}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Where cargo actually puts things, which is not always $repo/target: a
# `target-dir` in any config.toml, or CARGO_TARGET_DIR, moves it. Ask cargo
# rather than assume, and fall back to the default if cargo cannot say.
target="${CARGO_TARGET_DIR:-}"
if [ -z "$target" ] && command -v cargo >/dev/null; then
    target="$(cargo metadata --format-version 1 --no-deps 2>/dev/null |
        sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
fi
exe="${ORTSEAM_EXE:-${target:-$here/../target}/release/ortseam-gui}"
files="${ORTSEAM_FILES:-$here/../crates/ortseam-formats/tests/fixtures/eu152_spectra.Spe}"
size="${ORTSEAM_SIZE:-1900x1060}"
# Long enough for the window to map, the fonts to load and the first frame to
# settle. Two seconds is plenty on any machine that can run the program at all.
settle="${ORTSEAM_SETTLE:-3}"

for tool in Xvfb import; do
    command -v "$tool" >/dev/null || {
        echo "$tool is not installed" >&2
        echo "  Debian/Ubuntu: sudo apt install xvfb imagemagick" >&2
        exit 1
    }
done
[ -x "$exe" ] || {
    echo "no build at $exe - cargo build --release -p ortseam-gui" >&2
    exit 1
}

# A display number nothing else is using, so two captures can run at once.
for number in $(seq 90 99); do
    [ -e "/tmp/.X${number}-lock" ] || { display=":$number"; break; }
done
[ -n "${display:-}" ] || { echo "no free display number" >&2; exit 1; }

Xvfb "$display" -screen 0 "${size}x24" -nolisten tcp &
server=$!
trap 'kill "$server" 2>/dev/null || true; wait "$server" 2>/dev/null || true' EXIT

# Wait for the server rather than guessing: xdpyinfo is not always installed,
# so the lock file is the portable signal.
for _ in $(seq 1 50); do
    [ -e "/tmp/.X${display#:}-lock" ] && break
    sleep 0.1
done

# winit prefers Wayland when a Wayland socket is in the environment, and would
# ignore the X server entirely.
env -u WAYLAND_DISPLAY \
    DISPLAY="$display" \
    ORTSEAM_DEMO="$demo" \
    WINIT_UNIX_BACKEND=x11 \
    "$exe" $files &
app=$!
trap 'kill "$app" 2>/dev/null || true; kill "$server" 2>/dev/null || true' EXIT

sleep "$settle"
kill -0 "$app" 2>/dev/null || {
    echo "the application exited before it could be captured" >&2
    exit 1
}

mkdir -p "$(dirname "$out")"
DISPLAY="$display" import -window root "$out"
kill "$app" 2>/dev/null || true
echo "$out"
