//! Render diagnostics, for the machine where the interface is slow.
//!
//! Task Manager can say that a GPU is busy; it cannot say which adapter this
//! program is drawing with, over which backend, or whether a slow frame was
//! slow because it was expensive to build or because a cheaply built frame
//! stood waiting to be shown. Those are different faults with different
//! owners - this program, wgpu's backend choice, the driver - and a lag
//! report is actionable exactly when it says which one it is looking at.
//!
//! Shown when `MANTARAY_DEBUG` is set in the environment:
//!
//! ```text
//! set MANTARAY_DEBUG=1 && mantaray-gui     (Windows)
//! MANTARAY_DEBUG=1 mantaray-gui            (elsewhere)
//! ```
//!
//! The same switch keeps the written record: the mirror journals every fetch
//! and command to `mantaray-debug.log`, and the bridge writes its own half to
//! `mantaray-mcb-debug.log`, both in the directory the program was started
//! from - see [`mantaray_device::journal`]. The overlay answers "what is the
//! frame doing"; the journals answer "what did the instrument actually say",
//! which is the question a misbehaving bench gets asked.
//!
//! How to read it: drag the marker back and forth and watch the frame count.
//! Continuous input asks for a frame per event, so during a drag the count
//! should sit near the display's refresh rate. If it is far below that while
//! the CPU figures stay small, the frames were cheap and late - the wait is
//! after this program hands the frame over, in the backend, the driver or the
//! compositor. If the CPU figures themselves are large, the cost is here.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How far back the rolling figures look.
const WINDOW: Duration = Duration::from_secs(1);

/// The rolling frame record behind the overlay.
///
/// Held by the application only when `MANTARAY_DEBUG` is set; when it is
/// absent nothing is recorded and nothing is drawn.
pub struct Diagnostics {
    /// The adapter, backend and driver eframe is rendering with, asked once
    /// at start-up - the facts Task Manager cannot show.
    adapter: String,
    /// The recent frames, oldest first: when each was recorded and what the
    /// frame before it cost to build, in milliseconds of CPU. Trimmed to
    /// [`WINDOW`].
    frames: VecDeque<(Instant, f32)>,
}

impl Diagnostics {
    /// Built from eframe's creation context, or `None` when `MANTARAY_DEBUG`
    /// is not in the environment.
    pub fn from_environment(cc: &eframe::CreationContext<'_>) -> Option<Self> {
        std::env::var_os("MANTARAY_DEBUG")?;
        let adapter = match cc.wgpu_render_state.as_ref() {
            Some(state) => {
                let info = state.adapter.get_info();
                // Only the pieces the backend fills in: Metal reports no
                // driver strings at all, and DX12 is the one this exists for.
                let mut line = format!("{:?}", info.backend);
                for piece in [&info.driver, &info.driver_info] {
                    if !piece.is_empty() {
                        line.push_str(" · ");
                        line.push_str(piece);
                    }
                }
                format!("{}\n{line}", info.name)
            }
            // eframe built without its wgpu renderer, which would itself be
            // worth knowing on a machine being diagnosed.
            None => "no wgpu render state".to_string(),
        };
        // Into the journal too, so a log file sent from a bench says what
        // was drawing it without needing the photograph as well.
        mantaray_device::journal::line(&format!("renderer: {}", adapter.replace('\n', " · ")));
        Some(Self {
            adapter,
            frames: VecDeque::new(),
        })
    }

    /// Records that a frame is being finished right now.
    ///
    /// `cpu` is eframe's measure of the time the previous frame took to
    /// build - `None` only before there was a previous frame.
    pub fn frame(&mut self, cpu: Option<f32>) {
        let now = Instant::now();
        self.frames.push_back((now, cpu.unwrap_or(0.0) * 1000.0));
        while self
            .frames
            .front()
            .is_some_and(|(then, _)| now.duration_since(*then) > WINDOW)
        {
            self.frames.pop_front();
        }
    }

    /// Draws the overlay.
    ///
    /// Numbers only, no verdict: the overlay cannot tell an idle second from
    /// a stalling one - only the person dragging the marker knows whether
    /// frames were being asked for - so the reading stays with the reader.
    pub fn window(&self, ctx: &egui::Context) {
        let count = self.frames.len();
        let gap = match (
            self.frames
                .len()
                .checked_sub(2)
                .and_then(|i| self.frames.get(i)),
            self.frames.back(),
        ) {
            (Some((earlier, _)), Some((later, _))) => {
                format!(
                    "{:.0} ms",
                    later.duration_since(*earlier).as_secs_f32() * 1000.0
                )
            }
            _ => "—".to_string(),
        };
        let last = self.frames.back().map(|(_, ms)| *ms).unwrap_or(0.0);
        let worst = self
            .frames
            .iter()
            .map(|(_, ms)| *ms)
            .fold(0.0_f32, f32::max);
        let points = ctx.content_rect().size();
        let scale = ctx.pixels_per_point();
        egui::Window::new("Render diagnostics")
            .default_pos([16.0, 64.0])
            .resizable(false)
            .show(ctx, |ui| {
                ui.monospace(&self.adapter);
                ui.separator();
                ui.monospace(format!("frames   {count} in the last second"));
                ui.monospace(format!("gap      {gap} before this frame"));
                ui.monospace(format!("cpu      {last:.1} ms · worst {worst:.1} ms"));
                ui.monospace(format!(
                    "screen   {:.0}×{:.0} px · {scale:.2} px/pt",
                    points.x * scale,
                    points.y * scale
                ));
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The record is rolling: a frame older than the window is gone, and the
    /// figures describe only the last second's worth.
    #[test]
    fn the_record_keeps_only_the_last_second() {
        let mut diagnostics = Diagnostics {
            adapter: "test".into(),
            frames: VecDeque::new(),
        };
        diagnostics.frame(Some(0.004));
        diagnostics.frame(Some(0.002));
        assert_eq!(diagnostics.frames.len(), 2);
        // The first frame's cost belongs to the frame before it, so the
        // figures are milliseconds of what was actually handed in.
        assert!((diagnostics.frames[0].1 - 4.0).abs() < 1e-6);

        std::thread::sleep(Duration::from_millis(1100));
        diagnostics.frame(None);
        assert_eq!(
            diagnostics.frames.len(),
            1,
            "frames from before the window must have been trimmed"
        );
    }
}
