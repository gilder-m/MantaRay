//! ortseam's own lossless spectrum file: JSON with a version tag.
//!
//! Unlike `.Chn`, `.Spe` and `.Spc`, this format carries the complete in-memory
//! model - regions, both calibrations, descriptions, acquisition mode and the
//! source it came from - so it round trips exactly.

use ortseam_core::Spectrum;
use serde::{Deserialize, Serialize};

use crate::FormatError;

/// Format tag written into every file.
pub const FORMAT_TAG: &str = "spectrum/1";

#[derive(Serialize, Deserialize)]
struct Document {
    /// Format tag, e.g. `spectrum/1`.
    ortseam: String,
    /// The spectrum itself.
    spectrum: Spectrum,
}

/// Serialises a spectrum as pretty-printed JSON.
pub fn write(spectrum: &Spectrum) -> Result<String, FormatError> {
    let document = Document {
        ortseam: FORMAT_TAG.to_string(),
        spectrum: spectrum.clone(),
    };
    Ok(serde_json::to_string_pretty(&document)?)
}

/// Parses a native JSON spectrum.
pub fn read(text: &str) -> Result<Spectrum, FormatError> {
    let document: Document = serde_json::from_str(text)?;
    if !document.ortseam.starts_with("spectrum/") {
        return Err(FormatError::BadMagic {
            expected: FORMAT_TAG.to_string(),
            found: document.ortseam,
        });
    }
    Ok(document.spectrum)
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
