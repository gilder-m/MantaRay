//! A crash snapshot written by a released mantaray, which every later version
//! must still offer back.
//!
//! `golden/v0.2.4/session.json` was written by the code at the `v0.2.4-alpha`
//! tag (two buffer windows over a made-up Cs-137 count, one with calibration
//! points entered and one pulled out into its own window). It is never
//! regenerated: a snapshot is the only copy of unsaved work, and the version
//! that reads it is often not the one that wrote it - the crash is what makes
//! somebody update.

use mantaray_core::CalibrationPoint;
use mantaray_gui::session::{self, Session};
use mantaray_gui::viewmodel::{FillMode, VerticalScale};
use serde_json::Value;

const SESSION: &str = include_str!("golden/v0.2.4/session.json");

fn parse(text: &str) -> Result<Session, String> {
    session::parse(text).map_err(|error| error.to_string())
}

fn render(session: &Session) -> String {
    session::to_text(session).expect("a snapshot")
}

/// Every field of `old` is in `new` with the same value; `new` may add more.
fn assert_kept(old: &Value, new: &Value, at: &str) {
    match (old, new) {
        (Value::Object(old), Value::Object(new)) => {
            for (key, value) in old {
                let Some(other) = new.get(key) else {
                    panic!("{at}.{key} was in the old snapshot and is gone");
                };
                assert_kept(value, other, &format!("{at}.{key}"));
            }
        }
        (Value::Array(old), Value::Array(new)) => {
            assert_eq!(old.len(), new.len(), "{at}: length changed");
            for (index, (a, b)) in old.iter().zip(new).enumerate() {
                assert_kept(a, b, &format!("{at}[{index}]"));
            }
        }
        _ => assert_eq!(old, new, "{at} changed"),
    }
}

fn edited(edit: impl FnOnce(&mut Value)) -> String {
    let mut value: Value = serde_json::from_str(SESSION).expect("JSON");
    edit(&mut value);
    value.to_string()
}

#[test]
fn a_v0_2_4_snapshot_reads_back_field_for_field() {
    let session = parse(SESSION).expect("the v0.2.4 snapshot opens");
    assert_eq!(session.saved_at, "2026-10-14 10:02");
    assert_eq!(session.windows.len(), 2);

    let first = &session.windows[0];
    assert_eq!(first.title, "cs137_shelf2.Spe");
    assert_eq!(
        first.path.as_deref().and_then(|path| path.to_str()),
        Some("C:/Lab/2026-10-14/cs137_shelf2.Spe")
    );
    assert_eq!(first.spectrum.len(), 512);
    assert_eq!(first.spectrum.channels[331], 2661);
    assert_eq!(first.spectrum.live_time, 1798.42);
    assert_eq!(first.spectrum.detector_name, "DET-01");
    assert_eq!(first.display.marker, 331);
    assert_eq!(first.display.view_start, 256);
    assert_eq!(first.display.view_width, 128);
    assert!(first.display.isotope_markers);
    assert_eq!(
        first.calibration.points(),
        [
            CalibrationPoint::new(59.3, 118.7),
            CalibrationPoint::new(331.0, 661.657)
        ]
    );
    assert!(first.modified);
    assert!(!first.floating);

    let second = &session.windows[1];
    assert_eq!(second.title, "Recovered");
    assert_eq!(second.path, None);
    assert_eq!(second.spectrum.source, None);
    assert_eq!(second.display.vertical, VerticalScale::Linear(5000));
    assert_eq!(second.display.fill, FillMode::Points);
    assert!(!second.display.energy_axis);
    assert!(second.calibration.points().is_empty());
    assert!(second.floating);

    let old: Value = serde_json::from_str(SESSION).expect("JSON");
    let new: Value = serde_json::from_str(&render(&session)).expect("JSON");
    assert_kept(&old, &new, "snapshot");
}

#[test]
fn a_snapshot_with_fields_this_version_does_not_know_still_reads() {
    let text = edited(|value| {
        value["workspace"] = Value::from("Lab 2");
        value["windows"][0]["pinned"] = Value::from(true);
        value["windows"][0]["display"]["grid"] = Value::from("major");
        value["windows"][0]["calibration"]["fit"] = Value::from("quadratic");
    });
    let session = parse(&text).expect("unknown fields are ignored");
    assert_eq!(session.windows.len(), 2);
    assert_eq!(session.windows[0].display.marker, 331);
}

#[test]
fn a_snapshot_missing_later_display_and_calibration_fields_reads() {
    // A display setting added after the snapshot was written must not cost
    // the operator the spectrum underneath it.
    let text = edited(|value| {
        for window in value["windows"].as_array_mut().expect("windows") {
            let display = window["display"].as_object_mut().expect("display");
            display.remove("peak_labels");
            display.remove("log_decade_top");
            display.remove("length");
            window["calibration"]
                .as_object_mut()
                .expect("calibration")
                .remove("units");
            window["spectrum"]
                .as_object_mut()
                .expect("spectrum")
                .remove("detector_description");
        }
    });
    let session = parse(&text).expect("missing later fields default");
    assert_eq!(session.windows.len(), 2);
    assert_eq!(session.windows[0].spectrum.channels[331], 2661);
    assert_eq!(session.windows[0].display.marker, 331);
    assert!(session.windows[0].display.peak_labels);
    assert_eq!(session.windows[0].calibration.points().len(), 2);
    assert_eq!(session.windows[0].calibration.units(), "keV");
}

#[test]
fn a_snapshot_from_a_newer_format_is_refused_naming_both_versions() {
    let text = edited(|value| {
        value["mantaray"] = Value::from("session/2");
        value["written_by"] = Value::from("mantaray 9.1.0");
    });
    let message = parse(&text).expect_err("a session/2 snapshot must not be guessed at");
    assert!(message.contains("session/2"), "{message}");
    assert!(message.contains("session/1"), "{message}");
    assert!(message.contains("mantaray 9.1.0"), "{message}");
}

#[test]
fn a_snapshot_written_now_carries_its_format_and_writer() {
    let session = parse(SESSION).expect("golden");
    let value: Value = serde_json::from_str(&render(&session)).expect("JSON");
    assert_eq!(value["mantaray"], "session/1");
    assert_eq!(
        value["written_by"],
        format!("mantaray {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn a_snapshot_from_a_newer_mantaray_is_kept_and_never_written_over() {
    // It is somebody's unsaved work. This build cannot read it, but the
    // version that wrote it can, so it must survive this run untouched.
    let text = edited(|value| value["mantaray"] = Value::from("session/2"));
    let reason = parse(&text).expect_err("newer");
    let mut app = mantaray_gui::app::App::headless();
    app.offer_unfinished_work(Err(reason));
    assert!(app.recovered.is_none(), "nothing this build could offer");
    assert!(
        !app.may_replace_snapshot(),
        "neither the autosave nor a clean exit may touch it"
    );
    assert!(app.status.contains("session/2"), "{}", app.status);
    assert!(
        !app.dialogs.is_open(mantaray_gui::dialogs::Dialog::Recover),
        "no offer of work that cannot be opened"
    );
}

#[test]
fn a_v0_2_4_snapshot_is_offered_back() {
    let mut app = mantaray_gui::app::App::headless();
    app.offer_unfinished_work(Ok(Some(parse(SESSION).expect("golden"))));
    assert_eq!(app.recovered.as_ref().map(|s| s.windows.len()), Some(2));
    assert!(app.dialogs.is_open(mantaray_gui::dialogs::Dialog::Recover));
}
