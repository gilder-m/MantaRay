//! mantaray's own lossless spectrum file: JSON with a version tag.
//!
//! Unlike `.Chn`, `.Spe` and `.Spc`, this format carries the complete in-memory
//! model - regions, both calibrations, descriptions, acquisition mode and the
//! source it came from - so it round trips exactly.
//!
//! Versioning follows [`crate::version`]: the tag is `spectrum/1` and has been
//! since the format began; a file from a newer major version is refused.

use mantaray_core::Spectrum;
use serde::{Deserialize, Serialize};

use crate::FormatError;
use crate::version::{Probe, WRITER, check};

/// Format tag written into every file.
pub const FORMAT_TAG: &str = "spectrum/1";

/// The newest major version of the layout this build reads.
const NEWEST: u32 = 1;

#[derive(Serialize)]
struct DocumentOut<'a> {
    /// Format tag, e.g. `spectrum/1`.
    mantaray: &'static str,
    /// Version that wrote the file; never read back to decide anything.
    written_by: &'static str,
    /// The spectrum itself.
    spectrum: &'a Spectrum,
}

#[derive(Deserialize)]
struct Document {
    /// The spectrum itself.
    spectrum: Spectrum,
}

/// Serialises a spectrum as pretty-printed JSON.
pub fn write(spectrum: &Spectrum) -> Result<String, FormatError> {
    let document = DocumentOut {
        mantaray: FORMAT_TAG,
        written_by: WRITER,
        spectrum,
    };
    Ok(serde_json::to_string_pretty(&document)?)
}

/// Parses a native JSON spectrum.
///
/// The tag is required: every file this format has ever written carries one,
/// so a JSON document without it is something else.
pub fn read(text: &str) -> Result<Spectrum, FormatError> {
    let probe: Probe = serde_json::from_str(text)?;
    let Some(tag) = probe.mantaray.as_deref() else {
        return Err(FormatError::BadMagic {
            expected: FORMAT_TAG.to_string(),
            found: "a JSON document with no format tag".to_string(),
        });
    };
    match check("spectrum", Some(tag), NEWEST, probe.written_by.as_deref())? {
        1 => Ok(serde_json::from_str::<Document>(text)?.spectrum),
        other => Err(FormatError::Unsupported {
            detail: format!("spectrum/{other}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_future_minor_version_still_loads() {
        let text = write(&Spectrum::from_counts(vec![1, 2, 3])).unwrap();
        let bumped = text.replace("spectrum/1", "spectrum/1.5");
        assert_eq!(read(&bumped).unwrap().channels, vec![1, 2, 3]);
    }

    #[test]
    fn a_wrong_tag_is_rejected() {
        let text = write(&Spectrum::from_counts(vec![1])).unwrap();
        let wrong = text.replace("spectrum/1", "library/1");
        assert!(matches!(read(&wrong), Err(FormatError::BadMagic { .. })));
    }
}
