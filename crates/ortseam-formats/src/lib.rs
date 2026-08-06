//! Readers and writers for gamma-spectroscopy files.
//!
//! | Format | Read | Write | Notes |
//! |---|---|---|---|
//! | `.Chn` | yes | yes | ORTEC integer binary, layout from the MAESTRO manual |
//! | `.Spe` | yes | yes | IAEA/CTBTO ASCII |
//! | `.Spc` | histogram only | no | see [`spc`] for why |
//! | `.Roi` | yes | yes | channel pairs |
//! | `.txt`/`.asc` | yes | yes | TRANSLT-style ASCII dump |
//! | `.json` | yes | yes | native, lossless |
//! | `.Lis` | yes | yes | native list mode (the ORTEC one is instrument specific) |
//! | `.Lib` | yes | as JSON | ORTEC binary nuclide library, File Structure Manual §6.1 |
//!
//! [`load_spectrum`] and [`save_spectrum`] dispatch on the file extension.

#![forbid(unsafe_code)]

pub mod ascii;
pub mod chn;
pub mod library;
pub mod list_mode;
pub mod n42;
pub mod native;
pub mod roi;
pub mod spc;
pub mod spe;

use std::path::Path;

use ortseam_core::{RoiSet, Spectrum};
use thiserror::Error;

pub use ascii::AsciiOptions;
pub use list_mode::{ListEvent, ListModeFile, ListRange};

/// The most channels any reader will size an allocation for.
///
/// Every channel count in these files arrives from outside and feeds a
/// `Vec::with_capacity`; a corrupt or hostile length must be an error, never
/// a multi-gigabyte allocation and an uncatchable abort. No real MCB reaches
/// a million channels - the largest here is 16k.
pub(crate) const MOST_CHANNELS: usize = 1 << 20;

/// Anything that can go wrong reading or writing a spectrum file.
#[derive(Debug, Error)]
pub enum FormatError {
    /// The file does not start with the expected marker.
    #[error("not a {expected} file (found {found})")]
    BadMagic {
        /// What was expected.
        expected: String,
        /// What was found.
        found: String,
    },
    /// The file ends before the data it promises.
    #[error("file is truncated: needed {expected} bytes, found {got}")]
    Truncated {
        /// Bytes required.
        expected: usize,
        /// Bytes available.
        got: usize,
    },
    /// A required keyword or column is missing.
    #[error("missing {field}")]
    MissingField {
        /// Name of the missing item.
        field: String,
    },
    /// A value could not be parsed.
    #[error("invalid {what}: {detail}")]
    Invalid {
        /// What was being parsed.
        what: String,
        /// Why it failed.
        detail: String,
    },
    /// The format, or this operation on it, is not supported.
    #[error("unsupported format: {detail}")]
    Unsupported {
        /// Explanation, including the extension or format name.
        detail: String,
    },
    /// Filesystem failure.
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// JSON failure.
    #[error("{0}")]
    Json(#[from] serde_json::Error),
}

impl FormatError {
    /// Shorthand for [`FormatError::Invalid`].
    pub fn invalid(what: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::Invalid {
            what: what.into(),
            detail: detail.into(),
        }
    }

    /// Shorthand for [`FormatError::MissingField`].
    pub fn missing(field: impl Into<String>) -> Self {
        Self::MissingField {
            field: field.into(),
        }
    }
}

/// The spectrum file formats ortseam knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpectrumFormat {
    /// ORTEC integer `.Chn`.
    Chn,
    /// ORTEC `.Spc`.
    Spc,
    /// IAEA ASCII `.Spe`.
    Spe,
    /// Plain ASCII channel dump.
    Ascii,
    /// Native lossless JSON.
    Native,
    /// List-mode event file.
    List,
    /// ANSI N42.42 XML.
    N42,
}

impl SpectrumFormat {
    /// Every known format.
    pub fn all() -> &'static [SpectrumFormat] {
        &[
            Self::Chn,
            Self::Spc,
            Self::Spe,
            Self::Ascii,
            Self::Native,
            Self::List,
            Self::N42,
        ]
    }

    /// Recognises a format from a path's extension.
    pub fn from_path(path: impl AsRef<Path>) -> Option<Self> {
        let extension = path.as_ref().extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "chn" => Some(Self::Chn),
            "spc" => Some(Self::Spc),
            "spe" => Some(Self::Spe),
            "txt" | "asc" | "ascii" => Some(Self::Ascii),
            "json" | "ortspec" => Some(Self::Native),
            "lis" => Some(Self::List),
            "n42" | "xml" => Some(Self::N42),
            _ => None,
        }
    }

    /// Canonical extension, without the dot.
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Chn => "chn",
            Self::Spc => "spc",
            Self::Spe => "spe",
            Self::Ascii => "txt",
            Self::Native => "json",
            Self::List => "lis",
            Self::N42 => "n42",
        }
    }

    /// Human-readable name for file dialogs.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Chn => "Integer .Chn spectrum",
            Self::Spc => "ORTEC .Spc spectrum",
            Self::Spe => "IAEA ASCII .Spe spectrum",
            Self::Ascii => "ASCII channel dump",
            Self::Native => "ortseam native spectrum",
            Self::List => "List-mode data",
            Self::N42 => "ANSI N42.42 spectrum",
        }
    }

    /// True for binary formats.
    pub fn is_binary(&self) -> bool {
        matches!(self, Self::Chn | Self::Spc | Self::List)
    }

    /// True when ortseam can write the format.
    pub fn can_write(&self) -> bool {
        !matches!(self, Self::Spc | Self::List | Self::N42)
    }
}

/// Recognises a format from the beginning of a file's contents.
///
/// Extensions are only a convention, and real collections are full of
/// `SPECTRUM.001`, `run3.dat` and files with no extension at all. [`load_spectrum`]
/// falls back to this so that such a file still opens.
pub fn sniff(bytes: &[u8]) -> Option<SpectrumFormat> {
    if bytes.len() >= 8 && bytes.starts_with(b"ORTSLIST") {
        return Some(SpectrumFormat::List);
    }
    // The text formats announce themselves in their first few hundred bytes.
    let head = &bytes[..bytes.len().min(4096)];
    if let Ok(text) = std::str::from_utf8(head) {
        let trimmed = text.trim_start();
        // N42 is XML, and the element that names it comes early.
        if trimmed.starts_with('<')
            && (text.contains("N42InstrumentData")
                || text.contains("RadInstrumentData")
                || text.contains("<Spectrum"))
        {
            return Some(SpectrumFormat::N42);
        }
        if trimmed.starts_with("$SPEC_ID:")
            || trimmed.starts_with('$') && text.contains("$DATA:")
            || text.contains("$MEAS_TIM:")
        {
            return Some(SpectrumFormat::Spe);
        }
        if trimmed.starts_with('{') && (text.contains("\"channels\"") || text.contains("\"len\"")) {
            return Some(SpectrumFormat::Native);
        }
        if looks_like_ascii_channels(trimmed) {
            return Some(SpectrumFormat::Ascii);
        }
    }
    let word = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
    // `.Chn` opens with the marker -1, and carries its channel count at byte 30.
    // The count is checked too: a run of 0xff bytes is not a spectrum.
    if bytes.len() >= chn::HEADER_SIZE && bytes[0] == 0xff && bytes[1] == 0xff {
        let channels = word(30) as usize;
        let expected = chn::HEADER_SIZE + 4 * channels;
        if channels > 0 && bytes.len() >= expected {
            return Some(SpectrumFormat::Chn);
        }
    }
    // `.Spc` record 1: INFTYP 1 for a spectrum, then FILTYP 1 or 2, and a channel
    // count whose data records fit in the file.
    if bytes.len() >= 2 * spc::RECORD_SIZE {
        let channels = word(64) as usize;
        let records = word(62) as usize;
        if word(0) == 1
            && (word(2) == 1 || word(2) == 2)
            && channels > 0
            && records * spc::CHANNELS_PER_RECORD >= channels
        {
            return Some(SpectrumFormat::Spc);
        }
    }
    None
}

/// Whether text looks like a channel dump: numbers, and little else.
fn looks_like_ascii_channels(text: &str) -> bool {
    let mut counted = 0usize;
    for line in text.lines().take(24) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        // One or two numeric columns: "counts" or "channel counts".
        let mut fields = line.split_whitespace();
        let ok = match (fields.next(), fields.next(), fields.next()) {
            (Some(first), None, None) => first.parse::<f64>().is_ok(),
            (Some(first), Some(second), None) => {
                first.parse::<f64>().is_ok() && second.parse::<f64>().is_ok()
            }
            _ => false,
        };
        if !ok {
            return false;
        }
        counted += 1;
    }
    counted >= 3
}

/// Loads a spectrum, choosing the reader by extension.
///
/// A list-mode file is loaded as the histogram of all its events.
pub fn load_spectrum(path: impl AsRef<Path>) -> Result<Spectrum, FormatError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path)?;
    // The name comes first, because it is what the user chose; the contents are
    // the fallback, both for files whose name says nothing and for files whose
    // name is simply wrong.
    let named = SpectrumFormat::from_path(path);
    let sniffed = sniff(&bytes);
    let format = named
        .or(sniffed)
        .ok_or_else(|| unsupported(path, "reader"))?;
    let mut spectrum = match read_as(format, &bytes) {
        Ok(spectrum) => spectrum,
        Err(named_error) => match sniffed.filter(|other| *other != format) {
            Some(other) => read_as(other, &bytes).map_err(|_| named_error)?,
            None => return Err(named_error),
        },
    };
    spectrum.source = Some(path.display().to_string());
    Ok(spectrum)
}

/// Reads a spectrum from bytes already in hand, in a known format.
///
/// Text formats are decoded leniently: a stray non-UTF-8 byte in a comment or a
/// sample description is not a reason to refuse a spectrum.
pub fn read_as(format: SpectrumFormat, bytes: &[u8]) -> Result<Spectrum, FormatError> {
    let text = || String::from_utf8_lossy(bytes).into_owned();
    match format {
        SpectrumFormat::Chn => chn::read(bytes),
        SpectrumFormat::Spc => spc::read(bytes),
        SpectrumFormat::Spe => spe::read(&text()),
        SpectrumFormat::Ascii => ascii::read(&text()),
        SpectrumFormat::Native => native::read(&text()),
        SpectrumFormat::List => list_mode::read(bytes).map(|file| file.to_spectrum()),
        SpectrumFormat::N42 => n42::from_str(&text()),
    }
}

/// Saves a spectrum, choosing the writer by extension.
pub fn save_spectrum(spectrum: &Spectrum, path: impl AsRef<Path>) -> Result<(), FormatError> {
    let path = path.as_ref();
    let format = SpectrumFormat::from_path(path).ok_or_else(|| unsupported(path, "writer"))?;
    match format {
        SpectrumFormat::Chn => std::fs::write(path, chn::write(spectrum)?)?,
        SpectrumFormat::Spc => std::fs::write(path, spc::write(spectrum)?)?,
        SpectrumFormat::Spe => std::fs::write(path, spe::write(spectrum)?)?,
        SpectrumFormat::Ascii => {
            std::fs::write(path, ascii::write(spectrum, &AsciiOptions::default()))?
        }
        SpectrumFormat::Native => std::fs::write(path, native::write(spectrum)?)?,
        SpectrumFormat::List => {
            return Err(FormatError::Unsupported {
                detail: "writing .lis files from a histogram (list mode needs events)".into(),
            });
        }
        SpectrumFormat::N42 => {
            return Err(FormatError::Unsupported {
                detail: "writing N42 (only reading is implemented)".into(),
            });
        }
    }
    Ok(())
}

fn unsupported(path: &Path, what: &str) -> FormatError {
    FormatError::Unsupported {
        detail: format!(
            "{} (no {what} for that extension)",
            path.extension()
                .and_then(|e| e.to_str())
                .unwrap_or("file without an extension")
        ),
    }
}

/// Writes a `.Roi` region file.
pub fn save_rois(rois: &RoiSet, path: impl AsRef<Path>) -> Result<(), FormatError> {
    std::fs::write(path, roi::write(rois))?;
    Ok(())
}

/// Reads a `.Roi` region file.
pub fn load_rois(path: impl AsRef<Path>) -> Result<RoiSet, FormatError> {
    roi::read(&std::fs::read(path)?)
}

/// Reads little-endian scalars out of a byte slice with bounds checks.
pub(crate) struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    pub(crate) fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }

    pub(crate) fn take(&mut self, count: usize) -> Result<&'a [u8], FormatError> {
        if self.remaining() < count {
            return Err(FormatError::Truncated {
                expected: self.position + count,
                got: self.bytes.len(),
            });
        }
        let slice = &self.bytes[self.position..self.position + count];
        self.position += count;
        Ok(slice)
    }

    pub(crate) fn i16(&mut self) -> Result<i16, FormatError> {
        let bytes = self.take(2)?;
        Ok(i16::from_le_bytes([bytes[0], bytes[1]]))
    }

    pub(crate) fn u16(&mut self) -> Result<u16, FormatError> {
        Ok(self.i16()? as u16)
    }

    pub(crate) fn i32(&mut self) -> Result<i32, FormatError> {
        let bytes = self.take(4)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, FormatError> {
        Ok(self.i32()? as u32)
    }

    pub(crate) fn f32(&mut self) -> Result<f32, FormatError> {
        let bytes = self.take(4)?;
        Ok(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub(crate) fn f64(&mut self) -> Result<f64, FormatError> {
        let bytes = self.take(8)?;
        let mut array = [0u8; 8];
        array.copy_from_slice(bytes);
        Ok(f64::from_le_bytes(array))
    }

    pub(crate) fn skip(&mut self, count: usize) -> Result<(), FormatError> {
        self.take(count)?;
        Ok(())
    }
}
