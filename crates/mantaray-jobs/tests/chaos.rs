//! Hostile job files: the parser refuses, it does not crash.

use mantaray_jobs::Job;

/// Deterministic pseudo-random bytes, printable-biased so lines form.
fn noise(seed: u64, length: usize) -> String {
    let mut state = seed | 1;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let byte = (state >> 24) as u8;
            match byte % 8 {
                0 => '\n',
                1 => '"',
                2 => ' ',
                _ => (32 + (byte % 95)) as char,
            }
        })
        .collect()
}

#[test]
fn random_text_never_panics_the_parser() {
    for seed in 1..=100u64 {
        let text = noise(seed * 6151, 400);
        // Refusing with a line number is fine; crashing is the failure.
        let _ = Job::parse(&text);
    }
}

#[test]
fn hostile_edits_of_a_valid_job_never_panic() {
    // BEEP always takes an argument - the manual gives `BEEP <freq>,<duration>`,
    // `BEEP ID` and `BEEP "String"`, no bare form. The corpus once wrote a
    // bare BEEP, so it failed to parse at line 8 and every edit below
    // exercised less than intended.
    let valid = "SET_DETECTOR 1\nSET_PRESET_LIVE 300\nCLEAR\nSTART\nWAIT\nSAVE \"run???.chn\"\nLOOP 3\nBEEP 1\nEND_LOOP\n";
    assert!(
        Job::parse(valid).is_ok(),
        "the pristine corpus must parse, or the sweeps below exercise nothing"
    );
    // Truncations at every byte.
    for keep in 0..valid.len() {
        let _ = Job::parse(&valid[..keep]);
    }
    // Single-character corruption at every position.
    for position in 0..valid.len() {
        let mut text: Vec<char> = valid.chars().collect();
        text[position] = '\u{7}';
        let corrupted: String = text.into_iter().collect();
        let _ = Job::parse(&corrupted);
    }
}

#[test]
fn pathological_shapes_are_refused_quickly() {
    // A very long single line, deep quote nesting, and a wall of loops. Each
    // must parse (or refuse) promptly, not hang or blow the stack.
    let long_line = format!("REM {}", "x".repeat(200_000));
    let _ = Job::parse(&long_line);
    let quotes = "SAVE ".to_string() + &"\"".repeat(10_000);
    let _ = Job::parse(&quotes);
    let loops = "LOOP 2\n".repeat(5_000);
    let _ = Job::parse(&loops);
}
