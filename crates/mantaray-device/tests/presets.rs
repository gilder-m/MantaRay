//! Presets and the acquisition loop (MAESTRO Acquire/MCB Properties, Presets).

use mantaray_core::Roi;
use mantaray_device::{
    Mcb, MdaPreset, PresetKind, Presets, SimulatedMcb, SimulatedSource, SourceLine,
    UncertaintyPreset, advance,
};

fn detector(rate: f64) -> SimulatedMcb {
    SimulatedMcb::builder(1, "SIM")
        .channels(1024)
        .gain_kev_per_channel(2.0)
        .source(SimulatedSource {
            lines: vec![SourceLine::new(661.657, rate)],
            continuum_cps: rate / 4.0,
            ..SimulatedSource::default()
        })
        .seed(7)
        .build()
}

fn run(mcb: &mut SimulatedMcb, step: f64, limit: f64) -> Option<PresetKind> {
    let mut elapsed = 0.0;
    while elapsed < limit {
        if let Some(kind) = advance(mcb, step).unwrap() {
            return Some(kind);
        }
        elapsed += step;
    }
    None
}

#[test]
fn presets_start_empty_and_clear() {
    let mut presets = Presets::default();
    assert!(presets.is_empty());
    presets.real_time = Some(100.0);
    assert!(!presets.is_empty());
    presets.clear();
    assert!(presets.is_empty());
}

#[test]
fn a_real_time_preset_stops_the_detector() {
    let mut mcb = detector(500.0);
    mcb.presets_mut().real_time = Some(20.0);
    mcb.start().unwrap();
    let reason = run(&mut mcb, 1.0, 60.0);
    assert_eq!(reason, Some(PresetKind::RealTime));
    assert!(!mcb.is_active());
    assert!(mcb.status().real_time >= 20.0);
    assert!(mcb.status().real_time < 21.5);
}

#[test]
fn a_live_time_preset_stops_on_live_time() {
    let mut mcb = detector(20_000.0);
    mcb.presets_mut().live_time = Some(10.0);
    mcb.start().unwrap();
    let reason = run(&mut mcb, 0.5, 120.0);
    assert_eq!(reason, Some(PresetKind::LiveTime));
    assert!(mcb.status().live_time >= 10.0);
    assert!(
        mcb.status().real_time > mcb.status().live_time,
        "dead time must make the real time longer"
    );
}

#[test]
fn a_roi_peak_preset_needs_a_region_and_stops_at_the_channel_count() {
    let mut mcb = detector(2_000.0);
    mcb.presets_mut().roi_peak = Some(500);
    mcb.start().unwrap();
    // Without a region the preset can never be met.
    assert_eq!(run(&mut mcb, 1.0, 10.0), None);

    mcb.spectrum_mut().rois.mark(Roi::new(320, 340));
    let reason = run(&mut mcb, 1.0, 600.0);
    assert_eq!(reason, Some(PresetKind::RoiPeak));
    let peak = mcb
        .spectrum()
        .channels
        .iter()
        .skip(320)
        .take(21)
        .copied()
        .max()
        .unwrap();
    assert!(peak >= 500, "tallest marked channel is {peak}");
}

#[test]
fn a_roi_integral_preset_sums_every_marked_channel() {
    let mut mcb = detector(2_000.0);
    mcb.presets_mut().roi_integral = Some(20_000);
    mcb.spectrum_mut().rois.mark(Roi::new(300, 360));
    mcb.start().unwrap();
    let reason = run(&mut mcb, 1.0, 600.0);
    assert_eq!(reason, Some(PresetKind::RoiIntegral));
    assert!(mcb.spectrum().sum_range(300, 360) >= 20_000);
}

#[test]
fn an_uncertainty_preset_stops_when_the_statistics_are_good_enough() {
    let mut mcb = detector(2_000.0);
    mcb.presets_mut().uncertainty = Some(UncertaintyPreset {
        limit_percent: 1.0,
        low_channel: 300,
        high_channel: 360,
    });
    mcb.start().unwrap();
    let reason = run(&mut mcb, 1.0, 600.0);
    assert_eq!(reason, Some(PresetKind::Uncertainty));
    let gross = mcb.spectrum().sum_range(300, 360) as f64;
    assert!(100.0 / gross.sqrt() <= 1.0, "gross counts {gross}");
}

#[test]
fn an_mda_preset_stops_when_the_detection_limit_is_low_enough() {
    let mut mcb = detector(500.0);
    mcb.presets_mut().mda = Some(MdaPreset {
        target: 25.0,
        region: Roi::new(300, 360),
        efficiency: 0.05,
        yield_fraction: 0.851,
    });
    mcb.start().unwrap();
    let reason = run(&mut mcb, 2.0, 2000.0);
    assert_eq!(reason, Some(PresetKind::Mda));
    assert!(mcb.status().live_time > 0.0);
}

#[test]
fn the_first_preset_to_be_met_wins() {
    let mut mcb = detector(500.0);
    mcb.presets_mut().real_time = Some(1_000.0);
    mcb.presets_mut().live_time = Some(5.0);
    mcb.start().unwrap();
    assert_eq!(run(&mut mcb, 0.5, 100.0), Some(PresetKind::LiveTime));
}

#[test]
fn presets_can_only_be_changed_while_stopped() {
    let mut mcb = detector(500.0);
    mcb.start().unwrap();
    let presets = Presets {
        real_time: Some(10.0),
        ..Presets::default()
    };
    assert!(
        mcb.set_presets(presets).is_err(),
        "presets are locked while counting"
    );
    mcb.stop().unwrap();
    assert!(mcb.set_presets(Presets::default()).is_ok());
}

#[test]
fn preset_progress_reports_the_closest_preset() {
    let mut mcb = detector(500.0);
    mcb.presets_mut().live_time = Some(100.0);
    mcb.start().unwrap();
    assert_eq!(mcb.preset_progress(), Some(0.0));
    advance(&mut mcb, 50.0).unwrap();
    let progress = mcb.preset_progress().unwrap();
    assert!(progress > 0.4 && progress < 0.6, "progress {progress}");
}

#[test]
fn an_acquisition_without_presets_never_stops_by_itself() {
    let mut mcb = detector(500.0);
    mcb.start().unwrap();
    assert_eq!(run(&mut mcb, 10.0, 1000.0), None);
    assert!(mcb.is_active());
}
