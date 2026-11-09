//! Which bound adapter a detector number means.
//!
//! Windows keeps an interface record for every adapter ever bound, so the
//! registry list outlives the cable: a 926 unplugged months ago is still in it,
//! in whatever order the records were made. Counting a detector number down
//! that list opens a ghost, or on a bench with two 926s the other instrument.
//! The rule `docs/ortec-hardware.md` gives is to take what actually opens, so a
//! position here counts only the adapters present now. When the detector's
//! configured name (MAESTRO's list) carries an adapter serial, that serial
//! decides instead of the position, and a serial that is not connected is a
//! refusal rather than a guess.
//!
//! Kept free of Windows calls so the rule is tested on every platform; the
//! bridge supplies the registry list and the "does it open" check.

/// The adapter serial out of a device path.
///
/// A path reads `\\?\USB#VID_0A2D&PID_0016#08134079#{guid}`, and the serial is
/// the field before the interface GUID.
pub(crate) fn serial_of(path: &str) -> String {
    path.split('#')
        .nth(2)
        .filter(|field| !field.is_empty())
        .unwrap_or(path)
        .to_string()
}

/// Picks the path for detector `number`.
///
/// `paths` is every record the driver ever made, in registry order; `present`
/// says whether one is connected now (it opens, or this process already holds
/// it - an adapter opens in one process at a time, so one the hub holds would
/// otherwise look absent and shift every position after it). `named` is the
/// detector's configured name, when there is one to check against.
pub(crate) fn adapter_for(
    number: i32,
    paths: &[String],
    present: &mut dyn FnMut(&str) -> bool,
    named: Option<&str>,
) -> Result<String, String> {
    let live: Vec<&String> = paths.iter().filter(|path| present(path)).collect();
    let serials = |list: &[&String]| {
        list.iter()
            .map(|path| serial_of(path))
            .collect::<Vec<_>>()
            .join(", ")
    };
    if let Some(name) = named {
        // A serial in the configured name is what was asked for, whatever the
        // position says. Checked against every record, so a name naming an
        // adapter that is not plugged in is reported as such.
        if let Some(path) = paths.iter().find(|path| mentions(name, &serial_of(path))) {
            if live.contains(&path) {
                return Ok(path.clone());
            }
            return Err(format!(
                "detector {number} ({name}) is adapter {}, which is not connected; \
                 connected now: {}",
                serial_of(path),
                if live.is_empty() {
                    "none".to_string()
                } else {
                    serials(&live)
                }
            ));
        }
    }
    let index = usize::try_from(number)
        .ok()
        .filter(|index| *index >= 1 && *index <= live.len())
        .ok_or_else(|| {
            let stale = paths.len() - live.len();
            format!(
                "no adapter in position {number}: {} connected ({}){}",
                live.len(),
                if live.is_empty() {
                    "none".to_string()
                } else {
                    serials(&live)
                },
                if stale == 0 {
                    String::new()
                } else {
                    format!(", {stale} more recorded but not connected")
                }
            )
        })?;
    live.get(index - 1)
        .map(|path| (*path).clone())
        .ok_or_else(|| format!("no adapter in position {number}"))
}

/// What Scan reports when ORTEC's library is not on this machine.
///
/// Scan lists MAESTRO's detectors, which only `Mcbcio32.dll` can read; the USB
/// road needs none of it. So the refusal says which adapters answer over USB
/// and how to open one without the library: Add a local instrument, detector
/// number 1 (the first adapter connected), which the bridge tries over USB
/// before it ever looks for the library.
pub(crate) fn no_library_message(error: &str, connected: &[String]) -> String {
    let found = match connected {
        [] => "no 926 answers over USB either: check the cable, the power and \
               ORTEC's USB driver (an adapter already open in MantaRay or MAESTRO \
               does not answer a scan)"
            .to_string(),
        [one] => format!("adapter {one} answers over USB"),
        many => format!(
            "{} adapters answer over USB ({}); detector #1 is the first, #2 the next",
            many.len(),
            many.join(", ")
        ),
    };
    format!(
        "Scan needs ORTEC's library and could not load it ({error}). The USB road does \
         not: {found}. Use Services > Edit Detector List > Add a local instrument, set \
         detector # to 1 and press Open."
    )
}

/// Whether `name` mentions `serial` as a whole number, not as part of a longer
/// one (so serial 0813407 is not found in 08134079).
fn mentions(name: &str, serial: &str) -> bool {
    if serial.is_empty() || !serial.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return false;
    }
    name.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| word.eq_ignore_ascii_case(serial))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(serial: &str) -> String {
        format!("\\\\?\\USB#VID_0A2D&PID_0016#{serial}#{{guid}}")
    }

    fn records() -> Vec<String> {
        // A ghost recorded first, then the two 926s on the bench today.
        vec![path("07000001"), path("08134079"), path("11217584")]
    }

    fn connected(path: &str) -> bool {
        !path.contains("07000001")
    }

    #[test]
    fn the_serial_comes_out_of_the_path() {
        assert_eq!(serial_of(&path("08134079")), "08134079");
        assert_eq!(serial_of("odd"), "odd");
    }

    #[test]
    fn a_position_counts_only_connected_adapters() {
        // Before, detector 1 opened the ghost record and detector 2 the first
        // real 926, so each detector reached the wrong instrument.
        let paths = records();
        assert_eq!(
            adapter_for(1, &paths, &mut connected, None),
            Ok(path("08134079"))
        );
        assert_eq!(
            adapter_for(2, &paths, &mut connected, None),
            Ok(path("11217584"))
        );
    }

    #[test]
    fn a_position_past_the_connected_ones_names_what_is_there() {
        let error = adapter_for(3, &records(), &mut connected, None).unwrap_err();
        assert!(error.contains("2 connected"), "{error}");
        assert!(error.contains("08134079, 11217584"), "{error}");
        assert!(
            error.contains("1 more recorded but not connected"),
            "{error}"
        );
        assert!(adapter_for(0, &records(), &mut connected, None).is_err());
        assert!(adapter_for(-1, &records(), &mut connected, None).is_err());
        let error = adapter_for(1, &records(), &mut |_| false, None).unwrap_err();
        assert!(error.contains("0 connected (none)"), "{error}");
    }

    #[test]
    fn a_serial_in_the_configured_name_beats_the_position() {
        let paths = records();
        assert_eq!(
            adapter_for(1, &paths, &mut connected, Some("926 MCB 11217584")),
            Ok(path("11217584"))
        );
    }

    #[test]
    fn a_named_adapter_that_is_not_connected_is_refused() {
        let error = adapter_for(1, &records(), &mut connected, Some("MCB 07000001")).unwrap_err();
        assert!(
            error.contains("adapter 07000001, which is not connected"),
            "{error}"
        );
        assert!(error.contains("08134079, 11217584"), "{error}");
    }

    #[test]
    fn a_name_without_a_known_serial_falls_back_to_the_position() {
        assert_eq!(
            adapter_for(2, &records(), &mut connected, Some("0926-001 USB MCB")),
            Ok(path("11217584"))
        );
    }

    #[test]
    fn a_serial_only_matches_as_a_whole_word() {
        assert!(mentions("MCB 08134079", "08134079"));
        assert!(!mentions("MCB 081340790", "08134079"));
        assert!(!mentions("MCB 08134079", ""));
    }

    #[test]
    fn scan_without_the_library_points_at_add_a_local_instrument() {
        let error = "could not load Mcbcio32.dll";
        let none = no_library_message(error, &[]);
        assert!(none.contains("could not load Mcbcio32.dll"), "{none}");
        assert!(none.contains("no 926 answers over USB"), "{none}");
        let one = no_library_message(error, &["08134079".into()]);
        assert!(one.contains("adapter 08134079 answers over USB"), "{one}");
        assert!(one.contains("Add a local instrument"), "{one}");
        assert!(one.contains("detector # to 1"), "{one}");
        let two = no_library_message(error, &["08134079".into(), "11217584".into()]);
        assert!(
            two.contains("2 adapters answer over USB (08134079, 11217584)"),
            "{two}"
        );
    }

    #[test]
    fn presence_is_asked_once_per_record() {
        let mut asked = Vec::new();
        let paths = records();
        let _ = adapter_for(
            1,
            &paths,
            &mut |p| {
                asked.push(p.to_string());
                true
            },
            None,
        );
        assert_eq!(asked, paths);
    }
}
