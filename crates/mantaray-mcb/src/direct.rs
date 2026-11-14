//! The DPM-USB adapter over libusb, with no vendor driver at all.
//!
//! On Windows the adapter belongs to ORTEC's driver and is reached through its
//! IOCTLs; see `usb.rs`, which exists only there. Everywhere else there is no
//! driver to install:
//! the kernel's own USB stack hands the interface over, and the same frames go
//! down the same two bulk endpoints. That is the whole difference, which is why
//! [`crate::dpm`] sits on a trait rather than on either one.
//!
//! What this does **not** do is take a device away from another driver. On
//! Windows ORTEC's already owns it, so this is not wired in there; on Linux and
//! macOS nothing owns it and there is nothing to take.
//!
//! Two families of instrument are found here. A DPM-USB adapter is one
//! instrument, a 926 behind it, and its interface is claimed once and kept. A
//! Model 927 is two instruments in one box with no adapter between: it is
//! listed twice, once per input, and its interface is claimed for one
//! operation at a time so that two processes can each serve an input.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use nusb::MaybeFuture;
use nusb::transfer::{Bulk, In, Out};

use crate::dpm::BulkDevice;
use crate::mcb927::Shared;

/// ORTEC, as the bus reports it.
pub const VENDOR: u16 = 0x0A2D;

/// What kind of instrument a product number is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// The DPM-USB adapter in front of a 926.
    DpmUsb,
    /// The Model 927, a two-input MCB with USB of its own.
    Model927,
}

/// What a family looks like on the bus.
struct Kind {
    family: Family,
    product: u16,
    /// How many instruments one device is.
    inputs: u8,
    out: &'static [u8],
    input: &'static [u8],
}

const FAMILIES: [Kind; 2] = [
    Kind {
        family: Family::DpmUsb,
        product: 0x0016,
        inputs: 1,
        out: &[0x01],
        input: &[0x81],
    },
    Kind {
        family: Family::Model927,
        product: 0x0019,
        inputs: 2,
        out: &[crate::mcb927::SLOW_OUT, crate::mcb927::FAST_OUT],
        input: &[crate::mcb927::SLOW_IN, crate::mcb927::FAST_IN],
    },
];

impl Family {
    fn kind(self) -> &'static Kind {
        match self {
            Self::DpmUsb => &FAMILIES[0],
            Self::Model927 => &FAMILIES[1],
        }
    }

    fn of(product: u16) -> Option<Self> {
        FAMILIES
            .iter()
            .find(|kind| kind.product == product)
            .map(|kind| kind.family)
    }

    /// The USB product number.
    pub fn product(self) -> u16 {
        self.kind().product
    }
}

/// Every `vendor:product` pair looked for, for messages and help.
pub fn looked_for() -> String {
    FAMILIES
        .iter()
        .map(|kind| format!("{VENDOR:04x}:{:04x}", kind.product))
        .collect::<Vec<_>>()
        .join(" and ")
}

/// The interface the bulk endpoints live on.
const INTERFACE: u8 = 0;

/// One instrument on the bus: an adapter, or one input of a 927.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// What selects it: the serial number, with `-1` or `-2` after it for
    /// the inputs of a 927.
    pub name: String,
    /// The name without the input after it.
    serial: String,
    pub family: Family,
    /// Which input of the device this is, counted from one.
    pub input: u8,
    /// Which of the devices found it belongs to.
    device: usize,
}

/// The instruments a list of devices amounts to, in bus order.
///
/// `found` is each device's product number and the name it goes by.
fn entries_of(found: &[(u16, String)]) -> Vec<Entry> {
    let mut entries = Vec::new();
    for (device, (product, name)) in found.iter().enumerate() {
        let Some(family) = Family::of(*product) else {
            continue;
        };
        let inputs = family.kind().inputs;
        for input in 1..=inputs {
            entries.push(Entry {
                name: if inputs == 1 {
                    name.clone()
                } else {
                    format!("{name}-{input}")
                },
                serial: name.clone(),
                family,
                input,
                device,
            });
        }
    }
    entries
}

/// Picks the instrument asked for, or the first there when nothing is asked
/// for. `only` narrows it to a family; `Some(Family::Model927)` is for the
/// commands that act on a whole box, where a 927's bare serial names it.
///
/// A name given whole is that instrument. Otherwise `wanted` is matched
/// against serial numbers, never against the `-1` and `-2` a 927's inputs
/// are listed with - `--device 1` would find every 927 by its suffix - and
/// `PART-2` is input 2 of the 927 whose serial contains `PART`. Anything that
/// matches more than one is refused with the list, rather than settled by
/// bus order: a 927 picked by bus order is a box reloaded by mistake. For
/// the same reason a box-level command refuses the name of one input, and
/// with nothing asked for refuses a bus with more than one 927 on it.
fn choose<'a>(
    entries: &'a [Entry],
    wanted: Option<&str>,
    only: Option<Family>,
) -> Result<&'a Entry, String> {
    let mut candidates = entries
        .iter()
        .filter(|entry| only.is_none_or(|family| entry.family == family))
        .peekable();
    if candidates.peek().is_none() {
        return Err(match only {
            Some(Family::Model927) => format!(
                "no Model 927 on the bus (looking for {VENDOR:04x}:{:04x})",
                Family::Model927.product()
            ),
            Some(Family::DpmUsb) => format!(
                "no DPM-USB adapter on the bus (looking for {VENDOR:04x}:{:04x})",
                Family::DpmUsb.product()
            ),
            _ => format!(
                "no DPM-USB adapter or Model 927 on the bus (looking for {})",
                looked_for()
            ),
        });
    }
    // A box-level command acts on a 927 as a whole, so its bare serial names
    // it and each box is matched once.
    let whole_box = only == Some(Family::Model927);
    let Some(wanted) = wanted else {
        let first = candidates.next().expect("peeked");
        if !whole_box {
            return Ok(first);
        }
        let boxes: Vec<&str> = std::iter::once(first)
            .chain(candidates)
            .filter(|entry| entry.input == 1)
            .map(|entry| entry.serial.as_str())
            .collect();
        if let [_] = boxes.as_slice() {
            return Ok(first);
        }
        return Err(format!(
            "more than one Model 927 on the bus, so name one with --device: {}",
            boxes.join(", ")
        ));
    };
    let family: Vec<&Entry> = candidates.collect();
    let boxes: Vec<&Entry> = family
        .iter()
        .filter(|entry| !whole_box || entry.input == 1)
        .copied()
        .collect();
    let refused = |matches: &[&Entry], why: &str| {
        let names: Vec<&str> = matches
            .iter()
            .map(|entry| {
                if whole_box {
                    entry.serial.as_str()
                } else {
                    entry.name.as_str()
                }
            })
            .collect();
        format!("{wanted:?} {why}: {}; name one whole", names.join(", "))
    };
    let missing = || match only {
        Some(Family::DpmUsb) => format!("no DPM-USB adapter with {wanted:?} in its serial number"),
        _ => format!("no adapter with {wanted:?} in its serial number"),
    };
    if wanted.is_empty() {
        return Err(refused(&boxes, "names nothing, so it matches all of"));
    }
    if !whole_box && let Some(entry) = family.iter().find(|entry| entry.name == wanted) {
        return Ok(entry);
    }
    let bare: Vec<&Entry> = family
        .iter()
        .filter(|entry| entry.family.kind().inputs > 1 && entry.serial == wanted)
        .copied()
        .collect();
    match bare.first() {
        Some(entry) if whole_box => return Ok(entry),
        Some(_) => return Err(refused(&bare, "is a Model 927, which is two instruments")),
        None => {}
    }
    let suffixed = wanted
        .rsplit_once('-')
        .and_then(|(part, input)| Some((part, input.parse::<u8>().ok()?)));
    if whole_box
        && let Some((part, input)) = suffixed
        && (1..=Family::Model927.kind().inputs).contains(&input)
    {
        let named: Vec<&str> = boxes
            .iter()
            .filter(|entry| entry.serial.contains(part))
            .map(|entry| entry.serial.as_str())
            .collect();
        let mut error = format!(
            "{wanted:?} names one input, but this command acts on both inputs of a 927 \
             and takes the bare serial"
        );
        if !named.is_empty() {
            error.push_str(&format!(": {}", named.join(", ")));
        }
        return Err(error);
    }
    let mut matches: Vec<&Entry> = match suffixed {
        Some(("", _)) => {
            let inputs: Vec<&Entry> = family
                .iter()
                .filter(|entry| entry.name.ends_with(wanted))
                .copied()
                .collect();
            if inputs.is_empty() {
                return Err(missing());
            }
            return Err(refused(&inputs, "is an input of no 927 in particular"));
        }
        Some((part, input)) => family
            .iter()
            .filter(|entry| {
                entry.family.kind().inputs > 1
                    && entry.input == input
                    && entry.serial.contains(part)
            })
            .copied()
            .collect(),
        None => Vec::new(),
    };
    if matches.is_empty() {
        matches = boxes
            .iter()
            .filter(|entry| entry.serial.contains(wanted))
            .copied()
            .collect();
    }
    match matches.as_slice() {
        [entry] => Ok(entry),
        [] => Err(missing()),
        several => Err(refused(several, "matches more than one")),
    }
}

/// The endpoints of a claimed interface. Dropping them is what gives the
/// claim back.
struct Pipes {
    out: Vec<nusb::Endpoint<Bulk, Out>>,
    input: Vec<nusb::Endpoint<Bulk, In>>,
}

/// Why an interface could not be taken.
enum ClaimFault {
    Interface(nusb::Error),
    Endpoint(u8, nusb::Error),
}

/// Claims the interface and opens every endpoint the family uses.
fn claim(device: &nusb::Device, kind: &Kind) -> Result<Pipes, ClaimFault> {
    let interface = device
        .claim_interface(INTERFACE)
        .wait()
        .map_err(ClaimFault::Interface)?;
    let mut pipes = Pipes {
        out: Vec::new(),
        input: Vec::new(),
    };
    for address in kind.out {
        pipes.out.push(
            interface
                .endpoint::<Bulk, Out>(*address)
                .map_err(|error| ClaimFault::Endpoint(*address, error))?,
        );
    }
    for address in kind.input {
        pipes.input.push(
            interface
                .endpoint::<Bulk, In>(*address)
                .map_err(|error| ClaimFault::Endpoint(*address, error))?,
        );
    }
    Ok(pipes)
}

/// How often a busy 927 is asked again.
const CLAIM_PAUSE: Duration = Duration::from_millis(10);

/// Tries until it works, fails for a reason other than `busy`, or `patience`
/// runs out, and then hands back the last outcome.
fn patiently<T, E>(
    patience: Duration,
    pause: Duration,
    busy: impl Fn(&E) -> bool,
    mut attempt: impl FnMut() -> Result<T, E>,
) -> Result<T, E> {
    let deadline = Instant::now() + patience;
    loop {
        match attempt() {
            Err(fault) if busy(&fault) && Instant::now() < deadline => std::thread::sleep(pause),
            outcome => return outcome,
        }
    }
}

/// An instrument reached through libusb.
///
/// A DPM-USB adapter's endpoints are opened once and kept, because claiming
/// them per transfer would be both slow and a good way to lose a queued
/// answer. A 927's are opened for one operation at a time - see
/// [`Shared::hold`] - and between operations there are none. They sit behind
/// a `RefCell` so that sending stays a `&self` operation, which is what the
/// protocol layers above expect.
pub struct Device {
    pipes: RefCell<Option<Pipes>>,
    /// Kept for [`Device::cycle`] and for claiming a 927 again.
    device: nusb::Device,
    serial: String,
    family: Family,
    input: u8,
}

/// How long to wait on a leftover answer before deciding there is not one.
///
/// An answer already sitting in the adapter arrives at once, so this only has
/// to outlast the bus, not the instrument.
const DRAIN_TIMEOUT: Duration = Duration::from_millis(50);

/// Reads away answers left over from whoever held the adapter last.
///
/// Bounded rather than "until it goes quiet": an adapter that talks without
/// stopping would otherwise keep the program in here for good, and a handful
/// of frames is already far more than the one stale reply this is for.
fn drain(input: &mut nusb::Endpoint<Bulk, In>) {
    let packet = input.max_packet_size().max(1);
    for _ in 0..8 {
        let buffer = input.allocate(packet);
        let done = input.transfer_blocking(buffer, DRAIN_TIMEOUT);
        // Timing out is the expected end of this: nothing was waiting.
        if done.status.is_err() || done.actual_len == 0 {
            return;
        }
    }
}

/// Every DPM-USB adapter and Model 927 the bus can see.
fn adapters() -> Result<Vec<nusb::DeviceInfo>, String> {
    Ok(nusb::list_devices()
        .wait()
        .map_err(|error| format!("listing USB devices: {error}"))?
        .filter(|device| device.vendor_id() == VENDOR && Family::of(device.product_id()).is_some())
        .collect())
}

/// The instruments those devices amount to.
fn entries_among(found: &[nusb::DeviceInfo]) -> Vec<Entry> {
    let named: Vec<(u16, String)> = found
        .iter()
        .map(|device| {
            let name = device
                .serial_number()
                .map(str::to_string)
                .unwrap_or_else(|| format!("{}:{}", device.bus_id(), device.device_address()));
            (device.product_id(), name)
        })
        .collect();
    entries_of(&named)
}

/// What to say when an interface cannot be claimed.
///
/// Two different faults arrive here and they need opposite advice. Busy means
/// something already holds the interface - another window of this program, or
/// a kernel driver that bound it - and no udev rule will help. Denied is the
/// permission case. Sending somebody to write a udev rule when the real answer
/// is "you have it open over there" is the more likely of the two now that a
/// bench can carry more than one adapter.
fn claim_refused(fault: ClaimFault, serial: &str, family: Family) -> String {
    let error = match fault {
        ClaimFault::Endpoint(address, error) => {
            return format!("opening endpoint {address:#04x}: {error}");
        }
        ClaimFault::Interface(error) => error,
    };
    let advice = match error.kind() {
        nusb::ErrorKind::Busy => "Something else already has this adapter - another window of \
                                  this program, or a driver that has bound it. Close the other \
                                  one, or pick a different adapter."
            .to_string(),
        nusb::ErrorKind::PermissionDenied => format!(
            "This is permission: a udev rule giving {VENDOR:04x}:{PRODUCT:04x} \
             to your user is what is wanted.",
            PRODUCT = family.product()
        ),
        nusb::ErrorKind::Disconnected => {
            "The adapter went away between being listed and being opened.".to_string()
        }
        _ => "The adapter answered, but the interface could not be claimed.".to_string(),
    };
    format!("claiming interface {INTERFACE} of adapter {serial}: {error}. {advice}")
}

impl Device {
    /// Every instrument present, in the order `serve N` counts them.
    pub fn entries() -> Result<Vec<Entry>, String> {
        Ok(entries_among(&adapters()?))
    }

    /// The name of every instrument present: an adapter's serial number, or
    /// a 927's with `-1` and `-2` after it for its two inputs.
    pub fn list() -> Result<Vec<String>, String> {
        Ok(Self::entries()?
            .into_iter()
            .map(|entry| entry.name)
            .collect())
    }

    /// Opens an instrument: the one `wanted` names, as [`choose`] reads it,
    /// or the first one there when nothing is asked for.
    ///
    /// `9049204-2` is input 2 of the 927 with that serial; the bare serial
    /// is refused, naming both inputs, since it is two instruments.
    pub fn open(wanted: Option<&str>) -> Result<Self, String> {
        Self::open_among(wanted, None)
    }

    /// Opens a Model 927 and nothing else, for the commands only it has,
    /// which act on the whole box: its bare serial names it.
    pub fn open_927(wanted: Option<&str>) -> Result<Self, String> {
        Self::open_among(wanted, Some(Family::Model927))
    }

    /// Opens a DPM-USB adapter and nothing else, for the 926's commands.
    pub fn open_dpm(wanted: Option<&str>) -> Result<Self, String> {
        Self::open_among(wanted, Some(Family::DpmUsb))
    }

    fn open_among(wanted: Option<&str>, only: Option<Family>) -> Result<Self, String> {
        let found = adapters()?;
        let entries = entries_among(&found);
        let entry = choose(&entries, wanted, only)?;
        let chosen = &found[entry.device];
        let serial = chosen
            .serial_number()
            .map(str::to_string)
            .unwrap_or_default();
        let device = chosen
            .open()
            .wait()
            .map_err(|error| format!("opening adapter {serial}: {error}"))?;
        let pipes = match entry.family {
            Family::DpmUsb => Some(
                claim(&device, entry.family.kind())
                    .map_err(|fault| claim_refused(fault, &serial, entry.family))?,
            ),
            // Claimed per operation, so that the process serving the other
            // input can have its turn.
            Family::Model927 => None,
        };
        // Deliberately *not* settled here. Draining an endpoint that has
        // nothing to say costs a run of cancelled reads, and on this transport
        // that is not free: the instrument answers an IN token it is never
        // given credit for, advances its data toggle, and from then on speaks
        // past a host whose own toggle never moved. Measured on a 926, opening
        // like that turned an adapter that answers into one that never answers
        // again. Settling is a repair, so it is asked for - see `usbfix` - and
        // not done to a healthy adapter on the way past.
        Ok(Self {
            pipes: RefCell::new(pipes),
            device,
            serial,
            family: entry.family,
            input: entry.input,
        })
    }

    /// The device's serial number, which is the instrument's identity here.
    /// A 927's two inputs share it; [`Device::input`] tells them apart.
    pub fn serial(&self) -> &str {
        &self.serial
    }

    pub fn family(&self) -> Family {
        self.family
    }

    /// Which input of the device this handle is for: always 1 on an adapter.
    pub fn input(&self) -> u8 {
        self.input
    }

    /// Brings both bulk endpoints back to a known state.
    ///
    /// The counterpart of the Windows `settle`, for the fault it is named
    /// after: the adapter answers what it was last asked whether or not anyone
    /// is still listening, so a request abandoned by some earlier program
    /// leaves a reply queued, and every answer after it is one question late.
    /// Reading until nothing more comes back is what puts the two back in step.
    ///
    /// Unlike the Windows one this is **not** run on opening or after a
    /// transfer times out. Over libusb the reads it costs do their own harm -
    /// see the note in [`Device::open`] - so it is worth doing only once an
    /// adapter is known to be stuck, which is [`crate::usb_fix`]'s job.
    /// The order matters and is the opposite of the obvious one. Reading the
    /// queue empty is what puts question and answer back in step, but every
    /// read that finds nothing is a cancelled transfer, and those are what put
    /// the two ends' data toggles out of step in the first place. So the halts
    /// are cleared last, once there is nothing left to read: clearing a halt
    /// resets the toggle, and doing it first only means throwing away the
    /// repair before the damage.
    pub fn settle(&self) {
        // Nothing to settle on a 927 between operations: it holds no
        // endpoints then, and nothing here has been tried on one.
        let mut pipes = self.pipes.borrow_mut();
        let Some(pipes) = pipes.as_mut() else {
            return;
        };
        for input in &mut pipes.input {
            drain(input);
        }
        for out in &mut pipes.out {
            out.clear_halt().wait().ok();
        }
        for input in &mut pipes.input {
            input.clear_halt().wait().ok();
        }
    }

    /// Unplugs and replugs the adapter, in software.
    ///
    /// For the state a pipe reset cannot reach: the adapter's own state machine
    /// stuck, refusing even to take a frame. It re-enumerates, so this handle
    /// is finished afterwards - hence taking `self` - and the caller opens the
    /// adapter afresh.
    pub fn cycle(self) -> Result<(), String> {
        let Self {
            pipes,
            device,
            serial,
            ..
        } = self;
        // A device with a claimed interface will not reset, and the endpoints
        // are what hold the claim, so they have to go first. This is the reason
        // the whole handle is consumed rather than borrowed.
        drop(pipes);
        device
            .reset()
            .wait()
            .map_err(|error| format!("cycling adapter {serial}: {error}"))
    }
}

impl BulkDevice for Device {
    // Every transfer goes through `transfer_blocking`, which cancels on
    // timeout and does not return until the transfer has actually come back.
    // The obvious `submit` + `wait_next_complete` pair is a trap: a timed-out
    // transfer stays queued, its completion is handed to the *next* request as
    // if it were the answer, and from then on every reply is one question
    // late. nusb's own documentation warns of exactly this.
    fn bulk(&self, endpoint: u8, data: &mut [u8], milliseconds: u32) -> Result<usize, String> {
        let timeout = Duration::from_millis(u64::from(milliseconds));
        use nusb::transfer::TransferError;
        // The top bit of an endpoint address is its direction, which decides
        // whether these bytes are being sent or are about to be filled in.
        // A failure here is deliberately left where it lies rather than settled
        // on the spot. Settling costs a run of cancelled reads, and cancelled
        // reads are what put the two ends' data toggles out of step in the
        // first place - so settling on every timeout would manufacture the
        // fault it is meant to cure, and would do it fastest on a link that is
        // merely slow. Recovery belongs to `usbfix`, where somebody has looked
        // at the adapter and decided it is actually stuck.
        let mut pipes = self.pipes.borrow_mut();
        let Some(pipes) = pipes.as_mut() else {
            return Err(
                "the 927 is not held: its endpoints are only open inside one operation".into(),
            );
        };
        if endpoint & 0x80 == 0 {
            let Some(out) = pipes
                .out
                .iter_mut()
                .find(|out| out.endpoint_address() == endpoint)
            else {
                return Err(format!("this device has no out endpoint {endpoint:#04x}"));
            };
            let done = out.transfer_blocking(data.to_vec().into(), timeout);
            match done.status {
                Ok(()) => Ok(done.actual_len),
                Err(TransferError::Cancelled) => Err(format!(
                    "the adapter did not accept a frame within {milliseconds} ms"
                )),
                Err(error) => Err(format!("writing to endpoint {endpoint:#04x}: {error}")),
            }
        } else {
            let Some(input) = pipes
                .input
                .iter_mut()
                .find(|input| input.endpoint_address() == endpoint)
            else {
                return Err(format!("this device has no in endpoint {endpoint:#04x}"));
            };
            read(input, data, timeout, milliseconds)
        }
    }
}

/// The reading half of [`BulkDevice::bulk`], which is the longer of the two
/// and the only one that has to size its own buffer.
fn read(
    input: &mut nusb::Endpoint<Bulk, In>,
    data: &mut [u8],
    timeout: Duration,
    milliseconds: u32,
) -> Result<usize, String> {
    use nusb::transfer::TransferError;
    // A read must be submitted in whole packets; the protocol layer
    // already rounds up, and this keeps the promise explicit.
    let packet = input.max_packet_size().max(1);
    let room = data.len().div_ceil(packet) * packet;
    let buffer = input.allocate(room);
    let done = input.transfer_blocking(buffer, timeout);
    match done.status {
        Ok(()) => {
            let taken = done.actual_len.min(data.len());
            data[..taken].copy_from_slice(&done.buffer[..taken]);
            Ok(taken)
        }
        Err(TransferError::Cancelled) => Err(format!(
            "the instrument did not answer within {milliseconds} ms"
        )),
        Err(error) => Err(format!(
            "reading from endpoint {:#04x}: {error}",
            input.endpoint_address()
        )),
    }
}

impl Shared for Device {
    /// Claims the 927's interface, runs the operation, and lets go.
    ///
    /// Claiming is one ioctl and so is letting go, and the device itself
    /// stays open in between, so this costs nothing an operator would see.
    /// What it buys is that the process serving the other input finds the
    /// interface free between this one's operations. A claim that is busy is
    /// asked for again until `patience` runs out.
    ///
    /// An adapter keeps the claim it took when it was opened, so for one the
    /// operation simply runs.
    fn hold<T>(
        &self,
        patience: Duration,
        operation: impl FnOnce(&dyn BulkDevice) -> Result<T, String>,
    ) -> Result<T, String> {
        if self.family != Family::Model927 {
            return operation(self);
        }
        if self.pipes.borrow().is_some() {
            return Err("the 927 is already held by this process".into());
        }
        let pipes = patiently(
            patience,
            CLAIM_PAUSE,
            |fault| matches!(fault, ClaimFault::Interface(error) if error.kind() == nusb::ErrorKind::Busy),
            || claim(&self.device, self.family.kind()),
        )
        .map_err(|fault| match fault {
            ClaimFault::Interface(error) if error.kind() == nusb::ErrorKind::Busy => format!(
                "the 927 {} stayed held by another program for {} ms. The process serving its \
                 other input lets go between operations, so this is something that does not: \
                 another program, or an FPGA load still running",
                self.serial,
                patience.as_millis()
            ),
            fault => claim_refused(fault, &self.serial, self.family),
        })?;
        holding(&self.pipes, pipes, || operation(self))
    }
}

/// Puts `pipes` where `bulk` finds them for the length of `operation`, and
/// takes them out again however it ends.
///
/// Dropping the endpoints is what releases the interface, so it is done by a
/// guard: a claim kept past a panic would shut the other process out, and
/// every later hold here would be refused as already held.
fn holding<P, T>(slot: &RefCell<Option<P>>, pipes: P, operation: impl FnOnce() -> T) -> T {
    struct Release<'a, P>(&'a RefCell<Option<P>>);
    impl<P> Drop for Release<'_, P> {
        fn drop(&mut self) {
            // A borrow still out is a bug elsewhere, and panicking here on an
            // unwind would abort; keeping the claim is the lesser harm.
            if let Ok(mut slot) = self.0.try_borrow_mut() {
                slot.take();
            }
        }
    }
    *slot.borrow_mut() = Some(pipes);
    let _release = Release(slot);
    operation()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bus(devices: &[(u16, &str)]) -> Vec<Entry> {
        let found: Vec<(u16, String)> = devices
            .iter()
            .map(|(product, name)| (*product, name.to_string()))
            .collect();
        entries_of(&found)
    }

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.name.as_str()).collect()
    }

    #[test]
    fn an_adapter_is_one_instrument_under_its_own_serial() {
        let entries = bus(&[(0x0016, "08134079"), (0x0016, "11217584")]);
        assert_eq!(names(&entries), ["08134079", "11217584"]);
        assert!(entries.iter().all(|entry| entry.family == Family::DpmUsb));
        assert!(entries.iter().all(|entry| entry.input == 1));
    }

    #[test]
    fn a_927_is_two_instruments_one_per_input() {
        let entries = bus(&[(0x0019, "9049204")]);
        assert_eq!(names(&entries), ["9049204-1", "9049204-2"]);
        assert!(entries.iter().all(|entry| entry.family == Family::Model927));
        assert_eq!(entries[0].input, 1);
        assert_eq!(entries[1].input, 2);
        assert_eq!(entries[0].device, entries[1].device);
    }

    #[test]
    fn both_families_are_listed_in_bus_order() {
        let entries = bus(&[(0x0016, "08134079"), (0x0019, "9049204"), (0x1234, "x")]);
        assert_eq!(names(&entries), ["08134079", "9049204-1", "9049204-2"]);
        assert_eq!(entries[2].device, 1);
    }

    #[test]
    fn an_input_of_a_927_is_chosen_by_its_suffix() {
        let entries = bus(&[(0x0016, "08134079"), (0x0019, "9049204")]);
        let chosen = choose(&entries, Some("9049204-2"), None).unwrap();
        assert_eq!((chosen.name.as_str(), chosen.input), ("9049204-2", 2));
        // Nothing asked for is the first instrument on the bus.
        assert_eq!(choose(&entries, None, None).unwrap().name, "08134079");
        // And an adapter is still found by any part of its serial.
        assert_eq!(
            choose(&entries, Some("0813"), None).unwrap().name,
            "08134079"
        );
    }

    #[test]
    fn the_927_commands_pass_over_an_adapter() {
        let entries = bus(&[(0x0016, "08134079"), (0x0019, "9049204")]);
        let chosen = choose(&entries, None, Some(Family::Model927)).unwrap();
        assert_eq!(chosen.name, "9049204-1");
        let error = choose(&entries, Some("0813"), Some(Family::Model927)).unwrap_err();
        assert_eq!(error, "no adapter with \"0813\" in its serial number");
    }

    #[test]
    fn a_box_level_command_refuses_the_name_of_one_input() {
        let entries = bus(&[(0x0019, "9049204"), (0x0019, "9049777")]);
        for wanted in ["9049204-2", "9049204-1", "777-2"] {
            let error = choose(&entries, Some(wanted), Some(Family::Model927)).unwrap_err();
            assert!(error.contains("acts on both inputs"), "{wanted}: {error}");
            assert!(error.contains("bare serial"), "{wanted}: {error}");
        }
        let error = choose(&entries, Some("777-2"), Some(Family::Model927)).unwrap_err();
        assert!(error.ends_with(": 9049777"), "{error}");
        // The commands that act on one input still take it.
        let chosen = choose(&entries, Some("9049204-2"), None).unwrap();
        assert_eq!(chosen.name, "9049204-2");
    }

    #[test]
    fn a_box_level_command_with_two_927s_and_no_name_is_refused_with_both() {
        let entries = bus(&[
            (0x0016, "08134079"),
            (0x0019, "9049204"),
            (0x0019, "9049777"),
        ]);
        let error = choose(&entries, None, Some(Family::Model927)).unwrap_err();
        assert!(error.ends_with(": 9049204, 9049777"), "{error}");
        // With no family asked for, nothing asked for is still the first there.
        assert_eq!(choose(&entries, None, None).unwrap().name, "08134079");
        let adapters = bus(&[(0x0016, "08134079"), (0x0016, "08139999")]);
        let chosen = choose(&adapters, None, Some(Family::DpmUsb)).unwrap();
        assert_eq!(chosen.name, "08134079");
    }

    #[test]
    fn the_926_commands_pass_over_a_927() {
        let entries = bus(&[(0x0019, "8888888"), (0x0016, "11217584")]);
        let chosen = choose(&entries, None, Some(Family::DpmUsb)).unwrap();
        assert_eq!(chosen.name, "11217584");
    }

    #[test]
    fn an_input_suffix_is_not_part_of_the_serial_it_is_matched_against() {
        // `--device 1` meant the adapter whose serial has a 1 in it before
        // a 927 was listed, and still does.
        let entries = bus(&[(0x0019, "8888888"), (0x0016, "11217584")]);
        assert_eq!(choose(&entries, Some("1"), None).unwrap().name, "11217584");
        assert!(choose(&entries, Some("-"), None).is_err());
        // Part of a serial with an input after it still finds that input.
        let chosen = choose(&entries, Some("888-2"), None).unwrap();
        assert_eq!(chosen.name, "8888888-2");
    }

    #[test]
    fn a_name_that_is_all_of_one_entry_and_part_of_another_finds_the_whole() {
        // Unnamed devices go by bus and address, and `probe` and `serve N`
        // open each by the name it was listed under.
        let entries = bus(&[(0x0016, "1:12"), (0x0016, "1:1")]);
        assert_eq!(choose(&entries, Some("1:1"), None).unwrap().name, "1:1");
    }

    #[test]
    fn a_927_named_whole_is_never_mistaken_for_one_its_serial_is_part_of() {
        let entries = bus(&[(0x0019, "1:12"), (0x0019, "1:1")]);
        let chosen = choose(&entries, Some("1:1-2"), None).unwrap();
        assert_eq!(chosen.name, "1:1-2");
        // The box-level commands take the bare serial, and it is that box.
        let chosen = choose(&entries, Some("1:1"), Some(Family::Model927)).unwrap();
        assert_eq!(chosen.serial, "1:1");
        let chosen = choose(&entries, Some("1:12"), Some(Family::Model927)).unwrap();
        assert_eq!(chosen.serial, "1:12");
    }

    #[test]
    fn a_bare_927_serial_where_one_input_is_wanted_names_both() {
        let entries = bus(&[(0x0016, "08134079"), (0x0019, "9049204")]);
        let error = choose(&entries, Some("9049204"), None).unwrap_err();
        assert!(error.contains("9049204-1"), "{error}");
        assert!(error.contains("9049204-2"), "{error}");
        // A fragment of it is two inputs as well.
        let error = choose(&entries, Some("9049"), None).unwrap_err();
        assert!(
            error.contains("9049204-1") && error.contains("9049204-2"),
            "{error}"
        );
        // But a box-level command is one box.
        let chosen = choose(&entries, Some("9049"), Some(Family::Model927)).unwrap();
        assert_eq!(chosen.serial, "9049204");
    }

    #[test]
    fn an_empty_name_or_a_bare_input_number_is_refused_with_what_it_matches() {
        let entries = bus(&[(0x0019, "8888888"), (0x0016, "11217584")]);
        let cases = [
            ("", None, ": 8888888-1, 8888888-2, 11217584; name one whole"),
            ("-1", None, ": 8888888-1; name one whole"),
            ("-2", None, ": 8888888-2; name one whole"),
            ("", Some(Family::Model927), ": 8888888; name one whole"),
            ("-1", Some(Family::Model927), "bare serial: 8888888"),
            ("-2", Some(Family::Model927), "bare serial: 8888888"),
        ];
        for (wanted, only, listed) in cases {
            let error = choose(&entries, Some(wanted), only).unwrap_err();
            assert!(error.ends_with(listed), "{wanted:?} {only:?}: {error}");
        }
    }

    #[test]
    fn a_fragment_that_matches_two_entries_is_refused_with_both() {
        let entries = bus(&[(0x0016, "08134079"), (0x0016, "08139999")]);
        let error = choose(&entries, Some("0813"), None).unwrap_err();
        assert!(
            error.contains("08134079") && error.contains("08139999"),
            "{error}"
        );
        let entries = bus(&[(0x0019, "9049204"), (0x0019, "9049777")]);
        let error = choose(&entries, Some("9049-2"), None).unwrap_err();
        assert!(
            error.contains("9049204-2") && error.contains("9049777-2"),
            "{error}"
        );
        let error = choose(&entries, Some("9049"), Some(Family::Model927)).unwrap_err();
        assert!(
            error.contains("9049204") && error.contains("9049777"),
            "{error}"
        );
        // One that matches one still finds it.
        assert_eq!(
            choose(&entries, Some("777-2"), None).unwrap().name,
            "9049777-2"
        );
    }

    #[test]
    fn an_empty_bus_says_what_was_looked_for() {
        let error = choose(&[], None, None).unwrap_err();
        assert_eq!(
            error,
            "no DPM-USB adapter or Model 927 on the bus (looking for 0a2d:0016 and 0a2d:0019)"
        );
        let adapters = bus(&[(0x0016, "08134079")]);
        let error = choose(&adapters, None, Some(Family::Model927)).unwrap_err();
        assert_eq!(error, "no Model 927 on the bus (looking for 0a2d:0019)");
        let error = choose(&adapters, Some("999"), None).unwrap_err();
        assert_eq!(error, "no adapter with \"999\" in its serial number");
    }

    #[test]
    fn a_927_opens_all_four_endpoints_and_an_adapter_its_two() {
        assert_eq!(Family::DpmUsb.kind().out, [0x01]);
        assert_eq!(Family::DpmUsb.kind().input, [0x81]);
        assert_eq!(Family::Model927.kind().out, [0x01, 0x08]);
        assert_eq!(Family::Model927.kind().input, [0x81, 0x82]);
        assert_eq!(Family::of(0x0016), Some(Family::DpmUsb));
        assert_eq!(Family::of(0x0019), Some(Family::Model927));
        assert_eq!(Family::of(0x000F), None);
    }

    #[test]
    fn a_hold_that_panics_still_lets_go() {
        let slot = RefCell::new(None);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            holding(&slot, "endpoints", || panic!("an operation that panics"))
        }));
        assert!(unwound.is_err());
        assert!(
            slot.borrow().is_none(),
            "the claim was kept, and every later hold would be refused"
        );
    }

    #[test]
    fn letting_go_never_panics_on_a_slot_that_is_still_borrowed() {
        // During an unwind a second panic is an abort, so the release must
        // not be able to raise one.
        let slot = RefCell::new(None);
        let ended = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            holding(&slot, "endpoints", || std::mem::forget(slot.borrow()));
        }));
        assert!(ended.is_ok(), "the release panicked");
    }

    #[test]
    fn a_busy_claim_is_asked_for_again_until_it_is_free() {
        let mut attempts = 0;
        let outcome = patiently(
            Duration::from_secs(2),
            Duration::from_millis(1),
            |fault: &&str| *fault == "busy",
            || {
                attempts += 1;
                if attempts < 4 {
                    Err("busy")
                } else {
                    Ok(attempts)
                }
            },
        );
        assert_eq!(outcome, Ok(4));
    }

    #[test]
    fn a_claim_that_stays_busy_gives_up_when_patience_runs_out() {
        let began = Instant::now();
        let mut attempts = 0;
        let outcome: Result<(), &str> = patiently(
            Duration::from_millis(30),
            Duration::from_millis(5),
            |fault: &&str| *fault == "busy",
            || {
                attempts += 1;
                Err("busy")
            },
        );
        assert_eq!(outcome, Err("busy"));
        assert!(attempts > 1, "it was asked more than once");
        assert!(began.elapsed() >= Duration::from_millis(30));
        assert!(
            began.elapsed() < Duration::from_secs(2),
            "and it is bounded"
        );
    }

    #[test]
    fn a_claim_refused_for_any_other_reason_is_not_asked_again() {
        let mut attempts = 0;
        let outcome: Result<(), &str> = patiently(
            Duration::from_secs(2),
            Duration::from_millis(1),
            |fault: &&str| *fault == "busy",
            || {
                attempts += 1;
                Err("permission denied")
            },
        );
        assert_eq!(outcome, Err("permission denied"));
        assert_eq!(attempts, 1);
    }
}
