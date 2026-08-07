//! The simulated multichannel buffer: what MAESTRO calls a Detector.

use mantaray_core::{CalculationSettings, peak_search};
use mantaray_device::{
    AcquisitionMode, DeviceError, Mcb, SimulatedMcb, SimulatedSource, SourceLine,
};

fn detector() -> SimulatedMcb {
    let source = SimulatedSource {
        lines: vec![
            SourceLine::new(661.657, 400.0),
            SourceLine::new(1173.228, 200.0),
            SourceLine::new(1332.492, 180.0),
        ],
        continuum_cps: 300.0,
        ..SimulatedSource::default()
    };
    SimulatedMcb::builder(1, "SIM-01")
        .channels(2048)
        .gain_kev_per_channel(1.0)
        .source(source)
        .seed(0xC0FFEE)
        .build()
}

#[test]
fn a_new_detector_is_idle_and_empty() {
    let mcb = detector();
    assert!(!mcb.is_active());
    assert_eq!(mcb.spectrum().total_counts(), 0);
    assert_eq!(mcb.spectrum().len(), 2048);
    assert_eq!(mcb.status().real_time, 0.0);
    assert_eq!(mcb.identity().number, 1);
    assert_eq!(mcb.identity().name, "SIM-01");
    assert!(mcb.identity().capabilities.list_mode);
}

#[test]
fn counts_only_accumulate_while_started() {
    let mut mcb = detector();
    mcb.poll(10.0).unwrap();
    assert_eq!(
        mcb.spectrum().total_counts(),
        0,
        "idle detectors collect nothing"
    );

    mcb.start().unwrap();
    mcb.poll(10.0).unwrap();
    let after_start = mcb.spectrum().total_counts();
    assert!(after_start > 0);
    assert!(mcb.status().real_time >= 9.99);

    mcb.stop().unwrap();
    mcb.poll(10.0).unwrap();
    assert_eq!(
        mcb.spectrum().total_counts(),
        after_start,
        "stopped is stopped"
    );
    assert!(mcb.status().real_time < 10.01, "the clock stops too");
}

#[test]
fn the_count_rate_matches_the_configured_source() {
    let mut mcb = detector();
    mcb.start().unwrap();
    mcb.poll(100.0).unwrap();
    let counts = mcb.spectrum().total_counts() as f64;
    // 1080 counts per second of live time.
    let expected = 1080.0 * mcb.status().live_time;
    assert!(
        (counts - expected).abs() < 6.0 * expected.sqrt(),
        "got {counts}, expected about {expected}"
    );
}

#[test]
fn peaks_land_at_the_calibrated_energies() {
    let mut mcb = detector();
    mcb.start().unwrap();
    mcb.poll(600.0).unwrap();
    let spectrum = mcb.spectrum();
    let cal = spectrum
        .energy_calibration
        .as_ref()
        .expect("the simulator publishes its calibration");

    let found = peak_search(spectrum, &CalculationSettings::default());
    let energies: Vec<f64> = found.iter().map(|p| cal.energy(p.centroid)).collect();
    for line in [661.657, 1173.228, 1332.492] {
        assert!(
            energies.iter().any(|e| (e - line).abs() < 3.0),
            "no peak near {line} keV in {energies:?}"
        );
    }
}

#[test]
fn resolution_follows_the_shape_calibration() {
    let mut mcb = detector();
    mcb.start().unwrap();
    mcb.poll(600.0).unwrap();
    let spectrum = mcb.spectrum();
    let shape = spectrum.shape_calibration.expect("a shape calibration");
    let found = peak_search(spectrum, &CalculationSettings::default());
    let cal = spectrum.energy_calibration.as_ref().unwrap();

    let peak = found
        .iter()
        .find(|p| (cal.energy(p.centroid) - 1332.492).abs() < 3.0)
        .expect("the 1332 keV peak");
    let expected = shape.fwhm(peak.centroid);
    assert!(
        (peak.width - expected).abs() < 0.35 * expected,
        "measured width {} against predicted {expected}",
        peak.width
    );
}

#[test]
fn dead_time_grows_with_the_input_rate() {
    let quiet = {
        let mut mcb = SimulatedMcb::builder(1, "quiet")
            .source(SimulatedSource {
                continuum_cps: 100.0,
                ..SimulatedSource::default()
            })
            .build();
        mcb.start().unwrap();
        mcb.poll(50.0).unwrap();
        mcb.status()
    };
    let busy = {
        let mut mcb = SimulatedMcb::builder(2, "busy")
            .source(SimulatedSource {
                continuum_cps: 100_000.0,
                ..SimulatedSource::default()
            })
            .build();
        mcb.start().unwrap();
        mcb.poll(50.0).unwrap();
        mcb.status()
    };
    assert!(quiet.dead_time_percent < 5.0, "{:?}", quiet);
    assert!(busy.dead_time_percent > 20.0, "{:?}", busy);
    assert!(busy.live_time < busy.real_time);
    assert!(busy.input_count_rate > quiet.input_count_rate);
    // Throughput saturates: the stored rate is below the input rate.
    assert!(busy.total_count_rate < busy.input_count_rate);
}

#[test]
fn clear_erases_data_and_times_but_not_the_calibration() {
    let mut mcb = detector();
    mcb.start().unwrap();
    mcb.poll(20.0).unwrap();
    mcb.clear().unwrap();
    assert_eq!(mcb.spectrum().total_counts(), 0);
    assert_eq!(mcb.status().real_time, 0.0);
    assert_eq!(mcb.status().live_time, 0.0);
    assert!(mcb.spectrum().energy_calibration.is_some());
    assert!(mcb.is_active(), "clearing does not stop the acquisition");
}

#[test]
fn a_snapshot_taken_before_clearing_survives_it() {
    // Undo works by keeping a copy: instrument memory is never written back to,
    // the copy is recovered into a buffer window instead.
    let mut mcb = detector();
    mcb.start().unwrap();
    mcb.poll(20.0).unwrap();
    let snapshot = mcb.spectrum().clone();
    assert!(snapshot.total_counts() > 0);

    mcb.clear().unwrap();
    assert_eq!(mcb.spectrum().total_counts(), 0);
    assert!(snapshot.total_counts() > 0, "the snapshot is independent");
    assert!(snapshot.live_time > 0.0);
    assert!(snapshot.energy_calibration.is_some());
}

#[test]
fn the_same_seed_gives_the_same_spectrum() {
    let run = || {
        let mut mcb = detector();
        mcb.start().unwrap();
        mcb.poll(30.0).unwrap();
        mcb.spectrum().channels.clone()
    };
    assert_eq!(run(), run());
}

#[test]
fn different_seeds_give_different_spectra() {
    let run = |seed: u64| {
        let mut mcb = SimulatedMcb::builder(1, "seeded").seed(seed).build();
        mcb.start().unwrap();
        mcb.poll(30.0).unwrap();
        mcb.spectrum().channels.clone()
    };
    assert_ne!(run(1), run(2));
}

#[test]
fn list_mode_records_events_and_histograms_the_same_data() {
    let mut mcb = detector();
    mcb.set_mode(AcquisitionMode::List).unwrap();
    mcb.start().unwrap();
    mcb.poll(5.0).unwrap();
    let list = mcb.list_data().expect("list mode data");
    assert!(list.len() > 1000, "recorded {} events", list.len());
    assert_eq!(list.to_spectrum().total_counts() as usize, list.len());
    assert!(list.events().windows(2).all(|w| w[0].time <= w[1].time));
    assert_eq!(mcb.spectrum().mode, AcquisitionMode::List);
}

#[test]
fn zero_dead_time_mode_publishes_a_corrected_and_an_error_spectrum() {
    let mut mcb = SimulatedMcb::builder(1, "zdt")
        .source(SimulatedSource {
            continuum_cps: 50_000.0,
            ..SimulatedSource::default()
        })
        .build();
    mcb.set_mode(AcquisitionMode::Zdt).unwrap();
    mcb.start().unwrap();
    mcb.poll(30.0).unwrap();

    let corrected = mcb.spectrum().total_counts();
    let uncorrected = mcb.uncorrected_spectrum().total_counts();
    assert!(
        corrected > uncorrected,
        "zero-dead-time counts {corrected} should exceed the raw {uncorrected}"
    );
    let error = mcb.error_spectrum().expect("an ERR spectrum in ZDT mode");
    assert!(error.total_counts() > 0);
    assert_eq!(mcb.spectrum().mode, AcquisitionMode::Zdt);
}

#[test]
fn locking_blocks_destructive_commands() {
    let mut mcb = detector();
    mcb.lock("secret", "operator").unwrap();
    assert!(mcb.is_locked());
    assert!(matches!(mcb.clear(), Err(DeviceError::Locked { .. })));
    assert!(matches!(mcb.start(), Err(DeviceError::Locked { .. })));
    assert!(matches!(
        mcb.unlock("wrong"),
        Err(DeviceError::Locked { .. })
    ));
    mcb.unlock("secret").unwrap();
    assert!(!mcb.is_locked());
    mcb.start().unwrap();
}

#[test]
fn hardware_messages_set_properties() {
    let mut mcb = detector();
    mcb.send_message("SET_GAIN_FINE 2048").unwrap();
    mcb.send_message("SET_GAIN_COARSE 100").unwrap();
    assert!((mcb.properties().amplifier.fine_gain - 0.5).abs() < 1e-6);
    assert!((mcb.properties().amplifier.coarse_gain - 100.0).abs() < 1e-6);
    assert!((mcb.properties().amplifier.gain() - 50.0).abs() < 1e-6);

    mcb.send_message("SET_PRESET_REAL 300").unwrap();
    assert_eq!(mcb.properties().presets.real_time, Some(300.0));

    assert!(matches!(
        mcb.send_message("FLY_TO_THE_MOON"),
        Err(DeviceError::Command { .. })
    ));
    assert!(!mcb.send_message("SHOW_STATUS").unwrap().is_empty());
}

#[test]
fn the_conversion_gain_resizes_the_spectrum() {
    let mut mcb = detector();
    let mut properties = mcb.properties().clone();
    properties.adc.conversion_gain = 512;
    mcb.set_properties(properties).unwrap();
    assert_eq!(mcb.spectrum().len(), 512);

    let mut properties = mcb.properties().clone();
    properties.adc.conversion_gain = 4096;
    assert!(
        matches!(
            mcb.set_properties(properties),
            Err(DeviceError::OutOfRange { .. })
        ),
        "the conversion gain cannot exceed the memory size"
    );
}

#[test]
fn discriminators_reject_events_outside_the_window() {
    let mut mcb = detector();
    let mut properties = mcb.properties().clone();
    properties.adc.lower_discriminator = 700;
    properties.adc.upper_discriminator = 1200;
    mcb.set_properties(properties).unwrap();
    mcb.start().unwrap();
    mcb.poll(60.0).unwrap();
    let spectrum = mcb.spectrum();
    assert_eq!(spectrum.sum_range(0, 699), 0, "below the LLD");
    assert_eq!(spectrum.sum_range(1201, 2047), 0, "above the ULD");
    assert!(spectrum.sum_range(700, 1200) > 0);
}
