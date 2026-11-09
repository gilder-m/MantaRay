//! A description that runs over several lines must not become counts.
//!
//! The GUI edits the sample description in a multi-line box, and the `.txt`
//! reader takes any line whose first word is a number as channel data. A
//! second line "500 g, dried" written raw came back as a channel of 500 in
//! front of channel 0, shifting every count after it (test gaps FMT-2).

use mantaray_core::Spectrum;
use mantaray_formats::ascii;

fn round_trip(spectrum: &Spectrum) -> Spectrum {
    ascii::read(&ascii::write(spectrum, &Default::default())).expect("the dump reads back")
}

#[test]
fn a_second_description_line_that_starts_with_a_number_is_not_a_channel() {
    let mut spectrum = Spectrum::from_counts(vec![1, 2, 3, 4]);
    spectrum.sample_description = "Soil core 4\n500 g, dried".into();
    assert_eq!(round_trip(&spectrum).channels, vec![1, 2, 3, 4]);
}

#[test]
fn windows_line_ends_and_the_detector_description_are_kept_off_the_counts_too() {
    let mut spectrum = Spectrum::from_counts(vec![7, 8, 9]);
    spectrum.sample_description = "Filter A\r\n12.5 cm from the face\r3 h".into();
    spectrum.detector_description = "HPGe\n40 % rel".into();
    assert_eq!(round_trip(&spectrum).channels, vec![7, 8, 9]);
}

#[test]
fn the_description_is_written_on_one_line_with_every_part_kept() {
    let mut spectrum = Spectrum::from_counts(vec![1]);
    spectrum.sample_description = "Soil core 4\n500 g, dried".into();
    let text = ascii::write(&spectrum, &Default::default());
    let sample: Vec<&str> = text
        .lines()
        .filter(|line| line.starts_with("Sample"))
        .collect();
    assert_eq!(sample.len(), 1, "{text}");
    assert!(sample[0].contains("Soil core 4"), "{text}");
    assert!(sample[0].contains("500 g, dried"), "{text}");
}
