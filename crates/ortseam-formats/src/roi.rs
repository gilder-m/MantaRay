//! `.Roi` region files: a table of channel pairs.
//!
//! From the reader published in the MAESTRO manual: the file is a sequence of
//! 16-bit words, the first two of which are a header. Regions follow as
//! `first`/`last + 1` pairs and the list ends at a word that is zero or negative.

use ortseam_core::{Roi, RoiSet};

use crate::{Cursor, FormatError};

/// Reads a `.Roi` file.
pub fn read(bytes: &[u8]) -> Result<RoiSet, FormatError> {
    let mut cursor = Cursor::new(bytes);
    // Header: two words, holding the region count in the first.
    cursor.i16()?;
    cursor.i16()?;
    let mut set = RoiSet::default();
    while cursor.remaining() >= 4 {
        let first = cursor.i16()?;
        if first <= 0 {
            break;
        }
        let end_exclusive = cursor.i16()?;
        if end_exclusive <= first {
            break;
        }
        set.mark(Roi::new(first as usize, (end_exclusive - 1) as usize));
    }
    Ok(set)
}

/// Writes a `.Roi` file.
pub fn write(rois: &RoiSet) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 4 * rois.len() + 4);
    out.extend_from_slice(&(rois.len() as i16).to_le_bytes());
    out.extend_from_slice(&0i16.to_le_bytes());
    for roi in rois.iter() {
        // Zero ends the list, so a region touching channel 0 starts at 1.
        let first = roi.start.clamp(1, i16::MAX as usize) as i16;
        let end = (roi.end + 1).min(i16::MAX as usize) as i16;
        out.extend_from_slice(&first.to_le_bytes());
        out.extend_from_slice(&end.to_le_bytes());
    }
    // Terminator.
    out.extend_from_slice(&0i16.to_le_bytes());
    out.extend_from_slice(&0i16.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_region_starting_at_channel_zero_is_shifted_into_range() {
        // Channel 0 cannot be stored, because zero terminates the list; the
        // region starts at channel 1 instead.
        let set = RoiSet::from_pairs([(0, 10)]);
        let back = read(&write(&set)).unwrap();
        assert_eq!(back.to_pairs(), vec![(1, 10)]);
    }
}
