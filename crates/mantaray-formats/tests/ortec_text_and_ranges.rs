//! What ORTEC's own programs read back from the files MantaRay writes: the
//! `.Spe` channel range, the text fields of `.Chn`/`.Spe`/`.Spc`, and the
//! `.Chn` date of a count that has none (release 0.2.5 deep review, S-1, N-2,
//! N-3).

use mantaray_core::Spectrum;
use mantaray_formats::{chn, load_spectrum, save_spectrum, spe};

/// The description every format is checked with: a micro sign and a degree
/// sign, both in Windows-1252 and both common on a lab bench.
const LAB_TEXT: &str = "137Cs 1 \u{b5}Ci, 22 \u{b0}C";

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mantaray-ortec-text-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir.join(name)
}

fn data_range(text: &str) -> String {
    text.lines()
        .skip_while(|line| *line != "$DATA:")
        .nth(1)
        .expect("a range line")
        .to_string()
}

// --- S-1: `$DATA:` is `first last`, not `first count-1` ---------------------

#[test]
fn a_spe_with_a_channel_offset_keeps_every_channel() {
    let mut s = Spectrum::from_counts((0..1024u64).collect());
    s.channel_offset = 100;
    let text = spe::write(&s).expect("write");
    assert_eq!(data_range(&text), "100 1123");
    let back = spe::read(&text).expect("read");
    assert_eq!(back.len(), 1024);
    assert_eq!(back.channel_offset, 100);
    assert_eq!(back.channels, s.channels);
}

#[test]
fn a_chn_at_an_offset_saved_as_spe_opens_again() {
    let mut s = Spectrum::from_counts(vec![5; 50]);
    s.channel_offset = 60;
    let chn_back = chn::read(&chn::write(&s).expect("chn")).expect("chn back");
    assert_eq!(chn_back.channel_offset, 60);
    let text = spe::write(&chn_back).expect("spe");
    assert_eq!(data_range(&text), "60 109");
    let back = spe::read(&text).expect("the .Spe of a 50-channel spectrum at offset 60 opens");
    assert_eq!(back.channels, vec![5; 50]);
    assert_eq!(back.channel_offset, 60);
}

#[test]
fn a_spe_range_ending_before_it_starts_is_refused_saying_so() {
    let text = "$SPEC_ID:\nx\n$DATA:\n60 49\n1\n2\n";
    let error = spe::read(text)
        .expect_err("an impossible range")
        .to_string();
    assert!(
        error.contains("60") && error.contains("49") && error.contains("before"),
        "{error}"
    );
}

#[test]
fn a_spe_range_too_long_for_any_mcb_is_refused_saying_so() {
    let text = "$DATA:\n0 900000000\n1\n";
    let error = spe::read(text).expect_err("a huge range").to_string();
    assert!(error.contains("channels"), "{error}");
}

/// Files the 0.2.4 writer put out with an offset: the range says
/// `offset count-1` and the counts that follow are all there. Those files are
/// read as what they hold, rather than lose the end or refuse to open.
#[test]
fn a_spe_written_by_0_2_4_at_an_offset_still_reads_every_channel() {
    let legacy = |first: usize, len: usize| {
        let mut text = format!(
            "$SPEC_ID:\nx\n$SPEC_REM:\nDET# 1\nDETDESC# d\nAP# mantaray 0.2.4\n$MEAS_TIM:\n1 1\n\
             $DATA:\n{first} {}\n",
            len - 1
        );
        for c in 0..len {
            text.push_str(&format!("{c}\n"));
        }
        text
    };
    let back = spe::read(&legacy(100, 1024)).expect("offset below the count");
    assert_eq!(back.len(), 1024);
    assert_eq!(back.channel_offset, 100);
    assert_eq!(back.channels.last(), Some(&1023));
    let back = spe::read(&legacy(60, 50)).expect("offset above the count");
    assert_eq!(back.len(), 50);
    assert_eq!(back.channel_offset, 60);
}

/// A file from anyone else is read by the spec: counts past `last` are not
/// channels of this spectrum.
#[test]
fn a_foreign_spe_is_read_by_its_range_only() {
    let mut text = String::from("$SPEC_REM:\nAP# GammaVision\n$DATA:\n2 5\n");
    for c in 0..10 {
        text.push_str(&format!("{c}\n"));
    }
    let back = spe::read(&text).expect("read");
    assert_eq!(back.channel_offset, 2);
    assert_eq!(back.channels, vec![0, 1, 2, 3]);
}

// --- N-3: text fields are Windows-1252, as MAESTRO reads them ---------------

fn through(name: &str) -> (Vec<u8>, Spectrum) {
    let mut s = Spectrum::from_counts(vec![1; 64]);
    s.sample_description = LAB_TEXT.into();
    s.detector_description = format!("GEM {LAB_TEXT}");
    let path = scratch(name);
    save_spectrum(&s, &path).expect("save");
    let bytes = std::fs::read(&path).expect("bytes");
    let back = load_spectrum(&path).expect("load");
    std::fs::remove_file(&path).expect("clean");
    (bytes, back)
}

fn windows_1252(text: &str) -> Vec<u8> {
    // Every character of LAB_TEXT is ASCII or Latin-1, where Windows-1252 is
    // the code point itself.
    text.chars()
        .map(|c| u8::try_from(u32::from(c)).expect("Latin-1"))
        .collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn a_chn_description_is_stored_as_windows_1252_and_reads_back() {
    let (bytes, back) = through("lab.Chn");
    assert!(
        contains(&bytes, &windows_1252(LAB_TEXT)),
        "not Windows-1252"
    );
    assert!(!contains(&bytes, "\u{b5}".as_bytes()), "UTF-8 in a .Chn");
    assert_eq!(back.sample_description, LAB_TEXT);
    assert_eq!(back.detector_description, format!("GEM {LAB_TEXT}"));
}

#[test]
fn a_spe_is_stored_as_windows_1252_and_reads_back() {
    let (bytes, back) = through("lab.Spe");
    assert!(
        contains(&bytes, &windows_1252(LAB_TEXT)),
        "not Windows-1252"
    );
    assert!(!contains(&bytes, "\u{b5}".as_bytes()), "UTF-8 in a .Spe");
    assert_eq!(back.sample_description, LAB_TEXT);
    assert_eq!(back.detector_description, format!("GEM {LAB_TEXT}"));
}

#[test]
fn a_spc_description_is_stored_as_windows_1252_and_reads_back() {
    let (bytes, back) = through("lab.Spc");
    assert!(
        contains(&bytes, &windows_1252(LAB_TEXT)),
        "not Windows-1252"
    );
    assert_eq!(back.sample_description, LAB_TEXT);
    assert_eq!(back.detector_description, format!("GEM {LAB_TEXT}"));
}

#[test]
fn the_native_file_stays_utf_8() {
    let (bytes, back) = through("lab.json");
    assert!(contains(&bytes, LAB_TEXT.as_bytes()), "native is UTF-8");
    assert_eq!(back.sample_description, LAB_TEXT);
}

#[test]
fn a_chn_description_is_never_cut_through_a_character() {
    let mut s = Spectrum::from_counts(vec![1; 4]);
    s.sample_description = format!("{}\u{b5}Ci", "x".repeat(62));
    let back = chn::read(&chn::write(&s).expect("write")).expect("read");
    assert_eq!(back.sample_description, format!("{}\u{b5}", "x".repeat(62)));
}

#[test]
fn a_character_windows_1252_lacks_is_saved_as_a_question_mark() {
    let mut s = Spectrum::from_counts(vec![1; 4]);
    s.sample_description = "\u{3b2} source, 1 \u{b5}Ci".into();
    for name in ["beta.Chn", "beta.Spe", "beta.Spc"] {
        let path = scratch(name);
        save_spectrum(&s, &path).expect("save");
        let back = load_spectrum(&path).expect("load");
        std::fs::remove_file(&path).expect("clean");
        assert_eq!(back.sample_description, "? source, 1 \u{b5}Ci", "{name}");
    }
}

/// What MAESTRO writes: CRLF and single-byte text. `0xB5 0xB0` is not UTF-8,
/// and used to come back as two U+FFFD.
#[test]
fn a_maestro_spe_with_windows_1252_text_reads_and_sniffs() {
    let mut bytes = b"$SPEC_ID:\r\n137Cs 1 ".to_vec();
    bytes.extend_from_slice(&[0xb5]);
    bytes.extend_from_slice(b"Ci, 22 ");
    bytes.extend_from_slice(&[0xb0]);
    bytes.extend_from_slice(b"C\r\n$SPEC_REM:\r\nDET# 2\r\nAP# Maestro-PRO\r\n$MEAS_TIM:\r\n1 1\r\n$DATA:\r\n0 3\r\n1\r\n2\r\n3\r\n4\r\n");
    for name in ["maestro.Spe", "maestro.001"] {
        let path = scratch(name);
        std::fs::write(&path, &bytes).expect("fixture");
        let back = load_spectrum(&path).expect("load");
        std::fs::remove_file(&path).expect("clean");
        assert_eq!(back.sample_description, LAB_TEXT, "{name}");
        assert_eq!(back.channels, vec![1, 2, 3, 4], "{name}");
    }
}

/// 0.2.4 and earlier stored UTF-8; those files keep reading as they were meant.
#[test]
fn a_chn_with_a_utf_8_description_from_0_2_4_still_reads() {
    let mut bytes = chn::write(&Spectrum::from_counts(vec![1; 4])).expect("write");
    let at = 32 + 16 + 320;
    let text = LAB_TEXT.as_bytes();
    bytes[at] = u8::try_from(text.len()).expect("short");
    bytes[at + 1..at + 1 + text.len()].copy_from_slice(text);
    assert_eq!(
        chn::read(&bytes).expect("read").sample_description,
        LAB_TEXT
    );
}

// --- N-2: no start date is binary zeros ------------------------------------

#[test]
fn a_chn_with_no_start_time_stores_binary_zeros() {
    let bytes = chn::write(&Spectrum::from_counts(vec![1; 4])).expect("write");
    assert_eq!(&bytes[6..8], &[0u8; 2], "seconds");
    assert_eq!(&bytes[16..28], &[0u8; 12], "date and time");
    assert_eq!(chn::read(&bytes).expect("read").start_time, None);
}

#[test]
fn a_chn_with_a_blank_date_from_0_2_4_still_reads_as_no_date() {
    let mut bytes = chn::write(&Spectrum::from_counts(vec![1; 4])).expect("write");
    bytes[6..8].copy_from_slice(b"  ");
    bytes[16..28].copy_from_slice(b"            ");
    assert_eq!(chn::read(&bytes).expect("read").start_time, None);
}
