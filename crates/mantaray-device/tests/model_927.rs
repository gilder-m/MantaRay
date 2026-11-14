//! The application's side of a Model 927 input, without the box.
//!
//! The helper's half is tested in `mantaray-mcb` against its own fake 927;
//! this is the half above the pipe: what [`RemoteMcb`] makes of the answers
//! `serve` gives for a 927 input. The answers are the ones measured on the
//! bench (input 2 of 9049204), and the refusals are the helper's own words
//! for the verbs a 927 has no register for.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use mantaray_device::{DeviceError, Mcb, Presets, RemoteMcb, Transport};

/// What `serve` answers for one 927 input, holding the state a count needs.
struct Input927 {
    serial: &'static str,
    counting: bool,
    presets: [u64; 4],
    log: Arc<Mutex<Vec<String>>>,
}

impl Transport for Input927 {
    fn exchange(&mut self, command: &str) -> Result<String, DeviceError> {
        self.log.lock().expect("log").push(command.to_string());
        let (verb, argument) = command.split_once(' ').unwrap_or((command, ""));
        let [real, live, peak, integral] = self.presets;
        Ok(match verb {
            "SHOW_CONFIGURATION" => {
                format!("MODEL=927 SERIAL={} FIRMWARE=2 CHANNELS=16384", self.serial)
            }
            "SHOW_STATUS" => format!(
                "RT=11.96 LT=11.96 DT=0.00% ICR=0 ACTIVE={} TOTAL=0",
                u8::from(self.counting)
            ),
            "SHOW_DATA" => format!("DATA 16384{}", " 0".repeat(16_384)),
            "SHOW_PRESETS" => format!(
                "PRESETS REAL={:.2} LIVE={:.2} PEAK={peak} INTEG={integral}",
                real as f64 * 0.02,
                live as f64 * 0.02
            ),
            "START" => {
                self.counting = true;
                "OK".into()
            }
            "STOP" => {
                self.counting = false;
                "OK".into()
            }
            "CLEAR" | "SET_PRESET_CLEAR" => {
                if verb == "SET_PRESET_CLEAR" {
                    self.presets = [0; 4];
                }
                "OK".into()
            }
            "SET_PRESET_REAL" | "SET_PRESET_LIVE" => {
                let ticks = (argument.parse::<f64>().expect("seconds") / 0.02).round() as u64;
                self.presets[usize::from(verb == "SET_PRESET_LIVE")] = ticks;
                "OK".into()
            }
            "SET_PRESET_COUNT" => {
                self.presets[2] = argument.parse().expect("counts");
                "OK".into()
            }
            "SET_PRESET_INTEG" => {
                self.presets[3] = argument.parse().expect("counts");
                "OK".into()
            }
            other => {
                format!("ERR {other} is not a command this bridge can carry out on a Model 927")
            }
        })
    }

    fn peer(&self) -> String {
        "detector 2 through the bridge".into()
    }
}

fn input_2() -> (Input927, Arc<Mutex<Vec<String>>>) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let input = Input927 {
        serial: "9049204-2",
        counting: false,
        presets: [0; 4],
        log: Arc::clone(&log),
    };
    (input, log)
}

/// Opened the way the application opens a scanned entry the first time:
/// nothing remembered, so the serial is learned.
fn opened() -> (RemoteMcb, Arc<Mutex<Vec<String>>>) {
    let (input, log) = input_2();
    let mut remote = RemoteMcb::connect_expecting(Box::new(input), 2, "927 9049204-2", Some(""))
        .expect("a 927 input connects");
    remote.set_confirm_step(Duration::ZERO);
    (remote, log)
}

fn sent(log: &Arc<Mutex<Vec<String>>>, verb: &str) -> usize {
    log.lock()
        .expect("log")
        .iter()
        .filter(|line| line.split(' ').next() == Some(verb))
        .count()
}

#[test]
fn a_927_input_connects_as_itself_with_its_channels_and_no_calibration() {
    let (remote, _) = opened();
    let identity = remote.identity();
    assert_eq!(identity.model, "927");
    assert_eq!(identity.serial, "9049204-2");
    assert_eq!(identity.firmware, "2");
    assert_eq!(identity.channels, 16_384);
    assert_eq!(remote.spectrum().len(), 16_384);
    assert!(
        remote.spectrum().energy_calibration.is_none(),
        "a 927 stores none, so the spectrum is in channels"
    );
    assert!(!remote.is_active());
    assert!(remote.reading_untrusted().is_none());
    assert_eq!(remote.status().real_time, 11.96);
}

#[test]
fn a_remembered_input_1_refuses_the_box_answering_as_input_2() {
    let (input, _) = input_2();
    let refused =
        RemoteMcb::connect_expecting(Box::new(input), 1, "927 9049204-1", Some("9049204-1"));
    let Err(error) = refused else {
        panic!("input 2 was taken for input 1");
    };
    assert!(error.to_string().contains("9049204-2"), "{error}");
}

#[test]
fn a_927_counts_to_a_preset_stops_and_clears() {
    let (mut remote, log) = opened();
    remote
        .set_presets(Presets {
            live_time: Some(300.0),
            ..Presets::default()
        })
        .expect("a live preset is a register a 927 has");
    remote.start().expect("start");
    for _ in 0..4 {
        remote.poll(0.5).expect("a poll of a counting 927");
    }
    assert!(remote.is_active());
    remote.stop().expect("stop is confirmed by the next status");
    assert!(!remote.is_active());
    remote.clear().expect("clear");
    assert!(remote.reading_untrusted().is_none());
    assert_eq!(sent(&log, "SET_PRESET_LIVE"), 1);
    assert_eq!(remote.properties().presets.live_time, Some(300.0));
}

#[test]
fn presets_a_927_already_holds_are_read_back_on_connect() {
    let (mut input, _) = input_2();
    input.presets = [0, 15_000, 0, 0];
    let remote =
        RemoteMcb::connect_expecting(Box::new(input), 2, "927 9049204-2", None).expect("connects");
    assert_eq!(remote.properties().presets.live_time, Some(300.0));
}

/// A 927 has no bias supply and no coarse gain register, and the helper says
/// so. The application must take that as a refusal: one command sent, an
/// error back, and the mirror left saying what it said before.
#[test]
fn bias_and_coarse_gain_are_refused_once_and_the_mirror_does_not_move() {
    let (mut remote, log) = opened();
    let bias = remote.properties().high_voltage.target_volts;
    let gain = remote.properties().amplifier.coarse_gain;

    let error = remote
        .set_high_voltage(1000.0)
        .expect_err("no bias on a 927");
    assert!(error.to_string().contains("Model 927"), "{error}");
    let error = remote
        .set_coarse_gain(2.0)
        .expect_err("no coarse gain on a 927");
    assert!(error.to_string().contains("Model 927"), "{error}");

    assert_eq!(remote.properties().high_voltage.target_volts, bias);
    assert_eq!(remote.properties().amplifier.coarse_gain, gain);
    assert_eq!(sent(&log, "SET_HV"), 1, "refused once, never retried");
    assert_eq!(
        sent(&log, "SET_GAIN_COARSE"),
        1,
        "refused once, never retried"
    );

    // The refusals leave the reading alone: polling carries on, believed.
    for _ in 0..4 {
        remote.poll(0.5).expect("a poll after a refusal");
    }
    assert!(remote.reading_untrusted().is_none());
}
