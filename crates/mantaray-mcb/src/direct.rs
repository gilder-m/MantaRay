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

use std::cell::RefCell;
use std::time::Duration;

use nusb::MaybeFuture;
use nusb::transfer::{Bulk, In, Out};

use crate::dpm::BulkDevice;

/// The adapter, as the bus reports it.
pub const VENDOR: u16 = 0x0A2D;
/// The DPM-USB product.
pub const PRODUCT: u16 = 0x0016;

/// The interface the bulk endpoints live on.
const INTERFACE: u8 = 0;
/// Frames to the adapter.
const OUT: u8 = 0x01;
/// Answers back.
const IN: u8 = 0x81;

/// An adapter reached through libusb.
///
/// The endpoints are opened once and kept, because claiming them per transfer
/// would be both slow and a good way to lose a queued answer. They sit behind
/// a `RefCell` so that sending stays a `&self` operation, which is what the
/// protocol layer above expects.
pub struct Device {
    out: RefCell<nusb::Endpoint<Bulk, Out>>,
    input: RefCell<nusb::Endpoint<Bulk, In>>,
    serial: String,
}

/// Every DPM-USB adapter the bus can see.
fn adapters() -> Result<Vec<nusb::DeviceInfo>, String> {
    Ok(nusb::list_devices()
        .wait()
        .map_err(|error| format!("listing USB devices: {error}"))?
        .filter(|device| device.vendor_id() == VENDOR && device.product_id() == PRODUCT)
        .collect())
}

impl Device {
    /// The serial number of every adapter present.
    pub fn list() -> Result<Vec<String>, String> {
        Ok(adapters()?
            .iter()
            .map(|device| {
                device
                    .serial_number()
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{}:{}", device.bus_id(), device.device_address()))
            })
            .collect())
    }

    /// Opens an adapter: the one whose serial contains `wanted`, or the first
    /// one there when nothing is asked for.
    pub fn open(wanted: Option<&str>) -> Result<Self, String> {
        let found = adapters()?;
        if found.is_empty() {
            return Err(format!(
                "no DPM-USB adapter on the bus (looking for {VENDOR:04x}:{PRODUCT:04x})"
            ));
        }
        let chosen = match wanted {
            None => &found[0],
            Some(wanted) => found
                .iter()
                .find(|device| {
                    device
                        .serial_number()
                        .is_some_and(|serial| serial.contains(wanted))
                })
                .ok_or_else(|| format!("no adapter with {wanted:?} in its serial number"))?,
        };
        let serial = chosen
            .serial_number()
            .map(str::to_string)
            .unwrap_or_default();
        let device = chosen
            .open()
            .wait()
            .map_err(|error| format!("opening adapter {serial}: {error}"))?;
        let interface = device.claim_interface(INTERFACE).wait().map_err(|error| {
            // Two different faults arrive here and they need opposite advice.
            // Busy means something already holds the interface - another
            // window of this program, or a kernel driver that bound it - and
            // no udev rule will help. Denied is the permission case. Sending
            // somebody to write a udev rule when the real answer is "you have
            // it open over there" is the more likely of the two now that a
            // bench can carry more than one adapter.
            let advice = match error.kind() {
                nusb::ErrorKind::Busy => {
                    "Something else already has this adapter - another window of this \
                     program, or a driver that has bound it. Close the other one, or \
                     pick a different adapter."
                        .to_string()
                }
                nusb::ErrorKind::PermissionDenied => format!(
                    "This is permission: a udev rule giving {VENDOR:04x}:{PRODUCT:04x} \
                     to your user is what is wanted."
                ),
                nusb::ErrorKind::Disconnected => {
                    "The adapter went away between being listed and being opened.".to_string()
                }
                _ => "The adapter answered, but the interface could not be claimed.".to_string(),
            };
            format!("claiming interface {INTERFACE} of adapter {serial}: {error}. {advice}")
        })?;
        let out = interface
            .endpoint::<Bulk, Out>(OUT)
            .map_err(|error| format!("opening endpoint {OUT:#04x}: {error}"))?;
        let input = interface
            .endpoint::<Bulk, In>(IN)
            .map_err(|error| format!("opening endpoint {IN:#04x}: {error}"))?;
        Ok(Self {
            out: RefCell::new(out),
            input: RefCell::new(input),
            serial,
        })
    }

    /// The adapter's serial number, which is the instrument's identity here.
    pub fn serial(&self) -> &str {
        &self.serial
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
        if endpoint & 0x80 == 0 {
            let mut out = self.out.borrow_mut();
            let done = out.transfer_blocking(data.to_vec().into(), timeout);
            match done.status {
                Ok(()) => Ok(done.actual_len),
                Err(TransferError::Cancelled) => Err(format!(
                    "the adapter did not accept a frame within {milliseconds} ms"
                )),
                Err(error) => Err(format!("writing to endpoint {endpoint:#04x}: {error}")),
            }
        } else {
            let mut input = self.input.borrow_mut();
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
                Err(error) => Err(format!("reading from endpoint {endpoint:#04x}: {error}")),
            }
        }
    }
}
