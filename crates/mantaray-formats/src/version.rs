//! Format versions of the files mantaray writes itself.
//!
//! Every such file carries a tag `<kind>/<major>` (or `<kind>/<major>.<minor>`)
//! under the key `mantaray`, and the version of the program that wrote it under
//! `written_by`. The rule a reader keeps:
//!
//! - a minor step only adds fields, so `spectrum/1.5` still opens here - the
//!   fields it adds are ignored, and every field a later version adds must
//!   have a default so older files open in newer versions too;
//! - a major step changes what is there, so a major this build does not know
//!   is refused, naming both versions, rather than read and misread;
//! - a file with no tag at all predates tags and is read as major 1, which is
//!   the only layout there was.
//!
//! `written_by` is never used to decide anything. It is there so the message
//! for a newer file can say which version to open it with, and so a file
//! handed round a lab says where it came from.

use serde::{Deserialize, Serialize};

use crate::FormatError;

/// What goes in `written_by`: this program and its version.
pub const WRITER: &str = concat!("mantaray ", env!("CARGO_PKG_VERSION"));

/// A document's own fields with the tag and writer stamp in front of them.
///
/// The two keys sit beside the document's fields rather than wrapping them,
/// so the file keeps the shape it always had: a version that predates tags
/// reads it as before and ignores the two keys it does not know.
#[derive(Serialize)]
pub struct Tagged<'a, T: Serialize> {
    /// Format tag, e.g. `library/1`.
    pub mantaray: &'static str,
    /// [`WRITER`].
    pub written_by: &'static str,
    /// The document itself.
    #[serde(flatten)]
    pub body: &'a T,
}

impl<'a, T: Serialize> Tagged<'a, T> {
    /// Tags `body` as written by this build.
    pub fn new(tag: &'static str, body: &'a T) -> Self {
        Self {
            mantaray: tag,
            written_by: WRITER,
            body,
        }
    }
}

/// Just the tag and stamp of a document, read before the rest of it so the
/// version decides how the rest is read.
#[derive(Deserialize)]
pub struct Probe {
    /// Format tag; absent in files written before the kind had one.
    #[serde(default)]
    pub mantaray: Option<String>,
    /// Version that wrote the file, when it says.
    #[serde(default)]
    pub written_by: Option<String>,
}

impl Probe {
    /// Reads the tag and stamp of a JSON document and checks them, returning
    /// the major version (see [`check`]).
    ///
    /// # Errors
    ///
    /// [`FormatError::Json`] when the text is not a JSON object, otherwise as
    /// [`check`].
    pub fn check(text: &str, kind: &str, newest: u32) -> Result<u32, FormatError> {
        let probe: Self = serde_json::from_str(text)?;
        check(
            kind,
            probe.mantaray.as_deref(),
            newest,
            probe.written_by.as_deref(),
        )
    }
}

/// Checks a file's tag against the newest major version this build reads.
///
/// `tag` is `None` for a file written before the kind carried a tag, which is
/// read as major 1. Returns the major version, for the caller to dispatch on.
///
/// # Errors
///
/// [`FormatError::BadMagic`] for a tag of another kind,
/// [`FormatError::Invalid`] for a version that is not `<major>[.<minor>]`
/// with a major of at least 1, and [`FormatError::TooNew`] for a major newer
/// than `newest`.
pub fn check(
    kind: &str,
    tag: Option<&str>,
    newest: u32,
    written_by: Option<&str>,
) -> Result<u32, FormatError> {
    let supported = format!("{kind}/{newest}");
    let Some(tag) = tag else {
        return Ok(1);
    };
    let Some(version) = tag
        .strip_prefix(kind)
        .and_then(|rest| rest.strip_prefix('/'))
    else {
        return Err(FormatError::BadMagic {
            expected: supported,
            found: tag.to_string(),
        });
    };
    let (major, minor) = version.split_once('.').unwrap_or((version, "0"));
    let garbled = || FormatError::invalid(format!("{kind} format version"), format!("{tag:?}"));
    let major: u32 = major.parse().map_err(|_| garbled())?;
    minor.parse::<u32>().map_err(|_| garbled())?;
    if major == 0 {
        return Err(garbled());
    }
    if major > newest {
        return Err(FormatError::TooNew {
            found: tag.to_string(),
            supported,
            written_by: written_by.unwrap_or("a newer mantaray").to_string(),
        });
    }
    Ok(major)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_untagged_file_is_the_first_layout() {
        assert_eq!(check("library", None, 1, None).unwrap(), 1);
    }

    #[test]
    fn minor_steps_open_and_major_steps_do_not() {
        assert_eq!(check("spectrum", Some("spectrum/1"), 1, None).unwrap(), 1);
        assert_eq!(check("spectrum", Some("spectrum/1.5"), 1, None).unwrap(), 1);
        let error = check("spectrum", Some("spectrum/2.0"), 1, Some("mantaray 0.4.0")).unwrap_err();
        assert_eq!(
            error.to_string(),
            "this file is format spectrum/2.0, written by mantaray 0.4.0; this is \
             mantaray 0.2.4, which reads up to spectrum/1 - open it with a newer mantaray"
                .replace("0.2.4", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn a_tag_for_another_kind_is_bad_magic() {
        assert!(matches!(
            check("library", Some("spectrum/1"), 1, None),
            Err(FormatError::BadMagic { .. })
        ));
        assert!(matches!(
            check("spectrum", Some("spectrum1"), 1, None),
            Err(FormatError::BadMagic { .. })
        ));
    }

    #[test]
    fn a_garbled_version_is_refused() {
        for tag in [
            "spectrum/",
            "spectrum/x",
            "spectrum/0",
            "spectrum/1.x",
            "spectrum/-1",
        ] {
            assert!(check("spectrum", Some(tag), 1, None).is_err(), "{tag}");
        }
    }
}
