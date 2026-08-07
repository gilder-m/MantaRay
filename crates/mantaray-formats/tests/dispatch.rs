//! Format detection, whole-file load/save and list-mode files.

use mantaray_core::{EnergyCalibration, Roi, Spectrum};
use mantaray_formats::{
    ListModeFile, ListRange, SpectrumFormat, list_mode, load_spectrum, save_spectrum,
};

fn temp_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mantaray-tests-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn sample() -> Spectrum {
    let mut s = Spectrum::from_counts((0..256u64).map(|c| c * 2 + 3).collect());
    s.live_time = 300.0;
    s.real_time = 310.0;
    s.sample_description = "dispatch test".into();
    s.energy_calibration = Some(EnergyCalibration::linear(0.5, 0.3));
    s
}

#[test]
fn formats_are_recognised_by_extension() {
    assert_eq!(
        SpectrumFormat::from_path("a.chn"),
        Some(SpectrumFormat::Chn)
    );
    assert_eq!(
        SpectrumFormat::from_path("A.CHN"),
        Some(SpectrumFormat::Chn)
    );
    assert_eq!(
        SpectrumFormat::from_path("a.Spe"),
        Some(SpectrumFormat::Spe)
    );
    assert_eq!(
        SpectrumFormat::from_path("a.spc"),
        Some(SpectrumFormat::Spc)
    );
    assert_eq!(
        SpectrumFormat::from_path("a.txt"),
        Some(SpectrumFormat::Ascii)
    );
    assert_eq!(
        SpectrumFormat::from_path("a.asc"),
        Some(SpectrumFormat::Ascii)
    );
    assert_eq!(
        SpectrumFormat::from_path("a.json"),
        Some(SpectrumFormat::Native)
    );
    assert_eq!(
        SpectrumFormat::from_path("a.lis"),
        Some(SpectrumFormat::List)
    );
    assert_eq!(SpectrumFormat::from_path("a.zzz"), None);
    assert_eq!(SpectrumFormat::from_path("noext"), None);
}

#[test]
fn formats_describe_themselves_for_file_dialogs() {
    assert_eq!(SpectrumFormat::Chn.extension(), "chn");
    assert!(SpectrumFormat::Chn.description().contains("Chn"));
    assert!(SpectrumFormat::all().len() >= 6);
    assert!(SpectrumFormat::Chn.is_binary());
    assert!(!SpectrumFormat::Spe.is_binary());
}

#[test]
fn save_and_load_round_trip_through_every_writable_format() {
    let dir = temp_dir();
    let original = sample();
    for format in SpectrumFormat::all() {
        if !format.can_write() {
            continue;
        }
        let path = dir.join(format!("round-trip.{}", format.extension()));
        save_spectrum(&original, &path).unwrap_or_else(|e| panic!("{format:?}: {e}"));
        let restored = load_spectrum(&path).unwrap_or_else(|e| panic!("{format:?}: {e}"));
        assert_eq!(
            restored.channels, original.channels,
            "{format:?} lost channel data"
        );
        assert!(
            (restored.live_time - original.live_time).abs() < 0.05,
            "{format:?} lost the live time"
        );
    }
}

#[test]
fn loading_an_unknown_extension_fails_clearly() {
    let dir = temp_dir();
    let path = dir.join("mystery.zzz");
    std::fs::write(&path, b"nothing").unwrap();
    let err = load_spectrum(&path).unwrap_err().to_string();
    assert!(
        err.contains("zzz") || err.to_lowercase().contains("unsupported"),
        "{err}"
    );
}

#[test]
fn list_mode_files_round_trip_and_slice_by_time() {
    // Ten seconds of events, one per 100 ms, alternating between two channels.
    let events: Vec<(f64, u32)> = (0..100)
        .map(|i| (i as f64 * 0.1, if i % 2 == 0 { 100 } else { 200 }))
        .collect();
    let mut file = ListModeFile::new(1024);
    for (time, channel) in &events {
        file.push(*time, *channel);
    }
    file.real_time = 10.0;
    file.live_time = 9.5;

    let bytes = list_mode::write(&file).unwrap();
    let back = list_mode::read(&bytes).unwrap();
    assert_eq!(back.len(), 100);
    assert_eq!(back.channel_count, 1024);
    assert!((back.real_time - 10.0).abs() < 1e-6);

    // The whole file as a histogram.
    let all = back.to_spectrum();
    assert_eq!(all.total_counts(), 100);
    assert_eq!(all.counts(100), 50);
    assert_eq!(all.counts(200), 50);
    assert!((all.real_time - 10.0).abs() < 1e-6);

    // A two-second slice starting at t = 1 s holds twenty events.
    let slice = back
        .slice(ListRange::from_seconds(1.0, 2.0))
        .expect("a valid slice");
    assert_eq!(slice.total_counts(), 20);
    assert!((slice.real_time - 2.0).abs() < 1e-6);

    // Slices outside the data are empty but valid.
    let empty = back.slice(ListRange::from_seconds(100.0, 1.0)).unwrap();
    assert_eq!(empty.total_counts(), 0);
}

#[test]
fn list_mode_slices_can_be_incremented() {
    let mut file = ListModeFile::new(256);
    for i in 0..50 {
        file.push(i as f64 * 0.2, 10);
    }
    file.real_time = 10.0;
    let range = ListRange::from_seconds(0.0, 1.0);
    let next = range.incremented(1.0);
    assert!((next.start - 0.0).abs() < 1e-9);
    assert!((next.duration - 2.0).abs() < 1e-9);
    assert_eq!(file.slice(range).unwrap().total_counts(), 5);
    assert_eq!(file.slice(next).unwrap().total_counts(), 10);
}

#[test]
fn list_mode_rejects_a_foreign_file() {
    assert!(list_mode::read(b"not a list file at all").is_err());
}

#[test]
fn saving_a_region_file_next_to_a_spectrum() {
    let dir = temp_dir();
    let mut s = sample();
    s.rois.mark(Roi::new(10, 30));
    let path = dir.join("with-regions.roi");
    mantaray_formats::save_rois(&s.rois, &path).unwrap();
    let back = mantaray_formats::load_rois(&path).unwrap();
    assert_eq!(back.to_pairs(), vec![(10, 30)]);
}
