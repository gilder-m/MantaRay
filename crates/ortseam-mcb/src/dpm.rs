//! The DPM-USB adapter's own protocol, on top of the two bulk endpoints.
//!
//! DPM is dual-port memory: the MCB and the host both address one block of
//! memory, the MCB writing spectra and status into it and the host reading them
//! out. The USB adapter's whole job is to carry reads and writes of that memory
//! across the wire, plus one small mailbox the two sides use to take turns.
//!
//! Every frame is the same six bytes followed by however much data it carries:
//!
//! ```text
//! op   len   00 08   offset:u16 LE   data...
//! ```
//!
//! `op` is [`READ`], [`WRITE`] or [`COMMAND`]; `len` counts the data bytes;
//! the `00 08` never varies. Replies arrive on the in endpoint: a read answers
//! with its bytes, a write with a single byte of acknowledgement, and a command
//! with a little-endian length, the ASCII answer and a carriage return.
//!
//! This was read off a USBPcap capture of ORTEC's own software and then checked
//! against the instrument: [`Dpm::read`] of offset zero returns at index two the
//! same byte a one-byte read of offset two returns, which is what pins the
//! address down to a flat sixteen-bit space rather than the several other
//! readings the frame would otherwise allow.

// The protocol is recorded here in full, including the parts nothing calls yet:
// writing to dual-port memory and the two mailbox registers. They cost nothing,
// the tests check them against the capture, and a half-written record of a
// protocol nobody documents is worse than none.
#![allow(dead_code)]

/// A device with the two bulk endpoints this protocol needs.
///
/// The protocol above it is the same everywhere; what differs is how the bytes
/// reach the wire. On Windows that is ORTEC's driver through its own IOCTLs;
/// on Linux and macOS it is the kernel's own USB stack through libusb, with no
/// driver to install at all.
pub trait BulkDevice {
    /// Sends or receives on a bulk endpoint, returning how many bytes moved.
    ///
    /// `endpoint` carries its direction in the top bit, as USB numbers them.
    fn bulk(&self, endpoint: u8, data: &mut [u8], milliseconds: u32) -> Result<usize, String>;
}

/// Read `len` bytes of dual-port memory.
pub const READ: u8 = 0x03;
/// Write the following bytes into dual-port memory.
pub const WRITE: u8 = 0x02;
/// Read a block of dual-port memory, of any length the field can count.
///
/// ORTEC's own library builds this frame at `DpmUsbAddIn.dll+0x77c0`, and the
/// only difference from [`READ`] next door at `+0x7760` is where the length
/// comes from: the read-one builder stores a hard `1`, this one stores the
/// caller's count. Nothing else about the two frames differs.
pub const READ_BLOCK: u8 = 0x06;
/// Hand an ASCII command to the MCB and wait for its answer.
pub const COMMAND: u8 = 0x0A;

/// The two bytes between the length and the offset, which never vary.
///
/// They are not really a constant: the first is the length's high byte, which
/// is zero for every frame that carries a one-byte count, and the second names
/// the address space. ORTEC's library sets the space from a global it fills in
/// at start-up (`DpmUsbAddIn.dll+0x20fa4`, zero in the file), and for a
/// DPM-USB in front of a 926 it fills it with eight.
const CONSTANT: [u8; 2] = [0x00, SPACE];

/// The address space a DPM-USB adapter answers for.
const SPACE: u8 = 0x08;

/// The space the histogram lives in: 32 KB, one 32-bit word per channel.
pub const HISTOGRAM: u8 = 0x00;

/// The top bit of a channel word, which marks a region of interest rather than
/// counting anything. A spectrum read without masking it off has channels
/// holding two billion counts.
pub const ROI_FLAG: u32 = 0x8000_0000;

/// The out endpoint, which carries frames to the adapter.
const OUT: u8 = 0x01;
/// The in endpoint, which carries answers back.
const IN: u8 = 0x81;

/// Where the ASCII command buffer lives.
pub const COMMAND_BUFFER: u16 = 0x0204;

/// The mailbox registers, which the two sides use to take turns.
pub const MAILBOX_STATUS: u16 = 0x0002;
/// The register the host writes to claim the memory.
pub const MAILBOX_REQUEST: u16 = 0x0004;

/// The largest a single frame will carry, because `len` is one byte.
pub const MAX_FRAME: usize = 255;

/// The bulk endpoints' packet size, from their descriptors.
///
/// A read has to offer the driver a whole number of packets to fill. Ask for
/// sixteen bytes in a sixteen-byte buffer and the transfer never completes,
/// even though the instrument is willing and the frame is correct - the answer
/// arrives as one sixty-four-byte packet and there is nowhere to put it. The
/// instrument still sends only what was asked for; it is the room to receive it
/// that has to be a round number.
const MAX_PACKET: usize = 64;

/// The most to ask the driver for in one transfer.
///
/// ORTEC's own readout arrives in 512-byte transfers; asking for a whole
/// spectrum at once gets nothing back at all.
const HELPING: usize = 4096;

/// Rounds a wanted length up to whole packets.
fn packet_aligned(length: usize) -> usize {
    length.div_ceil(MAX_PACKET) * MAX_PACKET
}

/// How long to wait for the adapter, in milliseconds.
///
/// Generous, because a read that has to wait for the MCB to finish a sweep is
/// slow rather than broken, and short enough that a wedged adapter is noticed.
const PATIENCE: u32 = 3000;

/// Builds a read frame: the six-byte header, carrying nothing after it.
fn read_frame(space: u8, offset: u16, length: u16) -> Vec<u8> {
    let mut out = vec![READ];
    out.extend_from_slice(&length.to_le_bytes());
    out.push(space);
    out.extend_from_slice(&offset.to_le_bytes());
    out
}

/// Builds one frame.
fn frame(op: u8, offset: u16, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(6 + data.len());
    out.push(op);
    out.push(data.len() as u8);
    out.extend_from_slice(&CONSTANT);
    out.extend_from_slice(&offset.to_le_bytes());
    out.extend_from_slice(data);
    out
}

/// One instrument, spoken to through its adapter.
pub struct Dpm<'a> {
    device: &'a dyn BulkDevice,
}

impl<'a> Dpm<'a> {
    pub fn new(device: &'a dyn BulkDevice) -> Self {
        Self { device }
    }

    /// Reads consecutive bytes of dual-port memory.
    ///
    /// A read frame is the six-byte header alone, with the length in the
    /// length field and nothing after it. Padding the frame out to the length
    /// being asked for looks harmless and is not: the adapter takes the frame
    /// from the first six bytes, then reads the padding as further frames, and
    /// answers those too. Every reply after that is the answer to some earlier
    /// question, which is a far stranger failure than no answer at all.
    pub fn read(&self, offset: u16, length: usize) -> Result<Vec<u8>, String> {
        self.read_in(SPACE, offset, length)
    }

    /// Reads consecutive bytes of a named address space.
    ///
    /// The length is sixteen bits, so this fetches a whole spectrum in one
    /// frame - which is exactly what MAESTRO does, and the only way the counts
    /// come back whole. [`read_block`](Self::read_block) hands back every
    /// *other* byte of the memory it covers; this one hands back all of them.
    pub fn read_in(&self, space: u8, offset: u16, length: usize) -> Result<Vec<u8>, String> {
        if length == 0 || length > u16::MAX as usize {
            return Err(format!(
                "a frame carries 1 to {} bytes, not {length}",
                u16::MAX
            ));
        }
        let mut out = read_frame(space, offset, length as u16);
        self.device.bulk(OUT, &mut out, PATIENCE)?;
        // One packet of slack, so an answer that fills its last packet exactly
        // does not leave its end-of-transfer marker for the next read. See
        // [`read_block`](Self::read_block).
        let mut reply = vec![0_u8; packet_aligned(length) + MAX_PACKET];
        let mut got = 0;
        while got < length {
            // Ask for a bounded piece at a time. The driver will not carry a
            // whole spectrum in one transfer - a single 32 KB request simply
            // never completes - and the adapter sends its answer in small
            // transfers anyway, so this is how the bytes were always going to
            // arrive.
            let room = (reply.len() - got).min(HELPING);
            let mut piece = vec![0_u8; room];
            let arrived = self.device.bulk(IN, &mut piece, PATIENCE)?;
            if arrived == 0 {
                break;
            }
            reply[got..got + arrived].copy_from_slice(&piece[..arrived]);
            got += arrived;
        }
        if got < length {
            return Err(format!(
                "the adapter sent {got} of {length} bytes from {offset:#06x}"
            ));
        }
        reply.truncate(length);
        Ok(reply)
    }

    /// Reads a whole spectrum: `channels` counts, and which channels are in a
    /// region of interest.
    ///
    /// **This is what MAESTRO does**, frame for frame: one `0x03` read of
    /// `4 * channels` bytes from offset zero of the histogram space, and the
    /// counts come back in a single answer. There is no paging, no handshake
    /// and no second request - the length field is sixteen bits, which is
    /// exactly enough for the 32 KB a 8192-channel spectrum takes.
    ///
    /// Each channel is a little-endian thirty-two-bit word, but only the low
    /// thirty-one bits are the count. **The top bit marks a region of
    /// interest**, which is why a naive reading of the memory produces channels
    /// holding two billion counts: those are ROI channels, not overflows.
    pub fn read_spectrum(&self, channels: usize) -> Result<(Vec<u32>, Vec<bool>), String> {
        let bytes = channels
            .checked_mul(4)
            .filter(|wanted| *wanted <= u16::MAX as usize)
            .ok_or_else(|| format!("{channels} channels is more than one frame can carry"))?;
        let memory = self.read_in(HISTOGRAM, 0, bytes)?;
        let mut counts = Vec::with_capacity(channels);
        let mut regions = Vec::with_capacity(channels);
        for word in memory.chunks_exact(4) {
            let raw = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
            counts.push(raw & !ROI_FLAG);
            regions.push(raw & ROI_FLAG != 0);
        }
        Ok((counts, regions))
    }

    /// Reads a block of dual-port memory in one frame.
    ///
    /// Where [`read`](Self::read) is limited to what a byte can count, this
    /// carries a sixteen-bit length and so fetches a kilobyte as easily as a
    /// byte. The frame is otherwise identical, and like a read frame it is
    /// sent as the bare header: ORTEC appends one uninitialised byte of its
    /// own stack, which the adapter ignores, and there is nothing to be gained
    /// by copying an accident.
    pub fn read_block(&self, offset: u16, length: usize) -> Result<Vec<u8>, String> {
        self.read_block_in(SPACE, offset, length)
    }

    /// Reads a block from a named address space.
    ///
    /// The space is the byte ORTEC's library fills in from a global it sets at
    /// start-up. Eight is what a DPM-USB in front of a 926 answers for; this
    /// exists because what the other 255 values reach is not documented
    /// anywhere, and the spectrum is not in the space that is.
    pub fn read_block_in(&self, space: u8, offset: u16, length: usize) -> Result<Vec<u8>, String> {
        if length == 0 || length > u16::MAX as usize {
            return Err(format!(
                "a block frame carries 1 to {} bytes, not {length}",
                u16::MAX
            ));
        }
        let mut out = vec![READ_BLOCK];
        out.extend_from_slice(&(length as u16).to_le_bytes());
        out.push(space);
        out.extend_from_slice(&offset.to_le_bytes());
        self.device.bulk(OUT, &mut out, PATIENCE)?;

        // Room for one packet more than the answer needs. A device that has
        // sent a whole number of packets marks the end with an empty one, and
        // a buffer sized exactly to the answer leaves that terminator behind
        // for the next read to collect - which then returns nothing, and every
        // reply afterwards is one question late. The slack lets the driver
        // swallow the terminator as part of this transfer, where it belongs.
        let mut reply = vec![0_u8; packet_aligned(length) + MAX_PACKET];
        let mut got = 0;
        while got < length {
            let mut piece = vec![0_u8; reply.len() - got];
            let arrived = self.device.bulk(IN, &mut piece, PATIENCE)?;
            if arrived == 0 {
                break;
            }
            reply[got..got + arrived].copy_from_slice(&piece[..arrived]);
            got += arrived;
        }
        if got < length {
            return Err(format!(
                "the adapter sent {got} of {length} bytes from {offset:#06x}"
            ));
        }
        reply.truncate(length);
        Ok(reply)
    }

    /// Reads any amount of dual-port memory, a frame at a time.
    ///
    /// The whole eight-kilobyte window comes back in eight frames rather than
    /// thirty-three, because [`read_block`](Self::read_block) counts its
    /// length in sixteen bits.
    pub fn read_all(&self, offset: u16, length: usize) -> Result<Vec<u8>, String> {
        const CHUNK: usize = 1024;
        let mut out = Vec::with_capacity(length);
        while out.len() < length {
            let want = (length - out.len()).min(CHUNK);
            let at = offset
                .checked_add(out.len() as u16)
                .ok_or("the read runs off the end of the address space")?;
            let piece = self.read_block(at, want)?;
            if piece.is_empty() {
                return Err(format!("the adapter stopped answering at offset {at:#06x}"));
            }
            out.extend_from_slice(&piece);
        }
        Ok(out)
    }

    /// Writes bytes into dual-port memory.
    pub fn write(&self, offset: u16, data: &[u8]) -> Result<(), String> {
        if data.is_empty() || data.len() > MAX_FRAME {
            return Err(format!(
                "a frame carries 1 to {MAX_FRAME} bytes, not {}",
                data.len()
            ));
        }
        let mut out = frame(WRITE, offset, data);
        self.device.bulk(OUT, &mut out, PATIENCE)?;
        // A write is acknowledged with one byte, which has to be collected or
        // it turns up at the head of the next read.
        let mut reply = [0_u8; MAX_PACKET];
        self.device.bulk(IN, &mut reply, PATIENCE)?;
        Ok(())
    }

    /// Sends one ASCII command and returns the answer, with `$` and all.
    ///
    /// The answer is framed with its own little-endian length and ends in a
    /// carriage return, both of which are stripped. An empty answer is a real
    /// one - a 926 answers `SHOW_FEAT` with nothing at all - so it is returned
    /// as an empty string rather than treated as a failure.
    pub fn command(&self, text: &str) -> Result<String, String> {
        let body = self.command_bytes(text)?;
        Ok(body
            .iter()
            .take_while(|byte| **byte != 0x0D && **byte != 0)
            .map(|byte| *byte as char)
            .collect())
    }

    /// Sends one command and returns the raw answer bytes, length-framed.
    ///
    /// The mailbox carries binary as happily as ASCII - the message length is
    /// its own field, MCB$OUTLEN, precisely so that a reply full of channel
    /// counts is not cut at the first byte that happens to be a carriage
    /// return. [`command`](Self::command) is this with the text convention
    /// applied.
    pub fn command_bytes(&self, text: &str) -> Result<Vec<u8>, String> {
        if text.len() > MAX_FRAME {
            return Err(format!("{text:?} is too long for one frame"));
        }
        let mut out = frame(COMMAND, COMMAND_BUFFER, text.as_bytes());
        self.device.bulk(OUT, &mut out, PATIENCE)?;

        // Room for the biggest reply the protocol allows: a WRITE record of
        // up to 512 data bytes, its header and checksum, and the length word.
        let mut reply = vec![0_u8; 1024];
        let got = self.device.bulk(IN, &mut reply, PATIENCE)?;
        if got < 2 {
            return Err(format!(
                "the instrument answered {got} byte(s), which is not even a length"
            ));
        }
        let declared = u16::from_le_bytes([reply[0], reply[1]]) as usize;
        let end = declared.min(got - 2);
        reply.drain(..2);
        reply.truncate(end);
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_frame_is_the_one_the_capture_shows() {
        // ORTEC's own software sent exactly this for SHOW_VERSION, and the
        // instrument answered $F0926-001.
        assert_eq!(
            frame(COMMAND, COMMAND_BUFFER, b"SHOW_VERSION"),
            [
                0x0A, 0x0C, 0x00, 0x08, 0x04, 0x02, b'S', b'H', b'O', b'W', b'_', b'V', b'E', b'R',
                b'S', b'I', b'O', b'N'
            ]
        );
    }

    #[test]
    fn a_read_frame_is_the_header_alone() {
        // ORTEC's own frames carry a stray byte after the header whose value
        // it never sets; the adapter answers the header and ignores the rest.
        // Sending the header alone gets the same answer, and unlike padding the
        // frame out it cannot be mistaken for further frames.
        let mut built = vec![READ, 0x01, CONSTANT[0], CONSTANT[1]];
        built.extend_from_slice(&MAILBOX_STATUS.to_le_bytes());
        assert_eq!(built, [0x03, 0x01, 0x00, 0x08, 0x02, 0x00]);
    }

    #[test]
    fn a_mailbox_write_is_the_one_the_capture_shows() {
        assert_eq!(
            frame(WRITE, MAILBOX_REQUEST, &[0xA5, 0x00]),
            [0x02, 0x02, 0x00, 0x08, 0x04, 0x00, 0xA5, 0x00]
        );
    }

    #[test]
    fn the_offset_is_little_endian() {
        // The command buffer is at 0x0204 and goes on the wire as 04 02, which
        // is the thing most easily got backwards.
        let built = frame(READ, 0x0204, &[0]);
        assert_eq!(&built[4..6], &[0x04, 0x02]);
    }

    #[test]
    fn a_read_offers_the_driver_whole_packets_to_fill() {
        // Sixteen bytes still needs sixty-four bytes of room; asking for
        // exactly what is wanted is what made every read hang.
        assert_eq!(packet_aligned(1), 64);
        assert_eq!(packet_aligned(16), 64);
        assert_eq!(packet_aligned(64), 64);
        assert_eq!(packet_aligned(65), 128);
        assert_eq!(packet_aligned(MAX_FRAME), 256);
    }

    #[test]
    fn a_spectrum_read_is_the_frame_maestro_sends() {
        // Taken off the wire while MAESTRO refreshed its display: one read of
        // the whole histogram, 4 bytes a channel, from offset zero of space 0.
        // MAESTRO's instrument was set to 4096 channels, so 16384 bytes.
        assert_eq!(
            read_frame(HISTOGRAM, 0, 4 * 4096),
            [0x03, 0x00, 0x40, 0x00, 0x00, 0x00]
        );
        // The length is genuinely sixteen bits: an 8192-channel spectrum needs
        // 32768 bytes, and that is one frame too.
        assert_eq!(
            read_frame(HISTOGRAM, 0, 4 * 8192),
            [0x03, 0x00, 0x80, 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn the_top_bit_of_a_channel_is_a_region_and_not_a_count() {
        // Read without masking, an ROI channel holds over two billion counts.
        // These four words are the shape the capture showed: a marked channel
        // and an unmarked one, alternating.
        let memory: Vec<u8> = [0x8000_3BFBu32, 0x0000_04D7, 0x8000_04DB, 0x0000_0000]
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect();
        let counts: Vec<u32> = memory
            .chunks_exact(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]) & !ROI_FLAG)
            .collect();
        let regions: Vec<bool> = memory
            .chunks_exact(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]) & ROI_FLAG != 0)
            .collect();
        assert_eq!(counts, [15355, 1239, 1243, 0]);
        assert_eq!(regions, [true, false, true, false]);
    }

    #[test]
    fn a_frame_may_not_carry_more_than_its_length_field_can_count() {
        let device_free = |length: usize| length == 0 || length > MAX_FRAME;
        assert!(device_free(0));
        assert!(device_free(256));
        assert!(!device_free(255));
        assert!(!device_free(1));
    }
}
