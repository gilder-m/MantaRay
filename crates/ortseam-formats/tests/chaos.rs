//! Garbage in, errors out - never panics.
//!
//! Every reader is fed hostile bytes: random noise, truncations of valid
//! files, valid headers with corrupt bodies. The contract is modest and
//! absolute: a reader may refuse anything, but it may not crash, hang or
//! allocate the moon.

use ortseam_formats::{SpectrumFormat, read_as, sniff};

/// A deterministic pseudo-random byte stream (xorshift), so a failure here
/// reproduces exactly.
fn noise(seed: u64, length: usize) -> Vec<u8> {
    let mut state = seed | 1;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect()
}

#[test]
fn every_reader_survives_random_noise() {
    for seed in 1..=50u64 {
        for length in [0usize, 1, 7, 31, 32, 33, 128, 512, 4096] {
            let bytes = noise(seed * 7919, length);
            let _ = sniff(&bytes);
            for format in SpectrumFormat::all() {
                // Refusal is fine; panicking is the failure.
                let _ = read_as(*format, &bytes);
            }
        }
    }
}

#[test]
fn every_reader_survives_truncations_of_a_valid_file() {
    let mut spectrum = ortseam_core::Spectrum::new(256);
    spectrum.live_time = 10.0;
    spectrum.real_time = 11.0;
    for channel in 0..256 {
        spectrum.channels[channel] = channel as u64;
    }
    let whole: Vec<(SpectrumFormat, Vec<u8>)> = vec![
        (
            SpectrumFormat::Chn,
            ortseam_formats::chn::write(&spectrum).expect("chn"),
        ),
        (
            SpectrumFormat::Spe,
            ortseam_formats::spe::write(&spectrum)
                .expect("spe")
                .into_bytes(),
        ),
        (
            SpectrumFormat::Native,
            ortseam_formats::native::write(&spectrum)
                .expect("native")
                .into_bytes(),
        ),
    ];
    for (format, bytes) in whole {
        for keep in 0..bytes.len().min(200) {
            let _ = read_as(format, &bytes[..keep]);
        }
        // And a few mid-length cuts of longer files.
        for fraction in 1..20 {
            let keep = bytes.len() * fraction / 20;
            let _ = read_as(format, &bytes[..keep]);
        }
    }
}

#[test]
fn corrupt_bodies_behind_valid_headers_are_survived() {
    let mut spectrum = ortseam_core::Spectrum::new(64);
    spectrum.channels[10] = 1000;
    let clean = ortseam_formats::chn::write(&spectrum).expect("chn");
    for seed in 1..=30u64 {
        let mut bytes = clean.clone();
        let garbage = noise(seed, bytes.len());
        // Keep the 32-byte header, corrupt everything after it.
        for (byte, replacement) in bytes.iter_mut().skip(32).zip(garbage) {
            *byte = replacement;
        }
        let _ = read_as(SpectrumFormat::Chn, &bytes);
    }
}
