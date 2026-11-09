//! `.Roi` region files: a table of channel pairs.
//!
//! ORTEC's *Software File Structure Manual* (§3.2) and the reader published in
//! the MAESTRO manual agree: the file is a sequence of 16-bit words, the first
//! of which is a header (−2). Regions follow as `first`/`last + 1` pairs and
//! the list ends at a start channel that is zero or negative (−1 here).
//!
//! Versions up to 0.2.4 read the manual's `REC=I+2` (FORTRAN records of one
//! word, numbered from 1) as two header words and wrote the region count, a
//! zero, then the pairs. MAESTRO reads that zero as the end of the list. Those
//! files still open: their header is positive and is followed by a zero, which
//! no ORTEC file has (a zero start channel ends the list, so it would be an
//! empty file under a positive header ORTEC never writes).

use mantaray_core::{Roi, RoiSet};

use crate::{Cursor, FormatError};

/// The header word ORTEC writes.
const HEADER: i16 = -2;

/// Reads a `.Roi` file.
pub fn read(bytes: &[u8]) -> Result<RoiSet, FormatError> {
    let mut cursor = Cursor::new(bytes);
    let header = cursor.i16()?;
    // Even an empty table has its terminator after the header.
    if cursor.remaining() < 2 {
        return Err(FormatError::Truncated {
            expected: 4,
            got: bytes.len(),
        });
    }
    // A 0.2.4 file: the region count, then a zero word before the pairs.
    if header > 0 && bytes.get(2..4) == Some(&[0, 0][..]) {
        cursor.i16()?;
    }
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

/// Writes a `.Roi` file in ORTEC's layout.
pub fn write(rois: &RoiSet) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + 4 * rois.len() + 2);
    out.extend_from_slice(&HEADER.to_le_bytes());
    for roi in rois.iter() {
        // Zero ends the list, so a region touching channel 0 starts at 1.
        // The clamp at i16::MAX is the format itself: .Roi stores sixteen-bit
        // signed channel numbers, so a region above channel 32767 cannot be
        // written faithfully - it is clamped rather than wrapped, and a
        // spectrum that needs more keeps its regions in .Spe or native JSON.
        let first = roi.start.clamp(1, i16::MAX as usize) as i16;
        let end = (roi.end + 1).min(i16::MAX as usize) as i16;
        out.extend_from_slice(&first.to_le_bytes());
        out.extend_from_slice(&end.to_le_bytes());
    }
    // Terminator: a start channel of -1.
    out.extend_from_slice(&(-1i16).to_le_bytes());
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
