//! The Model 927's own protocol, on top of its four bulk endpoints.
//!
//! A 927 is two multichannel buffers in one NIM module behind one USB device,
//! and it shares nothing with the 926's dual-port memory in [`crate::dpm`]: no
//! ASCII goes down the wire at all. There are two channels instead.
//!
//! The **slow channel** (out `0x01`, in `0x81`) reaches the USB controller. A
//! command is a four-byte header and sometimes a payload, and every answer
//! starts with a two-byte status pair:
//!
//! ```text
//! op   00   n   00   payload...          ->   macro  micro  rest...
//! ```
//!
//! `n` is the length of the answer. The FPGA that does the counting has no
//! configuration of its own after a power-up; the slow channel is how one is
//! loaded into it, and how to ask whether one is there.
//!
//! The **fast channel** (out `0x08`, in `0x82`) reaches the FPGA once it is
//! configured. A command is little-endian 32-bit words - an opcode and address,
//! a word count, then data for a write - and it reads and writes the registers
//! and the spectrum memory of both inputs. Input 2 lives `0x20` above input 1
//! in the registers and `0x8000` above it in memory.
//!
//! **Where this comes from.** The framing, the opcodes, the register map and
//! the sequences are ORCA's open-source driver (`ORMCA927Model.m`, UNC/CENPA),
//! which drove real 927s. Part of it was then measured on a 927 here, on
//! 2026-11-13. Before a load, `06 00 04 00` answered the four bytes
//! `04 80 43 DF` and `12 00 06 00` the six bytes `00 00 00 00 00 02`, so the
//! header's third byte is the answer's length. The load sequence ran in
//! ORCA's order with `00 00` after every packet, in under a second. After it
//! a register read was eight bytes out and exactly four back, little-endian,
//! with input 2 at `0x20` above input 1. A register write and a clear were
//! each answered with a zero-length packet; a 16384-channel spectrum came
//! back as 65,536 bytes of plain little-endian counts with one zero-length
//! packet after them. Start, stop, clear and the clocks behaved as ORCA has
//! them, in twenty-millisecond ticks. And one thing ORCA does not say: a
//! freshly loaded FPGA counts nothing until its discriminators are set.
//! `docs/ortec-hardware.md` keeps the list, and what is still unmeasured.

// Recorded in full, as `dpm.rs` is, and compiled on every platform so that the
// tests run where the hardware is not.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::dpm::BulkDevice;

/// Commands to the USB controller.
pub const SLOW_OUT: u8 = 0x01;
/// Its answers.
pub const SLOW_IN: u8 = 0x81;
/// Commands to the FPGA.
pub const FAST_OUT: u8 = 0x08;
/// Its answers: registers and spectra.
pub const FAST_IN: u8 = 0x82;

/// The slow channel's packet size, which is also the most an answer can be.
const SLOW_PACKET: usize = 64;
/// The fast in endpoint's packet size.
const FAST_PACKET: usize = 512;

/// How long a write may take, in milliseconds. ORCA's figure.
const WRITE_PATIENCE: u32 = 5000;
/// How long to wait for an answer, in milliseconds.
///
/// ORCA waits one second. This waits three, as the 926 path does, because a
/// read that gives up early is a cancelled transfer, and those are what put a
/// reply stream out of step.
const READ_PATIENCE: u32 = 3000;

/// How long an ordinary operation waits for another process to let go of the
/// box.
pub const PATIENCE: Duration = Duration::from_secs(2);
/// How long opening waits. Longer, because the process serving the other
/// input may be in the middle of loading the FPGA, which is nearly two
/// thousand round trips under one claim. Measured at under a second.
pub const OPEN_PATIENCE: Duration = Duration::from_secs(10);

/// Slow-channel opcodes, named as ORCA names them.
mod op {
    pub const INIT_FPGA: u8 = 0x04;
    pub const FPGA_DATA: u8 = 0x05;
    pub const TEST_FPGA: u8 = 0x06;
    pub const DISCRIMINATOR: u8 = 0x07;
    pub const FIRMWARE_LOOP_OFF: u8 = 0x08;
    pub const FIRMWARE_LOOP_ON: u8 = 0x09;
    pub const RESET_MDA: u8 = 0x10;
    pub const RESET_FPGA: u8 = 0x11;
    pub const VERSION: u8 = 0x12;
}

/// How many bytes of the FPGA image ride in one packet. ORCA's figure, and
/// the only size known to work on a 927.
const FPGA_CHUNK: usize = 50;

/// Fast-channel command words.
const READ_MEMORY: u32 = 0x0000_0000;
const READ_REGISTER: u32 = 0x0004_0000;
const WRITE_REGISTER: u32 = 0x0104_0000;
/// The opcode ORCA's `clearSpectrum` writes zeroes with.
const CLEAR_MEMORY: u32 = 0x0400_0000;
/// How many words ORCA's `clearSpectrum` writes: one short of the sixteen
/// thousand three hundred and eighty-four a full spectrum has.
const CLEAR_WORDS: u32 = 0x3FFF;

/// The most channels an input has.
pub const MAX_CHANNELS: usize = 16_384;

/// The highest a discriminator goes: full scale.
pub const DISCRIMINATOR_MAX: u16 = 0x3FFF;

/// The lower discriminator given to an FPGA this program has just loaded.
///
/// A fresh load leaves the discriminators where nothing passes: on the bench
/// a ten-second count gave zero counts on both inputs until they were set,
/// and with these two values both inputs counted. One percent of full scale
/// keeps the noise at the bottom out. **The instrument cannot be asked what
/// its discriminators are**, so these are written once, after a load done
/// here, and never to an FPGA found already configured - whoever configured
/// it may have chosen others.
pub const DEFAULT_LOWER_DISCRIMINATOR: u16 = 164;
/// The upper discriminator to go with it: all the way open.
pub const DEFAULT_UPPER_DISCRIMINATOR: u16 = DISCRIMINATOR_MAX;

/// The start bit of the control register, and the acquiring bit of the
/// acquisition status.
const START: u32 = 1;

/// One of the two inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    One,
    Two,
}

impl Input {
    /// The input a number printed on the front panel names.
    pub fn from_number(number: u8) -> Option<Self> {
        match number {
            1 => Some(Self::One),
            2 => Some(Self::Two),
            _ => None,
        }
    }

    pub fn number(self) -> u8 {
        match self {
            Self::One => 1,
            Self::Two => 2,
        }
    }

    fn register_base(self) -> u32 {
        match self {
            Self::One => 0x00,
            Self::Two => 0x20,
        }
    }

    fn memory_base(self) -> u32 {
        match self {
            Self::One => 0x0000,
            Self::Two => 0x8000,
        }
    }
}

/// The registers of one input, by their offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Register {
    /// Bit 0 start, bit 4 trigger enable, bit 5 upper discriminator off,
    /// bits 28 and 29 the gate.
    Control = 0x00,
    Aux = 0x01,
    /// 0 for 16384 channels, each step up halving it, 6 for 256.
    ConversionGain = 0x03,
    ZdtMode = 0x04,
    /// Which presets are armed: see [`Preset`].
    PresetControl = 0x05,
    LivePreset = 0x06,
    RealPreset = 0x07,
    RoiPeakPreset = 0x08,
    RoiIntegralPreset = 0x09,
    /// Bit 0 acquiring, bit 1 trigger armed.
    AcquisitionStatus = 0x0A,
    LiveTime = 0x0B,
    RealTime = 0x0C,
    Aux0Counter = 0x0D,
    Aux1Counter = 0x0E,
    ControlMirror = 0x0F,
    Version = 0x11,
    /// Written 1 then 0 to stop.
    StopAcquisition = 0x1F,
}

/// One of the four presets that can stop an acquisition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    /// Real time, in twenty-millisecond ticks.
    Real,
    /// Live time, in twenty-millisecond ticks.
    Live,
    /// Counts in the tallest channel of a region.
    RoiPeak,
    /// Counts in all of a region.
    RoiIntegral,
}

impl Preset {
    fn register(self) -> Register {
        match self {
            Self::Real => Register::RealPreset,
            Self::Live => Register::LivePreset,
            Self::RoiPeak => Register::RoiPeakPreset,
            Self::RoiIntegral => Register::RoiIntegralPreset,
        }
    }

    /// The bit of the preset control register that arms this one.
    fn enable(self) -> u32 {
        match self {
            Self::Real => 1 << 2,
            Self::Live => 1 << 3,
            Self::RoiIntegral => 1 << 4,
            Self::RoiPeak => 1 << 5,
        }
    }
}

/// The two bytes every slow-channel answer starts with.
///
/// The numbers are the macro and micro error codes of ORTEC's ASCII dialect,
/// sent in binary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub macro_code: u8,
    pub micro_code: u8,
}

impl Status {
    pub const OK: Self = Self::new(0x00, 0x00);
    /// What a 927 says about its FPGA after a power-up. Measured.
    pub const NEEDS_CONFIGURATION: Self = Self::new(0x04, 0x80);
    pub const INVALID_COMMAND: Self = Self::new(0x81, 0x84);

    pub const fn new(macro_code: u8, micro_code: u8) -> Self {
        Self {
            macro_code,
            micro_code,
        }
    }

    fn of(reply: &[u8]) -> Self {
        Self::new(reply[0], reply[1])
    }

    pub fn is_ok(self) -> bool {
        self == Self::OK
    }

    /// What the pair means, tested in the order ORCA tests it.
    pub fn meaning(self) -> &'static str {
        if self.is_ok() {
            "success"
        } else if self.micro_code == 2 {
            "a preset is already exceeded"
        } else if self == Self::NEEDS_CONFIGURATION {
            "the FPGA failed or needs configuration"
        } else if self == Self::INVALID_COMMAND {
            "invalid command"
        } else {
            "unspecified error"
        }
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:02X} {:02X} ({})",
            self.macro_code,
            self.micro_code,
            self.meaning()
        )
    }
}

/// Why an exchange with a 927 did not produce an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The box answered a slow-channel command with something other than
    /// `00 00`.
    Status { op: u8, status: Status },
    /// A transfer moved the wrong number of bytes.
    Size {
        what: &'static str,
        wanted: usize,
        got: usize,
    },
    /// The request was refused here, before anything was sent.
    Refused(String),
    /// The FPGA load stopped after `sent` bytes of the image had gone.
    Loading { sent: usize, fault: Box<Error> },
    /// The wire itself failed: a timeout, an unplugged cable.
    Wire(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Status { op, status } => {
                write!(f, "the 927 answered command {op:#04x} with status {status}")
            }
            Self::Size { what, wanted, got } => {
                write!(f, "{what} is {wanted} bytes, and {got} moved")
            }
            Self::Refused(why) => f.write_str(why),
            Self::Loading { sent, fault } => write!(
                f,
                "the FPGA load stopped after {sent} bytes of the image: {fault}"
            ),
            Self::Wire(why) => f.write_str(why),
        }
    }
}

impl From<Error> for String {
    fn from(error: Error) -> Self {
        error.to_string()
    }
}

/// Whether the FPGA holds a configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fpga {
    Configured,
    /// The state after every power-up: nothing on the fast channel answers
    /// until an image is loaded.
    NeedsConfiguration,
}

/// A 927 that is shared between processes, one operation at a time.
///
/// The application runs one bridge process per detector, a 927 is two
/// detectors, and a USB interface has one claimant. So the claim is taken for
/// one operation and given back, and the process serving the other input gets
/// its turn in between.
pub trait Shared {
    /// Runs one operation with the box held, waiting up to `patience` for
    /// whoever else holds it.
    ///
    /// Everything that must not be interleaved with the other process goes in
    /// one call: a command and its answer at the least, since an answer read
    /// by the wrong process is lost to the right one.
    fn hold<T>(
        &self,
        patience: Duration,
        operation: impl FnOnce(&dyn BulkDevice) -> Result<T, String>,
    ) -> Result<T, String>;
}

/// Little-endian words, as the fast channel carries them.
fn words(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

/// A 927, spoken to through its endpoints.
pub struct Mcb927<'a> {
    device: &'a dyn BulkDevice,
}

impl<'a> Mcb927<'a> {
    pub fn new(device: &'a dyn BulkDevice) -> Self {
        Self { device }
    }

    /// Sends a command and insists every byte of it went.
    fn send(&self, endpoint: u8, bytes: &[u8]) -> Result<(), Error> {
        let mut frame = bytes.to_vec();
        let sent = self
            .device
            .bulk(endpoint, &mut frame, WRITE_PATIENCE)
            .map_err(Error::Wire)?;
        if sent != bytes.len() {
            return Err(Error::Size {
                what: "the command",
                wanted: bytes.len(),
                got: sent,
            });
        }
        Ok(())
    }

    /// One slow-channel exchange: the answer, status pair and all, unjudged.
    fn slow(&self, op: u8, answer: u8, payload: &[u8]) -> Result<Vec<u8>, Error> {
        let mut packet = vec![op, 0x00, answer, 0x00];
        packet.extend_from_slice(payload);
        self.send(SLOW_OUT, &packet)?;
        let mut reply = [0_u8; SLOW_PACKET];
        let got = self
            .device
            .bulk(SLOW_IN, &mut reply, READ_PATIENCE)
            .map_err(Error::Wire)?;
        if got < 2 {
            return Err(Error::Size {
                what: "a status pair",
                wanted: 2,
                got,
            });
        }
        Ok(reply[..got].to_vec())
    }

    /// One slow-channel exchange that must answer `00 00`.
    fn slow_ok(&self, op: u8, answer: u8, payload: &[u8]) -> Result<Vec<u8>, Error> {
        let reply = self.slow(op, answer, payload)?;
        let status = Status::of(&reply);
        if !status.is_ok() {
            return Err(Error::Status { op, status });
        }
        Ok(reply)
    }

    /// Asks whether the FPGA holds a configuration.
    ///
    /// Read-only as far as anyone knows: ORCA's name for it is a test, and
    /// the related digiBASE drivers send it first on every connection.
    pub fn fpga_status(&self) -> Result<Fpga, Error> {
        let reply = self.slow(op::TEST_FPGA, 4, &[])?;
        match Status::of(&reply) {
            Status::OK => Ok(Fpga::Configured),
            Status::NEEDS_CONFIGURATION => Ok(Fpga::NeedsConfiguration),
            status => Err(Error::Status {
                op: op::TEST_FPGA,
                status,
            }),
        }
    }

    /// The USB controller's firmware version. Answers whether or not the
    /// FPGA is configured.
    pub fn firmware_version(&self) -> Result<u32, Error> {
        let reply = self.slow_ok(op::VERSION, 6, &[])?;
        let Some(version) = reply.get(2..6) else {
            return Err(Error::Size {
                what: "a version answer",
                wanted: 6,
                got: reply.len(),
            });
        };
        Ok(u32::from_be_bytes([
            version[0], version[1], version[2], version[3],
        ]))
    }

    /// Loads a configuration into the FPGA: ORCA's `startFPGA`, step for
    /// step, stopping at the first answer that is not `00 00`.
    ///
    /// **This drops whatever configuration the FPGA held**, and with it both
    /// inputs' settings and anything either was counting. The image is
    /// ORTEC's `M927.rbf`, which is theirs and is never part of this project;
    /// the caller reads it from wherever the operator keeps it.
    pub fn load_fpga(&self, image: &[u8]) -> Result<(), Error> {
        if image.is_empty() {
            // ORCA would send the first two commands and then nothing, which
            // leaves an FPGA that had a configuration without one.
            return Err(Error::Refused(
                "the FPGA image is empty, so nothing was sent".into(),
            ));
        }
        self.slow_ok(op::FIRMWARE_LOOP_OFF, 2, &[])?;
        self.slow_ok(op::INIT_FPGA, 2, &[])?;
        for (index, chunk) in image.chunks(FPGA_CHUNK).enumerate() {
            self.slow_ok(op::FPGA_DATA, 2, chunk)
                .map_err(|fault| Error::Loading {
                    sent: index * FPGA_CHUNK,
                    fault: Box::new(fault),
                })?;
        }
        self.slow_ok(op::TEST_FPGA, 4, &[])?;
        self.slow_ok(op::RESET_FPGA, 2, &[])?;
        self.slow_ok(op::FIRMWARE_LOOP_ON, 2, &[])?;
        Ok(())
    }

    /// Sets an input's two discriminators, lower then upper as ORCA does.
    ///
    /// Each is a fraction of full scale, 0 to [`DISCRIMINATOR_MAX`]. They
    /// are write-only: nothing reads them back.
    pub fn set_discriminators(&self, input: Input, lower: u16, upper: u16) -> Result<(), Error> {
        if lower > DISCRIMINATOR_MAX || upper > DISCRIMINATOR_MAX {
            return Err(Error::Refused(format!(
                "a discriminator runs from 0 to {DISCRIMINATOR_MAX}, not to {}",
                lower.max(upper)
            )));
        }
        let (lower_index, upper_index) = match input {
            Input::One => (1, 0),
            Input::Two => (3, 2),
        };
        for (index, value) in [(lower_index, lower), (upper_index, upper)] {
            let [high, low] = value.to_be_bytes();
            self.slow_ok(
                op::DISCRIMINATOR,
                12,
                &[0x00, 0x00, 0x00, index, 0x00, 0x00, high, low],
            )?;
        }
        Ok(())
    }

    /// Loads the FPGA and opens both inputs' discriminators to the defaults,
    /// which is what it takes for a 927 just powered up to count at all.
    pub fn configure(&self, image: &[u8]) -> Result<(), Error> {
        self.load_fpga(image)?;
        for input in [Input::One, Input::Two] {
            self.set_discriminators(
                input,
                DEFAULT_LOWER_DISCRIMINATOR,
                DEFAULT_UPPER_DISCRIMINATOR,
            )?;
        }
        Ok(())
    }

    /// Reads one register of one input.
    pub fn read_register(&self, input: Input, register: Register) -> Result<u32, Error> {
        let address = input.register_base() + register as u32;
        self.send(FAST_OUT, &words(&[READ_REGISTER | address, 1]))?;
        let mut reply = [0_u8; FAST_PACKET];
        let got = self
            .device
            .bulk(FAST_IN, &mut reply, READ_PATIENCE)
            .map_err(Error::Wire)?;
        // Exactly four, not at least four: ORCA asks for four and no more,
        // and anything else here is an answer to some other question.
        if got != 4 {
            return Err(Error::Size {
                what: "a register",
                wanted: 4,
                got,
            });
        }
        Ok(u32::from_le_bytes([reply[0], reply[1], reply[2], reply[3]]))
    }

    /// Writes one register of one input.
    pub fn write_register(
        &self,
        input: Input,
        register: Register,
        value: u32,
    ) -> Result<(), Error> {
        let address = input.register_base() + register as u32;
        self.send(FAST_OUT, &words(&[WRITE_REGISTER | address, 1, value]))?;
        self.empty_answer()
    }

    /// Reads the answer to a write, which is a packet with nothing in it.
    ///
    /// "All commands have response. Read it" is all ORCA says. Measured, the
    /// response to a register write or a clear is one zero-length packet;
    /// left unread it would stand where the next answer should, and anything
    /// longer here is some other question's answer.
    fn empty_answer(&self) -> Result<(), Error> {
        let mut reply = [0_u8; FAST_PACKET];
        let got = self
            .device
            .bulk(FAST_IN, &mut reply, READ_PATIENCE)
            .map_err(Error::Wire)?;
        if got != 0 {
            return Err(Error::Size {
                what: "the answer to a write",
                wanted: 0,
                got,
            });
        }
        Ok(())
    }

    /// How many channels the input's conversion gain is set to.
    pub fn channels(&self, input: Input) -> Result<usize, Error> {
        let gain = self.read_register(input, Register::ConversionGain)?;
        if gain > 6 {
            return Err(Error::Refused(format!(
                "the conversion gain register of input {} holds {gain}, which is not a gain",
                input.number()
            )));
        }
        Ok(MAX_CHANNELS >> gain)
    }

    /// Reads `count` channels of an input's spectrum, from channel `start`.
    ///
    /// The words are plain counts: on the bench nothing but counts was seen
    /// in them. ORCA only ever reads from channel zero; a read from further
    /// in is the same frame with the address moved, which the memory map
    /// implies and nothing has tested.
    pub fn read_spectrum(
        &self,
        input: Input,
        start: usize,
        count: usize,
    ) -> Result<Vec<u32>, Error> {
        if count == 0 || start.saturating_add(count) > MAX_CHANNELS {
            return Err(Error::Refused(format!(
                "{count} channels from channel {start} is not inside the {MAX_CHANNELS} an input has"
            )));
        }
        let address = input.memory_base() + start as u32;
        self.send(FAST_OUT, &words(&[READ_MEMORY | address, count as u32]))?;
        let wanted = count * 4;
        // One packet more than the answer needs, as ORCA asks for one byte
        // more. Every spectrum is a whole number of packets, and the box
        // marks its end with a zero-length packet: measured, 65,536 bytes
        // and then one of those. With room to spare the transfer ends on the
        // marker and takes it along; sized exactly, it would end when full
        // and leave the marker to answer the next register read with nothing.
        let mut reply = vec![0_u8; wanted.div_ceil(FAST_PACKET) * FAST_PACKET + FAST_PACKET];
        let mut got = 0;
        while got < wanted {
            let arrived = self
                .device
                .bulk(FAST_IN, &mut reply[got..], READ_PATIENCE)
                .map_err(Error::Wire)?;
            if arrived == 0 {
                break;
            }
            got += arrived;
        }
        if got != wanted {
            return Err(Error::Size {
                what: "the spectrum",
                wanted,
                got,
            });
        }
        Ok(reply[..wanted]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|word| u32::from_le_bytes(*word))
            .collect())
    }

    /// Whether the input is acquiring.
    pub fn is_counting(&self, input: Input) -> Result<bool, Error> {
        Ok(self.read_register(input, Register::AcquisitionStatus)? & START != 0)
    }

    /// The live clock, in twenty-millisecond ticks.
    pub fn live_ticks(&self, input: Input) -> Result<u32, Error> {
        self.read_register(input, Register::LiveTime)
    }

    /// The real clock, in twenty-millisecond ticks.
    pub fn real_ticks(&self, input: Input) -> Result<u32, Error> {
        self.read_register(input, Register::RealTime)
    }

    /// The FPGA's own version register.
    ///
    /// Read where ORCA reads it, at input 2's offset. On the bench both
    /// inputs' registers held the same 2.
    pub fn fpga_version(&self) -> Result<u32, Error> {
        self.read_register(Input::Two, Register::Version)
    }

    /// Starts an input, changing nothing else about it.
    ///
    /// ORCA's `startAcquisition` first writes every setting it keeps - gain,
    /// presets, discriminators, the clocks - and this deliberately does not:
    /// starting a count must not change how the instrument is set up behind
    /// the operator's back. What is kept is what makes it start: the
    /// reset-MDA command, then the control register written with the start
    /// bit clear and again with it set, which are ORCA's last two writes with
    /// the register's other bits as they were found.
    ///
    /// An input that is already acquiring is left alone, as ORCA leaves it.
    pub fn start(&self, input: Input) -> Result<(), Error> {
        if self.is_counting(input)? {
            return Ok(());
        }
        self.slow_ok(op::RESET_MDA, 2, &[])?;
        let control = self.read_register(input, Register::Control)?;
        // The clear write is what makes the next one an edge. A count that a
        // preset ended may leave the start bit standing, and setting a bit
        // that is already set would start nothing.
        self.write_register(input, Register::Control, control & !START)?;
        self.write_register(input, Register::Control, control | START)
    }

    /// Stops an input: ORCA's `stopAcquisition`, with the control register
    /// read from the instrument rather than remembered.
    pub fn stop(&self, input: Input) -> Result<(), Error> {
        if !self.is_counting(input)? {
            return Ok(());
        }
        self.write_register(input, Register::StopAcquisition, 1)?;
        self.write_register(input, Register::StopAcquisition, 0)?;
        let control = self.read_register(input, Register::Control)?;
        self.write_register(input, Register::Control, control & !START)
    }

    /// Zeroes an input's spectrum: ORCA's `clearSpectrum`, byte for byte.
    ///
    /// That frame writes 16383 words, so as written the last channel of a
    /// 16384-channel spectrum is not cleared. The frame as ORCA has it is the
    /// one the bench saw work; whether the box takes the full count is not
    /// known, so it is kept.
    pub fn clear(&self, input: Input) -> Result<(), Error> {
        let mut frame = vec![CLEAR_MEMORY | input.memory_base(), CLEAR_WORDS];
        frame.resize(2 + CLEAR_WORDS as usize, 0);
        self.send(FAST_OUT, &words(&frame))?;
        self.empty_answer()
    }

    /// Zeroes an input's two clocks.
    ///
    /// ORCA writes both clock registers on every start, with a value that is
    /// zero unless a saved configuration says otherwise. On the bench the
    /// same two writes of zero were followed by clocks counting up from it.
    pub fn clear_clocks(&self, input: Input) -> Result<(), Error> {
        self.write_register(input, Register::RealTime, 0)?;
        self.write_register(input, Register::LiveTime, 0)
    }

    /// The preset control register: which presets are armed.
    pub fn preset_control(&self, input: Input) -> Result<u32, Error> {
        self.read_register(input, Register::PresetControl)
    }

    /// A preset's value, or zero when it is not armed.
    ///
    /// Zero is how ORTEC's dialect says "none set", and a value sitting in a
    /// register whose enable bit is clear stops nothing.
    pub fn preset(&self, input: Input, preset: Preset) -> Result<u32, Error> {
        if self.preset_control(input)? & preset.enable() == 0 {
            return Ok(0);
        }
        self.read_register(input, preset.register())
    }

    /// Sets a preset and arms it, or with zero disarms it.
    ///
    /// The other presets' enable bits are left as they are. The order keeps
    /// an armed preset from ever standing beside a stale value: the value
    /// goes in before the bit is set, and the bit is cleared before the value
    /// is zeroed.
    pub fn set_preset(&self, input: Input, preset: Preset, value: u32) -> Result<(), Error> {
        let control = self.preset_control(input)?;
        if value == 0 {
            self.write_register(input, Register::PresetControl, control & !preset.enable())?;
            self.write_register(input, preset.register(), 0)
        } else {
            self.write_register(input, preset.register(), value)?;
            self.write_register(input, Register::PresetControl, control | preset.enable())
        }
    }
}

/// The environment variable that names the FPGA image.
pub const FIRMWARE_VARIABLE: &str = "MANTARAY_M927_RBF";

/// The size of ORTEC's `M927.rbf`.
pub const FIRMWARE_SIZE: usize = 85_828;

/// The name the FPGA image is looked for under in the settings directory.
pub const FIRMWARE_FILE: &str = "M927.rbf";

/// Where the FPGA image is: the command line's word, the environment's, or
/// else [`FIRMWARE_FILE`] in `settings` (the application's settings
/// directory) when it is there. The last is for an application started from
/// a desktop launcher, whose environment holds no variable.
pub fn firmware_path(
    flag: Option<&str>,
    environment: Option<&std::ffi::OsStr>,
    settings: Option<&Path>,
) -> Option<PathBuf> {
    flag.filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            environment
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| {
            // Only a file that is not there at all is no answer. One that is
            // there and cannot be read - a broken link, a folder that may not
            // be searched - is named, so the error says why.
            let candidate = settings?.join(FIRMWARE_FILE);
            match std::fs::symlink_metadata(&candidate) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                _ => Some(candidate),
            }
        })
}

/// Reads the FPGA image, or says how to provide one.
pub fn firmware_image(path: Option<&Path>) -> Result<Vec<u8>, String> {
    let Some(path) = path else {
        let settings = match mantaray_core::storage::directory() {
            Some(directory) => directory.join(FIRMWARE_FILE).display().to_string(),
            None => FIRMWARE_FILE.to_string(),
        };
        return Err(format!(
            "the 927's FPGA is not configured (it forgets its configuration at every power-up) \
             and no image was given to load. Give ORTEC's M927.rbf with `--firmware PATH`, by \
             setting {FIRMWARE_VARIABLE}=PATH, or by copying it into MantaRay's settings \
             directory as {settings}; the file comes with ORTEC's software and is not part of \
             MantaRay"
        ));
    };
    let image = std::fs::read(path).map_err(|error| {
        let link = std::fs::read_link(path)
            .map(|target| format!(" (a link to {})", target.display()))
            .unwrap_or_default();
        format!("reading the FPGA image {}{link}: {error}", path.display())
    })?;
    if image.is_empty() {
        return Err(format!("the FPGA image {} is empty", path.display()));
    }
    Ok(image)
}

/// Test doubles for a 927, shared with the tests of the layers above.
#[cfg(test)]
pub(crate) mod double {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;

    /// A scripted device: answers reads from a queue, records what was sent
    /// and which endpoints were read.
    pub(crate) struct Scripted {
        pub sent: RefCell<Vec<(u8, Vec<u8>)>>,
        pub read: RefCell<Vec<u8>>,
        answers: RefCell<VecDeque<Result<Vec<u8>, String>>>,
        /// When set, a write accepts only this many bytes.
        pub accepts: Cell<Option<usize>>,
    }

    impl Scripted {
        pub fn answering(answers: &[&[u8]]) -> Self {
            Self {
                sent: RefCell::new(Vec::new()),
                read: RefCell::new(Vec::new()),
                answers: RefCell::new(answers.iter().map(|answer| Ok(answer.to_vec())).collect()),
                accepts: Cell::new(None),
            }
        }

        /// Queues a read that fails the way a timed-out one does.
        pub fn then_times_out(self) -> Self {
            self.answers
                .borrow_mut()
                .push_back(Err("the instrument did not answer within 3000 ms".into()));
            self
        }
    }

    impl BulkDevice for Scripted {
        fn bulk(&self, endpoint: u8, data: &mut [u8], _milliseconds: u32) -> Result<usize, String> {
            if endpoint & 0x80 == 0 {
                self.sent.borrow_mut().push((endpoint, data.to_vec()));
                Ok(self.accepts.get().unwrap_or(data.len()))
            } else {
                self.read.borrow_mut().push(endpoint);
                let answer = self
                    .answers
                    .borrow_mut()
                    .pop_front()
                    .unwrap_or(Ok(Vec::new()))?;
                let taken = answer.len().min(data.len());
                data[..taken].copy_from_slice(&answer[..taken]);
                // The marker the box sends after an answer of whole packets,
                // by the same rule as `Fake927`'s.
                if endpoint == FAST_IN
                    && !answer.is_empty()
                    && answer.len() % FAST_PACKET == 0
                    && data.len() <= answer.len()
                {
                    self.answers.borrow_mut().push_front(Ok(Vec::new()));
                }
                Ok(taken)
            }
        }
    }

    /// A 927 that behaves: registers, spectrum memory and an FPGA that needs
    /// loading, decoded from the same frames the protocol layer builds.
    ///
    /// The decoding is written from ORCA's source a second time, on purpose,
    /// so that a slip in the encoding does not pass by agreeing with itself.
    pub(crate) struct Fake927 {
        pub registers: RefCell<[u32; 0x40]>,
        pub memory: RefCell<Vec<u32>>,
        pub configured: Cell<bool>,
        /// The image bytes received since the last init.
        pub loaded: RefCell<Vec<u8>>,
        /// Every slow-channel opcode received, in order.
        pub slow_ops: RefCell<Vec<u8>>,
        /// Every register write received, as (address, value).
        pub writes: RefCell<Vec<(u8, u32)>>,
        /// Every discriminator set, as (index, value).
        pub discriminators: RefCell<Vec<(u8, u16)>>,
        /// How many times the box was held.
        pub holds: Cell<usize>,
        /// When set, the box is held by somebody else.
        pub busy: Cell<bool>,
        /// Whether this process holds it now.
        held: Cell<bool>,
        slow_answers: RefCell<VecDeque<Vec<u8>>>,
        fast_answers: RefCell<VecDeque<Vec<u8>>>,
    }

    impl Fake927 {
        pub fn configured() -> Self {
            let fake = Self::unconfigured();
            fake.configured.set(true);
            fake
        }

        pub fn unconfigured() -> Self {
            Self {
                registers: RefCell::new([0; 0x40]),
                memory: RefCell::new(vec![0; 0x1_0000]),
                configured: Cell::new(false),
                loaded: RefCell::new(Vec::new()),
                slow_ops: RefCell::new(Vec::new()),
                writes: RefCell::new(Vec::new()),
                discriminators: RefCell::new(Vec::new()),
                holds: Cell::new(0),
                busy: Cell::new(false),
                held: Cell::new(false),
                slow_answers: RefCell::new(VecDeque::new()),
                fast_answers: RefCell::new(VecDeque::new()),
            }
        }

        pub fn set(&self, input: Input, register: Register, value: u32) {
            self.registers.borrow_mut()[(input.register_base() + register as u32) as usize] = value;
        }

        pub fn get(&self, input: Input, register: Register) -> u32 {
            self.registers.borrow()[(input.register_base() + register as u32) as usize]
        }

        fn slow(&self, packet: &[u8]) {
            let op = packet[0];
            self.slow_ops.borrow_mut().push(op);
            // The header is `[op, 0, reply length, 0]`, the reply length being
            // what that op takes in `Mcb927`; a box answers anything else
            // "invalid command".
            let reply_length = match op {
                0x06 => 4,
                0x12 => 6,
                0x07 => 12,
                _ => 2,
            };
            let payload = packet.get(4..).unwrap_or_default();
            let well_formed = packet.len() >= 4
                && packet[1..4] == [0, reply_length, 0]
                && match op {
                    0x05 => !payload.is_empty() && payload.len() <= FPGA_CHUNK,
                    // Padding either side of the discriminator index.
                    0x07 => {
                        packet.len() == 12 && packet[4..7] == [0, 0, 0] && packet[8..10] == [0, 0]
                    }
                    _ => payload.is_empty(),
                };
            if !well_formed {
                self.slow_answers.borrow_mut().push_back(vec![0x81, 0x84]);
                return;
            }
            let answer: Vec<u8> = match op {
                0x04 => {
                    self.configured.set(false);
                    self.loaded.borrow_mut().clear();
                    vec![0, 0]
                }
                0x05 => {
                    self.loaded.borrow_mut().extend_from_slice(&packet[4..]);
                    vec![0, 0]
                }
                0x06 => {
                    if !self.loaded.borrow().is_empty() {
                        self.configured.set(true);
                    }
                    if self.configured.get() {
                        vec![0, 0, 0, 0]
                    } else {
                        // What the bench 927 answered before its first load.
                        vec![0x04, 0x80, 0x43, 0xDF]
                    }
                }
                0x07 => {
                    let value = u16::from_be_bytes([packet[10], packet[11]]);
                    self.discriminators.borrow_mut().push((packet[7], value));
                    vec![0, 0]
                }
                0x08 | 0x09 | 0x10 | 0x11 => vec![0, 0],
                0x12 => vec![0, 0, 0, 0, 0, 2],
                _ => vec![0x81, 0x84],
            };
            self.slow_answers.borrow_mut().push_back(answer);
        }

        fn fast(&self, frame: &[u8]) {
            // An unconfigured FPGA answers nothing, and the read times out.
            if !self.configured.get() {
                return;
            }
            let frame: Vec<u32> = frame
                .as_chunks::<4>()
                .0
                .iter()
                .map(|word| u32::from_le_bytes(*word))
                .collect();
            let (command, count) = (frame[0], frame[1] as usize);
            let address = (command & 0xFFFF) as usize;
            let register = command & 0x0004_0000 != 0;
            let answer = match (command >> 24, register) {
                (0x00, true) => self.registers.borrow()[address].to_le_bytes().to_vec(),
                (0x00, false) => words(&self.memory.borrow()[address..address + count]),
                // A write and a clear are answered with a zero-length packet.
                (0x01, true) => {
                    self.write(address, frame[2]);
                    Vec::new()
                }
                (0x04, false) => {
                    self.memory.borrow_mut()[address..address + count]
                        .copy_from_slice(&frame[2..2 + count]);
                    Vec::new()
                }
                _ => return,
            };
            self.fast_answers.borrow_mut().push_back(answer);
        }

        fn write(&self, address: usize, value: u32) {
            self.writes.borrow_mut().push((address as u8, value));
            let mut registers = self.registers.borrow_mut();
            let base = address & 0x20;
            let status = base + Register::AcquisitionStatus as usize;
            match address & 0x1F {
                // Acquisition follows the start bit's edges, not its level.
                0x00 => {
                    let was = registers[address] & START;
                    if value & START != 0 && was == 0 {
                        registers[status] |= START;
                    } else if value & START == 0 {
                        registers[status] &= !START;
                    }
                }
                0x1F if value == 1 => registers[status] &= !START,
                _ => {}
            }
            registers[address] = value;
        }
    }

    impl BulkDevice for Fake927 {
        fn bulk(&self, endpoint: u8, data: &mut [u8], milliseconds: u32) -> Result<usize, String> {
            // As `Device` does: no endpoints outside a hold.
            if !self.held.get() {
                return Err(
                    "the 927 is not held: its endpoints are only open inside one operation".into(),
                );
            }
            let answers = match endpoint {
                SLOW_OUT => {
                    self.slow(data);
                    return Ok(data.len());
                }
                FAST_OUT => {
                    self.fast(data);
                    return Ok(data.len());
                }
                SLOW_IN => &self.slow_answers,
                FAST_IN => &self.fast_answers,
                other => return Err(format!("a 927 has no endpoint {other:#04x}")),
            };
            let Some(answer) = answers.borrow_mut().pop_front() else {
                return Err(format!(
                    "the instrument did not answer within {milliseconds} ms"
                ));
            };
            let taken = answer.len().min(data.len());
            data[..taken].copy_from_slice(&answer[..taken]);
            // An answer that fills whole packets is followed by a zero-length
            // one, as measured after a spectrum. A read with room to spare
            // ends on it; a read sized exactly leaves it for the next.
            if endpoint == FAST_IN
                && !answer.is_empty()
                && answer.len() % FAST_PACKET == 0
                && data.len() <= answer.len()
            {
                answers.borrow_mut().push_front(Vec::new());
            }
            Ok(taken)
        }
    }

    impl Shared for Fake927 {
        fn hold<T>(
            &self,
            _patience: Duration,
            operation: impl FnOnce(&dyn BulkDevice) -> Result<T, String>,
        ) -> Result<T, String> {
            self.holds.set(self.holds.get() + 1);
            if self.held.get() {
                return Err("the 927 is already held by this process".into());
            }
            if self.busy.get() {
                return Err("the 927 is held by another program".into());
            }
            // Let go however the operation ends, as `Device::hold` does.
            struct Release<'a>(&'a Cell<bool>);
            impl Drop for Release<'_> {
                fn drop(&mut self) {
                    self.0.set(false);
                }
            }
            self.held.set(true);
            let _release = Release(&self.held);
            operation(self)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::double::{Fake927, Scripted};
    use super::*;

    /// The answer to a register write or a clear: a zero-length packet.
    const ACK: &[u8] = &[];

    fn sent(device: &Scripted) -> Vec<(u8, Vec<u8>)> {
        device.sent.borrow().clone()
    }

    /// Runs `operation` with the fake held, as every caller of a real box does.
    fn held<T>(fake: &Fake927, operation: impl FnOnce(&Mcb927<'_>) -> T) -> T {
        fake.hold(PATIENCE, |wire| Ok(operation(&Mcb927::new(wire))))
            .expect("held")
    }

    #[test]
    fn the_fake_lets_go_of_a_hold_that_panics() {
        let fake = Fake927::configured();
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fake.hold(PATIENCE, |_| -> Result<(), String> {
                panic!("an operation that panics")
            })
        }));
        assert!(unwound.is_err());
        assert_eq!(fake.hold(PATIENCE, |_| Ok(())), Ok(()));
    }

    #[test]
    fn the_fpga_test_is_the_exchange_measured_on_the_bench() {
        // 2026-11-13, a 927 just powered up: `06 00 04 00` out on 0x01, and
        // exactly these four bytes back on 0x81.
        let device = Scripted::answering(&[&[0x04, 0x80, 0x43, 0xDF]]);
        assert_eq!(
            Mcb927::new(&device).fpga_status(),
            Ok(Fpga::NeedsConfiguration)
        );
        assert_eq!(sent(&device), [(0x01, vec![0x06, 0x00, 0x04, 0x00])]);
        assert_eq!(*device.read.borrow(), [0x81]);

        let device = Scripted::answering(&[&[0x00, 0x00, 0x00, 0x00]]);
        assert_eq!(Mcb927::new(&device).fpga_status(), Ok(Fpga::Configured));
    }

    #[test]
    fn an_fpga_test_answered_with_any_other_status_is_an_error() {
        let device = Scripted::answering(&[&[0x81, 0x84]]);
        assert_eq!(
            Mcb927::new(&device).fpga_status(),
            Err(Error::Status {
                op: 0x06,
                status: Status::INVALID_COMMAND
            })
        );
    }

    #[test]
    fn the_firmware_version_is_the_exchange_measured_on_the_bench() {
        // Same bench, same day: `12 00 06 00` answered six bytes, version 2,
        // and did so with the FPGA not yet loaded.
        let device = Scripted::answering(&[&[0x00, 0x00, 0x00, 0x00, 0x00, 0x02]]);
        assert_eq!(Mcb927::new(&device).firmware_version(), Ok(2));
        assert_eq!(sent(&device), [(0x01, vec![0x12, 0x00, 0x06, 0x00])]);
        assert_eq!(*device.read.borrow(), [0x81]);
    }

    #[test]
    fn the_firmware_version_is_big_endian() {
        let device = Scripted::answering(&[&[0x00, 0x00, 0x01, 0x02, 0x03, 0x04]]);
        assert_eq!(Mcb927::new(&device).firmware_version(), Ok(0x0102_0304));
    }

    #[test]
    fn a_version_answer_cut_short_is_refused() {
        let device = Scripted::answering(&[&[0x00, 0x00, 0x00, 0x00]]);
        assert_eq!(
            Mcb927::new(&device).firmware_version(),
            Err(Error::Size {
                what: "a version answer",
                wanted: 6,
                got: 4
            })
        );
    }

    #[test]
    fn a_slow_answer_too_short_to_hold_a_status_is_refused() {
        for answer in [&[][..], &[0x00][..]] {
            let device = Scripted::answering(&[answer]);
            let error = Mcb927::new(&device).firmware_version().unwrap_err();
            assert_eq!(
                error,
                Error::Size {
                    what: "a status pair",
                    wanted: 2,
                    got: answer.len()
                }
            );
        }
    }

    #[test]
    fn a_slow_command_that_times_out_says_so() {
        let device = Scripted::answering(&[]).then_times_out();
        let error = Mcb927::new(&device).fpga_status().unwrap_err();
        assert_eq!(
            error,
            Error::Wire("the instrument did not answer within 3000 ms".into())
        );
    }

    #[test]
    fn a_command_only_partly_accepted_is_an_error_and_no_answer_is_read() {
        let device = Scripted::answering(&[&[0, 0, 0, 0]]);
        device.accepts.set(Some(3));
        let error = Mcb927::new(&device).fpga_status().unwrap_err();
        assert_eq!(
            error,
            Error::Size {
                what: "the command",
                wanted: 4,
                got: 3
            }
        );
        assert!(device.read.borrow().is_empty());
    }

    #[test]
    fn the_status_pairs_read_as_orca_reads_them() {
        assert_eq!(Status::new(0, 0).meaning(), "success");
        assert_eq!(
            Status::new(0x04, 0x80).meaning(),
            "the FPGA failed or needs configuration"
        );
        assert_eq!(Status::new(0x81, 0x84).meaning(), "invalid command");
        // Micro 2 is tested before the pairs, whatever the macro code.
        assert_eq!(
            Status::new(0x83, 0x02).meaning(),
            "a preset is already exceeded"
        );
        assert_eq!(Status::new(0x83, 0x01).meaning(), "unspecified error");
        let shown = Error::Status {
            op: 0x10,
            status: Status::new(0x83, 0x02),
        }
        .to_string();
        assert_eq!(
            shown,
            "the 927 answered command 0x10 with status 83 02 (a preset is already exceeded)"
        );
    }

    #[test]
    fn a_register_read_is_two_words_out_and_one_back() {
        // Acquisition status of input 2: 0x0A + 0x20, under the read opcode.
        let device = Scripted::answering(&[&[0x01, 0x00, 0x00, 0x00]]);
        let mcb = Mcb927::new(&device);
        assert_eq!(
            mcb.read_register(Input::Two, Register::AcquisitionStatus),
            Ok(1)
        );
        assert_eq!(
            sent(&device),
            [(0x08, vec![0x2A, 0x00, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00])]
        );
        assert_eq!(*device.read.borrow(), [0x82]);

        // Input 1 is the bare offset, and the value is little-endian.
        let device = Scripted::answering(&[&[0x78, 0x56, 0x34, 0x12]]);
        let mcb = Mcb927::new(&device);
        assert_eq!(
            mcb.read_register(Input::One, Register::AcquisitionStatus),
            Ok(0x1234_5678)
        );
        assert_eq!(
            sent(&device),
            [(0x08, vec![0x0A, 0x00, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00])]
        );
    }

    #[test]
    fn a_register_answer_of_any_other_size_is_refused() {
        for answer in [&[][..], &[1, 0][..], &[1, 0, 0, 0, 2, 0, 0, 0][..]] {
            let device = Scripted::answering(&[answer]);
            let error = Mcb927::new(&device)
                .read_register(Input::One, Register::LiveTime)
                .unwrap_err();
            assert_eq!(
                error,
                Error::Size {
                    what: "a register",
                    wanted: 4,
                    got: answer.len()
                }
            );
        }
    }

    #[test]
    fn a_register_read_that_times_out_says_so() {
        let device = Scripted::answering(&[]).then_times_out();
        let error = Mcb927::new(&device)
            .read_register(Input::One, Register::LiveTime)
            .unwrap_err();
        assert!(matches!(error, Error::Wire(_)), "{error}");
        assert!(error.to_string().contains("did not answer"), "{error}");
    }

    #[test]
    fn a_register_write_is_three_words_out_and_an_answer_read_away() {
        // 15000 ticks into the live preset of input 1, then of input 2.
        let device = Scripted::answering(&[ACK, ACK]);
        let mcb = Mcb927::new(&device);
        mcb.write_register(Input::One, Register::LivePreset, 15_000)
            .unwrap();
        mcb.write_register(Input::Two, Register::LivePreset, 15_000)
            .unwrap();
        assert_eq!(
            sent(&device),
            [
                (
                    0x08,
                    vec![
                        0x06, 0x00, 0x04, 0x01, 0x01, 0x00, 0x00, 0x00, 0x98, 0x3A, 0x00, 0x00
                    ]
                ),
                (
                    0x08,
                    vec![
                        0x26, 0x00, 0x04, 0x01, 0x01, 0x00, 0x00, 0x00, 0x98, 0x3A, 0x00, 0x00
                    ]
                ),
            ]
        );
        assert_eq!(*device.read.borrow(), [0x82, 0x82]);
    }

    #[test]
    fn a_register_write_with_no_answer_is_an_error() {
        let device = Scripted::answering(&[]).then_times_out();
        let error = Mcb927::new(&device)
            .write_register(Input::One, Register::LivePreset, 1)
            .unwrap_err();
        assert!(matches!(error, Error::Wire(_)), "{error}");
    }

    #[test]
    fn the_conversion_gain_reads_as_a_channel_count() {
        for (gain, channels) in [
            (0, 16_384),
            (1, 8192),
            (2, 4096),
            (3, 2048),
            (4, 1024),
            (5, 512),
            (6, 256),
        ] {
            let device = Scripted::answering(&[&u32::to_le_bytes(gain)]);
            assert_eq!(Mcb927::new(&device).channels(Input::One), Ok(channels));
        }
        let device = Scripted::answering(&[&u32::to_le_bytes(7)]);
        let error = Mcb927::new(&device).channels(Input::Two).unwrap_err();
        assert!(error.to_string().contains("input 2 holds 7"), "{error}");
    }

    #[test]
    fn a_spectrum_read_is_the_frame_orca_sends() {
        // Input 1, all 16384 channels: address 0, count 0x4000.
        let memory = words(&(0..16_384).collect::<Vec<u32>>());
        let device = Scripted::answering(&[&memory]);
        let counts = Mcb927::new(&device)
            .read_spectrum(Input::One, 0, 16_384)
            .unwrap();
        assert_eq!(counts.len(), 16_384);
        assert!(counts.iter().enumerate().all(|(i, c)| *c == i as u32));
        assert_eq!(
            sent(&device),
            [(0x08, vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00])]
        );
        assert_eq!(*device.read.borrow(), [0x82], "one read carries it all");

        // Input 2 is 0x8000 further on.
        let memory = words(&[7, 8, 9, 10]);
        let device = Scripted::answering(&[&memory]);
        let counts = Mcb927::new(&device)
            .read_spectrum(Input::Two, 0, 4)
            .unwrap();
        assert_eq!(counts, [7, 8, 9, 10]);
        assert_eq!(
            sent(&device),
            [(0x08, vec![0x00, 0x80, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00])]
        );
    }

    #[test]
    fn a_spectrum_read_from_further_in_moves_the_address() {
        let device = Scripted::answering(&[&words(&[5, 6])]);
        let counts = Mcb927::new(&device)
            .read_spectrum(Input::Two, 0x100, 2)
            .unwrap();
        assert_eq!(counts, [5, 6]);
        assert_eq!(
            sent(&device),
            [(0x08, vec![0x00, 0x81, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00])]
        );
    }

    #[test]
    fn a_spectrum_that_arrives_in_pieces_is_put_together() {
        let memory = words(&(0..256).collect::<Vec<u32>>());
        let mut pieces: Vec<&[u8]> = memory.chunks(512).collect();
        pieces.push(&[1, 0, 0, 0]);
        let device = Scripted::answering(&pieces);
        let mcb = Mcb927::new(&device);
        let counts = mcb.read_spectrum(Input::One, 0, 256).unwrap();
        assert!(counts.iter().enumerate().all(|(i, c)| *c == i as u32));
        assert_eq!(*device.read.borrow(), [0x82, 0x82]);
        // Every piece had room to spare and so took its marker along: the
        // next answer is the next question's.
        assert_eq!(mcb.live_ticks(Input::One), Ok(1));
    }

    #[test]
    fn a_spectrum_cut_short_is_refused_not_padded() {
        let memory = words(&[1, 2, 3]);
        let device = Scripted::answering(&[&memory]);
        let error = Mcb927::new(&device)
            .read_spectrum(Input::One, 0, 256)
            .unwrap_err();
        assert_eq!(
            error,
            Error::Size {
                what: "the spectrum",
                wanted: 1024,
                got: 12
            }
        );
        let device = Scripted::answering(&[]).then_times_out();
        let error = Mcb927::new(&device)
            .read_spectrum(Input::One, 0, 256)
            .unwrap_err();
        assert!(matches!(error, Error::Wire(_)), "{error}");
    }

    #[test]
    fn a_spectrum_read_outside_the_memory_sends_nothing() {
        let device = Scripted::answering(&[]);
        let mcb = Mcb927::new(&device);
        assert!(mcb.read_spectrum(Input::One, 0, 0).is_err());
        assert!(mcb.read_spectrum(Input::One, 0, 16_385).is_err());
        assert!(mcb.read_spectrum(Input::One, 16_383, 2).is_err());
        assert!(mcb.read_spectrum(Input::One, usize::MAX, 2).is_err());
        assert!(sent(&device).is_empty());
    }

    #[test]
    fn a_clear_is_the_frame_orca_sends() {
        for (input, address) in [(Input::One, [0x00, 0x00]), (Input::Two, [0x00, 0x80])] {
            let device = Scripted::answering(&[ACK]);
            Mcb927::new(&device).clear(input).unwrap();
            let sent = sent(&device);
            assert_eq!(sent.len(), 1);
            let (endpoint, frame) = &sent[0];
            assert_eq!(*endpoint, 0x08);
            // (0x3FFF + 2) words, as ORCA's buffer is sized.
            assert_eq!(frame.len(), 65_540);
            assert_eq!(
                frame[..8],
                [address[0], address[1], 0x00, 0x04, 0xFF, 0x3F, 0x00, 0x00]
            );
            assert!(frame[8..].iter().all(|byte| *byte == 0));
            assert_eq!(*device.read.borrow(), [0x82]);
        }
    }

    #[test]
    fn clearing_the_clocks_writes_zero_to_both() {
        let device = Scripted::answering(&[ACK, ACK]);
        Mcb927::new(&device).clear_clocks(Input::Two).unwrap();
        assert_eq!(
            sent(&device),
            [
                (0x08, words(&[0x0104_002C, 1, 0])),
                (0x08, words(&[0x0104_002B, 1, 0])),
            ]
        );
    }

    #[test]
    fn a_start_writes_the_control_register_and_nothing_else() {
        // Not acquiring; the control register holds a gate and the upper
        // discriminator bit, which must come back out as they went in.
        let control: u32 = 0x3000_0020;
        let device = Scripted::answering(&[
            &u32::to_le_bytes(0), // acquisition status
            &[0x00, 0x00],        // reset MDA
            &u32::to_le_bytes(control),
            ACK,
            ACK,
        ]);
        Mcb927::new(&device).start(Input::Two).unwrap();
        assert_eq!(
            sent(&device),
            [
                (0x08, words(&[0x0004_002A, 1])),
                (0x01, vec![0x10, 0x00, 0x02, 0x00]),
                (0x08, words(&[0x0004_0020, 1])),
                (0x08, words(&[0x0104_0020, 1, 0x3000_0020])),
                (0x08, words(&[0x0104_0020, 1, 0x3000_0021])),
            ]
        );
        assert_eq!(*device.read.borrow(), [0x82, 0x81, 0x82, 0x82, 0x82]);
    }

    #[test]
    fn a_start_on_an_input_already_acquiring_changes_nothing() {
        let device = Scripted::answering(&[&u32::to_le_bytes(1)]);
        Mcb927::new(&device).start(Input::One).unwrap();
        assert_eq!(sent(&device), [(0x08, words(&[0x0004_000A, 1]))]);
    }

    #[test]
    fn a_start_refused_by_the_reset_writes_nothing() {
        let device = Scripted::answering(&[&u32::to_le_bytes(0), &[0x83, 0x02]]);
        let error = Mcb927::new(&device).start(Input::One).unwrap_err();
        assert_eq!(
            error,
            Error::Status {
                op: 0x10,
                status: Status::new(0x83, 0x02)
            }
        );
        assert_eq!(sent(&device).len(), 2, "the status read and the reset");
    }

    #[test]
    fn a_start_makes_an_edge_even_when_the_start_bit_was_left_set() {
        // A count ended by a preset: not acquiring, start bit still standing.
        let fake = Fake927::configured();
        fake.set(Input::One, Register::Control, 0x21);
        held(&fake, |mcb| mcb.start(Input::One)).unwrap();
        assert_eq!(fake.get(Input::One, Register::AcquisitionStatus) & 1, 1);
        assert_eq!(fake.get(Input::One, Register::Control), 0x21);
        assert_eq!(*fake.writes.borrow(), [(0x00, 0x20), (0x00, 0x21)]);
    }

    #[test]
    fn a_stop_is_orcas_three_writes() {
        let device = Scripted::answering(&[
            &u32::to_le_bytes(1), // acquiring
            ACK,
            ACK,
            &u32::to_le_bytes(0x3000_0021),
            ACK,
        ]);
        Mcb927::new(&device).stop(Input::One).unwrap();
        assert_eq!(
            sent(&device),
            [
                (0x08, words(&[0x0004_000A, 1])),
                (0x08, words(&[0x0104_001F, 1, 1])),
                (0x08, words(&[0x0104_001F, 1, 0])),
                (0x08, words(&[0x0004_0000, 1])),
                (0x08, words(&[0x0104_0000, 1, 0x3000_0020])),
            ]
        );
    }

    #[test]
    fn a_stop_on_an_idle_input_changes_nothing() {
        let device = Scripted::answering(&[&u32::to_le_bytes(0)]);
        Mcb927::new(&device).stop(Input::Two).unwrap();
        assert_eq!(sent(&device), [(0x08, words(&[0x0004_002A, 1]))]);
    }

    /// A made-up image: not ORTEC's, and not a size any FPGA would take.
    fn image() -> Vec<u8> {
        (0..120_u8).collect()
    }

    #[test]
    fn an_fpga_load_is_orcas_sequence_in_fifty_byte_packets() {
        let ok: &[u8] = &[0x00, 0x00];
        let device = Scripted::answering(&[ok, ok, ok, ok, ok, &[0, 0, 0, 0], ok, ok]);
        let image = image();
        Mcb927::new(&device).load_fpga(&image).unwrap();
        let data = |range: std::ops::Range<usize>| {
            let mut packet = vec![0x05, 0x00, 0x02, 0x00];
            packet.extend_from_slice(&image[range]);
            (0x01, packet)
        };
        assert_eq!(
            sent(&device),
            [
                (0x01, vec![0x08, 0x00, 0x02, 0x00]),
                (0x01, vec![0x04, 0x00, 0x02, 0x00]),
                data(0..50),
                data(50..100),
                data(100..120),
                (0x01, vec![0x06, 0x00, 0x04, 0x00]),
                (0x01, vec![0x11, 0x00, 0x02, 0x00]),
                (0x01, vec![0x09, 0x00, 0x02, 0x00]),
            ]
        );
        assert_eq!(*device.read.borrow(), [0x81; 8], "an answer after each");
    }

    #[test]
    fn an_fpga_load_stops_at_the_first_bad_answer() {
        let ok: &[u8] = &[0x00, 0x00];
        let device = Scripted::answering(&[ok, ok, ok, &[0x81, 0x84]]);
        let error = Mcb927::new(&device).load_fpga(&image()).unwrap_err();
        assert_eq!(
            error,
            Error::Loading {
                sent: 50,
                fault: Box::new(Error::Status {
                    op: 0x05,
                    status: Status::INVALID_COMMAND
                })
            }
        );
        assert_eq!(sent(&device).len(), 4, "nothing after the refused packet");
    }

    #[test]
    fn an_fpga_that_fails_its_test_after_loading_is_not_reset_or_released() {
        let ok: &[u8] = &[0x00, 0x00];
        let device = Scripted::answering(&[ok, ok, ok, ok, ok, &[0x04, 0x80, 0x43, 0xDF]]);
        let error = Mcb927::new(&device).load_fpga(&image()).unwrap_err();
        assert_eq!(
            error,
            Error::Status {
                op: 0x06,
                status: Status::NEEDS_CONFIGURATION
            }
        );
        assert_eq!(sent(&device).last().unwrap().1[0], 0x06);
    }

    #[test]
    fn an_empty_image_sends_nothing() {
        let device = Scripted::answering(&[]);
        assert!(matches!(
            Mcb927::new(&device).load_fpga(&[]),
            Err(Error::Refused(_))
        ));
        assert!(sent(&device).is_empty());
    }

    #[test]
    fn the_fake_refuses_what_a_real_box_refuses() {
        // A real `Device` refuses a hold inside a hold, and has no
        // endpoints at all outside one.
        let fake = Fake927::configured();
        let nested = fake.hold(PATIENCE, |_| fake.hold(PATIENCE, |_| Ok(())));
        assert!(nested.is_err(), "a hold inside a hold");
        let outside = Mcb927::new(&fake).is_counting(Input::One);
        assert!(matches!(outside, Err(Error::Wire(_))), "{outside:?}");
        // And once let go, it can be held again.
        assert_eq!(fake.hold(PATIENCE, |_| Ok(())), Ok(()));
    }

    #[test]
    fn the_fake_refuses_a_slow_packet_with_the_wrong_header() {
        let fake = Fake927::configured();
        let answer = |packet: &[u8]| {
            fake.hold(PATIENCE, |wire| {
                wire.bulk(SLOW_OUT, &mut packet.to_vec(), 100)?;
                let mut reply = [0_u8; SLOW_PACKET];
                let got = wire.bulk(SLOW_IN, &mut reply, 100)?;
                Ok(reply[..got].to_vec())
            })
            .expect("held")
        };
        let invalid = vec![0x81, 0x84];
        assert_eq!(answer(&[0x06, 0x00, 0x04, 0x00]), [0, 0, 0, 0]);
        assert_eq!(answer(&[0x06, 0x00, 0x02, 0x00]), invalid, "reply length");
        assert_eq!(answer(&[0x06, 0xFF, 0x04, 0x00]), invalid, "second byte");
        assert_eq!(answer(&[0x06, 0x00, 0x04, 0xFF]), invalid, "fourth byte");
        let mut chunk = vec![0x05, 0x00, 0x02, 0x00];
        chunk.extend([0x5A; FPGA_CHUNK + 1]);
        assert_eq!(answer(&chunk), invalid, "a chunk over 50 bytes");
        let mut discriminator = vec![0x07, 0, 0x0C, 0, 0, 0, 0, 1, 0, 0, 0x3F, 0xFF];
        assert_eq!(answer(&discriminator), [0, 0]);
        discriminator[5] = 1;
        assert_eq!(answer(&discriminator), invalid, "padding");
    }

    #[test]
    fn the_fake_takes_a_load_the_way_the_sequence_gives_it() {
        let fake = Fake927::unconfigured();
        held(&fake, |mcb| {
            assert_eq!(mcb.fpga_status(), Ok(Fpga::NeedsConfiguration));
            mcb.load_fpga(&image()).unwrap();
            assert_eq!(*fake.loaded.borrow(), image());
            assert_eq!(
                *fake.slow_ops.borrow(),
                [0x06, 0x08, 0x04, 0x05, 0x05, 0x05, 0x06, 0x11, 0x09]
            );
            assert_eq!(mcb.fpga_status(), Ok(Fpga::Configured));
        });
    }

    #[test]
    fn the_discriminators_are_the_frames_measured_on_the_bench() {
        // 2026-11-13: these four, lower 164 and upper 16383 on each input,
        // each answered `00 00`, are what made a freshly loaded 927 count.
        let ok: &[u8] = &[0x00, 0x00];
        let device = Scripted::answering(&[ok, ok, ok, ok]);
        let mcb = Mcb927::new(&device);
        mcb.set_discriminators(Input::One, 164, 16_383).unwrap();
        mcb.set_discriminators(Input::Two, 164, 16_383).unwrap();
        let frame = |index: u8, high: u8, low: u8| {
            (
                0x01,
                vec![
                    0x07, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x00, index, 0x00, 0x00, high, low,
                ],
            )
        };
        assert_eq!(
            sent(&device),
            [
                frame(1, 0x00, 0xA4),
                frame(0, 0x3F, 0xFF),
                frame(3, 0x00, 0xA4),
                frame(2, 0x3F, 0xFF),
            ]
        );
        assert_eq!(*device.read.borrow(), [0x81; 4]);
    }

    #[test]
    fn a_discriminator_past_full_scale_sends_nothing() {
        let device = Scripted::answering(&[]);
        let error = Mcb927::new(&device)
            .set_discriminators(Input::One, 0, 0x4000)
            .unwrap_err();
        assert!(matches!(error, Error::Refused(_)), "{error}");
        assert!(sent(&device).is_empty());
    }

    #[test]
    fn a_discriminator_the_box_refuses_is_an_error() {
        let device = Scripted::answering(&[&[0x81, 0x84]]);
        let error = Mcb927::new(&device)
            .set_discriminators(Input::One, 164, 16_383)
            .unwrap_err();
        assert_eq!(
            error,
            Error::Status {
                op: 0x07,
                status: Status::INVALID_COMMAND
            }
        );
        assert_eq!(sent(&device).len(), 1, "the upper one is not sent after");
    }

    #[test]
    fn configuring_loads_the_fpga_and_then_opens_both_inputs() {
        let fake = Fake927::unconfigured();
        held(&fake, |mcb| mcb.configure(&image())).unwrap();
        assert!(fake.configured.get());
        assert_eq!(
            *fake.slow_ops.borrow(),
            [
                0x08, 0x04, 0x05, 0x05, 0x05, 0x06, 0x11, 0x09, 0x07, 0x07, 0x07, 0x07
            ]
        );
        assert_eq!(
            *fake.discriminators.borrow(),
            [(1, 164), (0, 16_383), (3, 164), (2, 16_383)]
        );
    }

    #[test]
    fn a_load_that_fails_sets_no_discriminators() {
        let device = Scripted::answering(&[&[0x00, 0x00], &[0x81, 0x84]]);
        assert!(Mcb927::new(&device).configure(&image()).is_err());
        assert!(sent(&device).iter().all(|(_, packet)| packet[0] != 0x07));
    }

    #[test]
    fn a_write_answered_with_anything_but_an_empty_packet_is_out_of_step() {
        // Measured: a register write and a clear are each answered with a
        // zero-length packet. Four bytes here are some register's value.
        let device = Scripted::answering(&[&[1, 0, 0, 0]]);
        let error = Mcb927::new(&device)
            .write_register(Input::One, Register::LivePreset, 1)
            .unwrap_err();
        assert_eq!(
            error,
            Error::Size {
                what: "the answer to a write",
                wanted: 0,
                got: 4
            }
        );
    }

    #[test]
    fn the_marker_after_a_spectrum_is_taken_with_it() {
        // Measured: 65,536 bytes, then one zero-length packet. The fake
        // leaves that packet behind for any read sized exactly to the data,
        // so a register read straight after is what shows it was taken.
        let fake = Fake927::configured();
        fake.memory.borrow_mut()[0x8000 + 5] = 77;
        fake.set(Input::Two, Register::LiveTime, 500);
        held(&fake, |mcb| {
            let counts = mcb.read_spectrum(Input::Two, 0, 16_384).unwrap();
            assert_eq!(counts.len(), 16_384);
            assert_eq!(counts[5], 77);
            assert_eq!(mcb.live_ticks(Input::Two), Ok(500));
        });
    }

    #[test]
    fn a_preset_is_written_before_it_is_armed() {
        // Real-time preset already armed (bit 2); arming live adds bit 3.
        let device = Scripted::answering(&[&u32::to_le_bytes(0b0100), ACK, ACK]);
        Mcb927::new(&device)
            .set_preset(Input::One, Preset::Live, 15_000)
            .unwrap();
        assert_eq!(
            sent(&device),
            [
                (0x08, words(&[0x0004_0005, 1])),
                (0x08, words(&[0x0104_0006, 1, 15_000])),
                (0x08, words(&[0x0104_0005, 1, 0b1100])),
            ]
        );
    }

    #[test]
    fn a_preset_of_zero_is_disarmed_before_it_is_zeroed() {
        let device = Scripted::answering(&[&u32::to_le_bytes(0b1100), ACK, ACK]);
        Mcb927::new(&device)
            .set_preset(Input::Two, Preset::Live, 0)
            .unwrap();
        assert_eq!(
            sent(&device),
            [
                (0x08, words(&[0x0004_0025, 1])),
                (0x08, words(&[0x0104_0025, 1, 0b0100])),
                (0x08, words(&[0x0104_0026, 1, 0])),
            ]
        );
    }

    #[test]
    fn each_preset_has_its_own_register_and_enable_bit() {
        let fake = Fake927::configured();
        for (preset, register, bit, value) in [
            (Preset::Real, Register::RealPreset, 1 << 2, 11),
            (Preset::Live, Register::LivePreset, 1 << 3, 22),
            (Preset::RoiIntegral, Register::RoiIntegralPreset, 1 << 4, 33),
            (Preset::RoiPeak, Register::RoiPeakPreset, 1 << 5, 44),
        ] {
            held(&fake, |mcb| mcb.set_preset(Input::Two, preset, value)).unwrap();
            assert_eq!(fake.get(Input::Two, register), value);
            assert_ne!(fake.get(Input::Two, Register::PresetControl) & bit, 0);
            assert_eq!(held(&fake, |mcb| mcb.preset(Input::Two, preset)), Ok(value));
        }
        assert_eq!(fake.get(Input::Two, Register::PresetControl), 0b11_1100);
        assert_eq!(
            fake.get(Input::One, Register::PresetControl),
            0,
            "input 1 untouched"
        );
    }

    #[test]
    fn a_preset_that_is_not_armed_reads_as_none() {
        // A value in the register and the enable bit clear stops nothing.
        let fake = Fake927::configured();
        fake.set(Input::One, Register::LivePreset, 15_000);
        let live = || held(&fake, |mcb| mcb.preset(Input::One, Preset::Live));
        assert_eq!(live(), Ok(0));
        fake.set(Input::One, Register::PresetControl, 1 << 3);
        assert_eq!(live(), Ok(15_000));
    }

    #[test]
    fn the_fpga_version_is_read_where_orca_reads_it() {
        let device = Scripted::answering(&[&u32::to_le_bytes(9)]);
        assert_eq!(Mcb927::new(&device).fpga_version(), Ok(9));
        assert_eq!(sent(&device), [(0x08, words(&[0x0004_0031, 1]))]);
    }

    #[test]
    fn an_input_is_named_by_its_front_panel_number() {
        assert_eq!(Input::from_number(1), Some(Input::One));
        assert_eq!(Input::from_number(2), Some(Input::Two));
        assert_eq!(Input::from_number(0), None);
        assert_eq!(Input::from_number(3), None);
        assert_eq!(Input::Two.number(), 2);
    }

    #[test]
    fn the_command_line_names_the_image_before_the_environment_does() {
        let environment = std::ffi::OsStr::new("/from/environment.rbf");
        assert_eq!(
            firmware_path(Some("/from/flag.rbf"), Some(environment), None),
            Some(PathBuf::from("/from/flag.rbf"))
        );
        assert_eq!(
            firmware_path(None, Some(environment), None),
            Some(PathBuf::from("/from/environment.rbf"))
        );
        assert_eq!(firmware_path(None, None, None), None);
        assert_eq!(
            firmware_path(Some(""), Some(std::ffi::OsStr::new("")), None),
            None
        );
    }

    #[test]
    fn the_settings_directory_is_the_last_place_looked() {
        let directory = std::env::temp_dir().join(format!("m927-rbf-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let stored = directory.join(FIRMWARE_FILE);
        std::fs::write(&stored, b"image").unwrap();
        let environment = std::ffi::OsStr::new("/from/environment.rbf");

        assert_eq!(
            firmware_path(Some("/from/flag.rbf"), Some(environment), Some(&directory)),
            Some(PathBuf::from("/from/flag.rbf"))
        );
        assert_eq!(
            firmware_path(None, Some(environment), Some(&directory)),
            Some(PathBuf::from("/from/environment.rbf"))
        );
        assert_eq!(firmware_path(None, None, Some(&directory)), Some(stored));

        // A directory with no image in it is no answer, so the error that
        // follows is the one that says where an image can go.
        std::fs::remove_dir_all(&directory).unwrap();
        assert_eq!(firmware_path(None, None, Some(&directory)), None);
    }

    #[cfg(unix)]
    #[test]
    fn an_image_in_the_settings_directory_that_cannot_be_read_says_why() {
        let directory = std::env::temp_dir().join(format!("m927-broken-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let stored = directory.join(FIRMWARE_FILE);
        // A link left behind by an ORTEC install that has since moved.
        let moved = directory.join("moved/M927.rbf");
        std::os::unix::fs::symlink(&moved, &stored).unwrap();
        let found = firmware_path(None, None, Some(&directory));
        let error = firmware_image(found.as_deref()).unwrap_err();
        assert!(error.contains(&stored.display().to_string()), "{error}");
        assert!(
            error.contains(&format!("(a link to {})", moved.display())),
            "{error}"
        );
        assert!(error.contains("No such file"), "{error}");
        // Something there that is not a file at all.
        std::fs::remove_file(&stored).unwrap();
        std::fs::create_dir(&stored).unwrap();
        let found = firmware_path(None, None, Some(&directory));
        let error = firmware_image(found.as_deref()).unwrap_err();
        assert!(error.contains(&stored.display().to_string()), "{error}");
        assert!(!error.contains("no image was given"), "{error}");
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_missing_image_says_how_to_give_one() {
        let error = firmware_image(None).unwrap_err();
        assert!(error.contains("--firmware PATH"), "{error}");
        assert!(error.contains("MANTARAY_M927_RBF"), "{error}");
        assert!(error.contains("settings directory"), "{error}");
        let error = firmware_image(Some(Path::new("/nonexistent/M927.rbf"))).unwrap_err();
        assert!(error.contains("/nonexistent/M927.rbf"), "{error}");
    }
}
