//! Talking to the instrument's USB driver directly, with no ORTEC software.
//!
//! ORTEC's `DpmUsbAddIn.dll` reaches the DPM-USB adapter through Cypress's
//! CyAPI, which is a thin wrapper over `DeviceIoControl` against the driver's
//! own interface. The driver is ORTEC's build of Cypress's `CyUSB`, so the
//! control codes are Cypress's published ones, and nothing here needs a vendor
//! library: the device is opened by name and the same requests are sent.
//!
//! This is the first half of doing without ORTEC's user-mode stack. The second
//! half is the mailbox protocol the ASCII dialect rides on, which is not yet
//! worked out - see `docs/ortec-hardware.md`.
//!
//! Only the pieces actually used are declared. Everything is `#[cfg(windows)]`,
//! because on Linux and macOS the same device is reached through libusb with no
//! driver at all, which will be its own module.

#![cfg(windows)]

use std::ffi::{CString, c_void};

/// The device interface ORTEC's driver registers, from `ortecusb3.inf`.
pub const ORTECUSB_INTERFACE: &str = "{02c10ba3-54e3-4bb0-9bbf-a848ae18116a}";

/// Cypress's control codes, from `cyioctl.h`.
///
/// `CTL_CODE(FILE_DEVICE_UNKNOWN, IOCTL_ADAPT_INDEX + n, METHOD_BUFFERED,
/// FILE_ANY_ACCESS)` with an index base of zero, which works out as
/// `0x0022_0000 | (n << 2)`. The numbering was checked against the driver
/// itself - its handler names survive in the binary in this order - and
/// against ORTEC's `DpmUsbAddIn.dll`, which sends exactly these values.
#[allow(dead_code)]
mod ioctl {
    pub const GET_NUMBER_ENDPOINTS: u32 = 0x0022_0014;
    pub const SEND_EP0_CONTROL_TRANSFER: u32 = 0x0022_0020;
    pub const SEND_NON_EP0_TRANSFER: u32 = 0x0022_0024;
    pub const CYCLE_PORT: u32 = 0x0022_0028;
    pub const RESET_PIPE: u32 = 0x0022_002C;
    pub const RESET_PARENT_PORT: u32 = 0x0022_0030;
    pub const GET_DEVICE_NAME: u32 = 0x0022_003C;
    pub const GET_FRIENDLY_NAME: u32 = 0x0022_0040;
    pub const ABORT_PIPE: u32 = 0x0022_0044;
    pub const GET_DEVICE_SPEED: u32 = 0x0022_004C;
}

type Handle = *mut c_void;
type Bool32 = i32;

const INVALID_HANDLE: Handle = usize::MAX as Handle;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_READ: u32 = 1;
const FILE_SHARE_WRITE: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
const ERROR_IO_PENDING: u32 = 997;

const HKEY_LOCAL_MACHINE: Handle = 0x8000_0002_u32 as usize as Handle;
const KEY_READ: u32 = 0x2_0019;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateFileA(
        name: *const i8,
        access: u32,
        share: u32,
        security: *mut c_void,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    fn CloseHandle(handle: Handle) -> Bool32;
    fn CreateEventA(
        security: *mut c_void,
        manual_reset: Bool32,
        initial: Bool32,
        name: *const i8,
    ) -> Handle;
    fn GetOverlappedResultEx(
        handle: Handle,
        overlapped: *mut c_void,
        transferred: *mut u32,
        milliseconds: u32,
        alertable: Bool32,
    ) -> Bool32;
    fn CancelIoEx(handle: Handle, overlapped: *mut c_void) -> Bool32;
    fn DeviceIoControl(
        handle: Handle,
        code: u32,
        in_buffer: *mut c_void,
        in_size: u32,
        out_buffer: *mut c_void,
        out_size: u32,
        returned: *mut u32,
        overlapped: *mut c_void,
    ) -> Bool32;
    fn GetLastError() -> u32;
}

#[link(name = "advapi32")]
unsafe extern "system" {
    fn RegOpenKeyExA(key: Handle, sub: *const i8, options: u32, sam: u32, out: *mut Handle) -> i32;
    fn RegEnumKeyExA(
        key: Handle,
        index: u32,
        name: *mut i8,
        name_len: *mut u32,
        reserved: *mut u32,
        class: *mut i8,
        class_len: *mut u32,
        written: *mut c_void,
    ) -> i32;
    fn RegCloseKey(key: Handle) -> i32;
}

/// Every instrument of ours the driver has bound, as paths to open.
///
/// Windows records each bound interface under `DeviceClasses`, keyed by the
/// interface GUID, and the subkey name is the device's symbolic link with the
/// separators written as `#`. Turning it back is the whole of what SetupAPI
/// would do here, and reading four registry keys is less to get wrong than the
/// enumeration dance.
pub fn devices() -> Vec<String> {
    let mut found = Vec::new();
    let path = format!("SYSTEM\\CurrentControlSet\\Control\\DeviceClasses\\{ORTECUSB_INTERFACE}");
    let Ok(path) = CString::new(path) else {
        return found;
    };
    // SAFETY: the key is opened read-only and closed below; every buffer passed
    // to RegEnumKeyExA carries its own length.
    unsafe {
        let mut key: Handle = std::ptr::null_mut();
        if RegOpenKeyExA(HKEY_LOCAL_MACHINE, path.as_ptr(), 0, KEY_READ, &mut key) != 0 {
            return found;
        }
        let mut index = 0;
        loop {
            let mut name = vec![0_i8; 512];
            let mut length = name.len() as u32;
            let status = RegEnumKeyExA(
                key,
                index,
                name.as_mut_ptr(),
                &mut length,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            if status != 0 {
                break;
            }
            index += 1;
            let bytes: Vec<u8> = name[..length as usize].iter().map(|c| *c as u8).collect();
            let Ok(text) = String::from_utf8(bytes) else {
                continue;
            };
            // "##?#USB#VID_0A2D&PID_0016#11217584#{guid}" is the link
            // "\\?\USB#VID_0A2D&PID_0016#11217584#{guid}" with the leading
            // separators escaped. Only the first three are separators; the
            // rest belong to the name.
            if let Some(rest) = text.strip_prefix("##?#") {
                found.push(format!("\\\\?\\{rest}"));
            }
        }
        RegCloseKey(key);
    }
    found
}

/// Whether a request that times out should try to clear the pipes afterwards.
///
/// The pipe-clearing requests themselves must not, or a wedged adapter sends
/// the code round in circles instead of reporting the problem.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Recover {
    Yes,
    No,
}

/// An open handle on one instrument's driver.
pub struct Device {
    handle: Handle,
    /// The event each overlapped request waits on. One per device is enough
    /// because requests are made one at a time.
    event: Handle,
    /// The path this was opened by, for reopening after a port cycle.
    path: String,
}

impl Device {
    /// Opens a device by the path [`devices`] reported.
    pub fn open(path: &str) -> Result<Self, String> {
        let name = CString::new(path).map_err(|_| "the device path has a NUL in it")?;
        // SAFETY: a null security descriptor and no template handle are the
        // documented defaults; the handle is closed on drop.
        let handle = unsafe {
            CreateFileA(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                // Transfers need this: the driver queues them and answers later.
                // The catch is that an overlapped handle makes a
                // DeviceIoControl with a null OVERLAPPED return before the
                // driver has finished with the buffer, so every call below
                // passes one and waits on it.
                FILE_FLAG_OVERLAPPED,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE || handle.is_null() {
            // SAFETY: reads a thread-local error code.
            let error = unsafe { GetLastError() };
            return Err(match error {
                // The driver allows one owner at a time, and ORTEC's library
                // may already be that owner.
                32 => format!("{path} is already open in another program"),
                2 => format!("{path} is not there any more"),
                _ => format!("could not open {path} (Windows error {error})"),
            });
        }
        // SAFETY: a manual-reset event, unnamed, initially unset; closed on drop.
        let event = unsafe { CreateEventA(std::ptr::null_mut(), 1, 0, std::ptr::null()) };
        if event.is_null() {
            // SAFETY: the handle was opened above and is closed exactly once.
            unsafe { CloseHandle(handle) };
            return Err("could not make an event to wait on".into());
        }
        let device = Self {
            handle,
            event,
            path: path.to_string(),
        };
        Ok(device)
    }

    /// The path this device was opened by.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Sends one request and waits for the driver to finish with it.
    ///
    /// Everything goes through the overlapped path with a wait, because the
    /// driver queues transfers and answers later; a call with no `OVERLAPPED`
    /// would come back before the buffers were safe to read, and a plain
    /// synchronous handle hangs on a transfer that has to wait for the
    /// instrument.
    fn control(&self, code: u32, input: &mut [u8], output: &mut [u8]) -> Result<usize, String> {
        self.control_with_timeout(code, input, output, 5000)
    }

    /// The same request, with one buffer serving as both input and output.
    fn control_both(
        &self,
        code: u32,
        buffer: &mut [u8],
        milliseconds: u32,
    ) -> Result<usize, String> {
        let mut overlapped = [0_usize; 5];
        overlapped[4] = self.event as usize;
        let mut returned = 0_u32;
        let pointer: *mut c_void = buffer.as_mut_ptr().cast();
        let length = buffer.len() as u32;
        // SAFETY: the same buffer is given for both directions, which is how a
        // buffered control code works; it outlives the wait below.
        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                code,
                pointer,
                length,
                pointer,
                length,
                &mut returned,
                overlapped.as_mut_ptr().cast(),
            )
        };
        self.finish(
            ok,
            &mut overlapped,
            &mut returned,
            milliseconds,
            Recover::Yes,
        )
    }

    fn control_with_timeout(
        &self,
        code: u32,
        input: &mut [u8],
        output: &mut [u8],
        milliseconds: u32,
    ) -> Result<usize, String> {
        self.request(code, input, output, milliseconds, Recover::Yes)
    }

    fn request(
        &self,
        code: u32,
        input: &mut [u8],
        output: &mut [u8],
        milliseconds: u32,
        recover: Recover,
    ) -> Result<usize, String> {
        // OVERLAPPED is Internal, InternalHigh, Offset, OffsetHigh, hEvent -
        // three pointer-sized words and two DWORDs on 32-bit, which is five
        // words either way. The event goes in the last one.
        let mut overlapped = [0_usize; 5];
        overlapped[4] = self.event as usize;
        let mut returned = 0_u32;
        // SAFETY: both buffers are passed with their own lengths, and the
        // overlapped structure and its event outlive the wait below.
        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                code,
                input.as_mut_ptr().cast(),
                input.len() as u32,
                output.as_mut_ptr().cast(),
                output.len() as u32,
                &mut returned,
                overlapped.as_mut_ptr().cast(),
            )
        };
        self.finish(ok, &mut overlapped, &mut returned, milliseconds, recover)
    }

    /// Waits for a queued request, or withdraws it.
    ///
    /// A request the driver has taken but not finished must be cancelled before
    /// the buffers it was given go out of scope, which is the whole reason this
    /// is not a plain synchronous call.
    fn finish(
        &self,
        ok: Bool32,
        overlapped: &mut [usize; 5],
        returned: &mut u32,
        milliseconds: u32,
        recover: Recover,
    ) -> Result<usize, String> {
        if ok == 0 {
            // SAFETY: reads a thread-local error code.
            let error = unsafe { GetLastError() };
            if error == ERROR_IO_PENDING {
                // SAFETY: the overlapped structure is alive for the whole wait.
                let finished = unsafe {
                    GetOverlappedResultEx(
                        self.handle,
                        overlapped.as_mut_ptr().cast(),
                        returned,
                        milliseconds,
                        0,
                    )
                };
                if finished == 0 {
                    // SAFETY: withdrawing the request is what makes it safe to
                    // let the buffers go.
                    unsafe {
                        CancelIoEx(self.handle, overlapped.as_mut_ptr().cast());
                        GetOverlappedResultEx(
                            self.handle,
                            overlapped.as_mut_ptr().cast(),
                            returned,
                            1000,
                            0,
                        );
                    }
                    // The withdrawn request may have left an answer nobody
                    // collected. Clearing both pipes here means the next
                    // exchange starts clean rather than reading this one's
                    // reply, which is a far more confusing failure than a
                    // timeout - every answer correct but for the wrong question.
                    if recover == Recover::Yes {
                        self.settle();
                    }
                    return Err(format!(
                        "the instrument did not answer within {milliseconds} ms"
                    ));
                }
                return Ok(*returned as usize);
            }
            // 122 is "the buffer is too small", which this driver also returns
            // for a request it does not implement - it is a 2017 build, and
            // some of the published codes came later.
            if error == 122 {
                return Err("not supported by this driver".to_string());
            }
            return Err(format!("the request failed (Windows error {error})"));
        }
        Ok(*returned as usize)
    }

    /// The driver's name for this device.
    pub fn name(&self) -> Result<String, String> {
        self.text(ioctl::GET_DEVICE_NAME)
    }

    /// The name a person would recognise, when the driver has one.
    pub fn friendly_name(&self) -> Result<String, String> {
        self.text(ioctl::GET_FRIENDLY_NAME)
    }

    fn text(&self, code: u32) -> Result<String, String> {
        let mut output = vec![0_u8; 256];
        let returned = self.control(code, &mut [], &mut output)?;
        let bytes = &output[..returned];
        let end = bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(bytes.len());
        Ok(String::from_utf8_lossy(&bytes[..end]).to_string())
    }

    /// How many endpoints the current interface setting has, not counting
    /// endpoint zero.
    pub fn endpoint_count(&self) -> Result<u32, String> {
        let mut setting = [0_u8; 1];
        let mut output = [0_u8; 32];
        let returned = self.control(ioctl::GET_NUMBER_ENDPOINTS, &mut setting, &mut output)?;
        Ok(match returned {
            0 => return Err("the driver reported no endpoint count".into()),
            1 => u32::from(output[0]),
            _ => u32::from_le_bytes([output[0], output[1], output[2], output[3]]),
        })
    }

    /// 0 low, 1 full, 2 high, 3 super - the driver's own numbering.
    pub fn speed(&self) -> Result<u32, String> {
        self.word(ioctl::GET_DEVICE_SPEED)
    }

    /// Throws away whatever a pipe was in the middle of.
    ///
    /// A transfer that timed out leaves the driver holding a request and the
    /// adapter holding an answer nobody collected, and every later exchange
    /// then reads the previous one's reply - one step behind for ever. Without
    /// this the only cure is unplugging the instrument, which is not a thing to
    /// ask of anyone.
    pub fn abort_pipe(&self, endpoint: u8) -> Result<(), String> {
        self.pipe(ioctl::ABORT_PIPE, endpoint)
    }

    /// Puts a pipe back to the start, clearing a stall and the data toggle.
    pub fn reset_pipe(&self, endpoint: u8) -> Result<(), String> {
        self.pipe(ioctl::RESET_PIPE, endpoint)
    }

    /// Unplugs and replugs the device, in software.
    ///
    /// The port is cycled, the device re-enumerates and its firmware starts
    /// over. This is the cure for an adapter whose own state machine is stuck -
    /// the state pipe resets cannot reach - and it is what the cable-pull it
    /// replaces actually did. The handle is dead afterwards; the caller waits
    /// and opens the device afresh.
    pub fn cycle_port(&self) -> Result<(), String> {
        self.request(ioctl::CYCLE_PORT, &mut [], &mut [], 10_000, Recover::No)?;
        Ok(())
    }

    fn pipe(&self, code: u32, endpoint: u8) -> Result<(), String> {
        let mut which = [endpoint];
        // Recovery is off here: this *is* the recovery, and a reset that timed
        // out must not ask for another one.
        self.request(code, &mut which, &mut [], 2000, Recover::No)?;
        Ok(())
    }

    /// Brings both bulk endpoints back to a known state.
    ///
    /// Done when a device is opened and after any transfer that timed out, so a
    /// session that ended badly - a timeout, or a program killed mid-transfer -
    /// does not poison the next one.
    ///
    /// Resetting the pipes is not enough on its own. The adapter answers
    /// whatever it was last asked whether or not anyone is still listening, so
    /// an abandoned request leaves a reply queued; the next exchange collects
    /// that one and every answer afterwards is one question behind. Reading
    /// until nothing more comes back is what actually restores the sequence,
    /// and it is why a wedged instrument no longer has to be unplugged.
    pub fn settle(&self) {
        for endpoint in [0x01, 0x81] {
            let _ = self.abort_pipe(endpoint);
            let _ = self.reset_pipe(endpoint);
        }
        // A short wait, because a pipe with nothing to say should say so at
        // once; anything slower is the instrument thinking, not a stale answer.
        let mut bin = [0_u8; 256];
        for _ in 0..8 {
            match self.bulk_with(0x81, &mut bin, 120, Recover::No) {
                Ok(0) | Err(_) => break,
                Ok(_) => continue,
            }
        }
    }

    /// One four-byte answer, asked for the way ORTEC's own code asks.
    ///
    /// The buffer is exactly four bytes and serves as both input and output.
    /// A larger one is not merely wasteful: the driver copies back only what it
    /// was told the request holds, and these codes are declared as taking a
    /// `ULONG` either way.
    fn word(&self, code: u32) -> Result<u32, String> {
        let mut buffer = [0_u8; 4];
        let returned = self.control_both(code, &mut buffer, 5000)?;
        if returned == 0 {
            return Err("the driver gave no answer".into());
        }
        Ok(u32::from_le_bytes(buffer))
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        // SAFETY: both handles were made in `open` and are closed exactly once.
        unsafe {
            if !self.event.is_null() {
                CloseHandle(self.event);
            }
            if self.handle != INVALID_HANDLE && !self.handle.is_null() {
                CloseHandle(self.handle);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registry_key_name_becomes_a_path_that_can_be_opened() {
        // Windows writes the symbolic link with its separators escaped; this is
        // the exact key name observed for the instrument on the bench.
        let key = "##?#USB#VID_0A2D&PID_0016#11217584#{02c10ba3-54e3-4bb0-9bbf-a848ae18116a}";
        let rest = key.strip_prefix("##?#").expect("the escaped prefix");
        assert_eq!(
            format!("\\\\?\\{rest}"),
            "\\\\?\\USB#VID_0A2D&PID_0016#11217584#{02c10ba3-54e3-4bb0-9bbf-a848ae18116a}"
        );
    }

    #[test]
    fn the_control_codes_are_the_published_ones() {
        // CTL_CODE(FILE_DEVICE_UNKNOWN, n, METHOD_BUFFERED, FILE_ANY_ACCESS)
        // is 0x22 << 16 | n << 2, which is worth checking rather than trusting
        // a transcription.
        let code = |n: u32| (0x22_u32 << 16) | (n << 2);
        assert_eq!(ioctl::GET_NUMBER_ENDPOINTS, code(5));
        assert_eq!(ioctl::SEND_EP0_CONTROL_TRANSFER, code(8));
        assert_eq!(ioctl::SEND_NON_EP0_TRANSFER, code(9));
        assert_eq!(ioctl::RESET_PIPE, code(11));
        assert_eq!(ioctl::GET_DEVICE_NAME, code(15));
        assert_eq!(ioctl::ABORT_PIPE, code(17));
        assert_eq!(ioctl::GET_DEVICE_SPEED, code(19));
    }

    #[test]
    fn the_transfer_header_is_laid_out_the_way_the_driver_reads_it() {
        // Every field is a byte or a little-endian word packed with no padding,
        // in this order: the eight-byte setup packet, a timeout in seconds, a
        // wait-forever flag, the endpoint, then NtStatus, UsbdStatus, the two
        // isochronous fields and the two buffer fields.
        let mut at = 0;
        let mut field = |size: usize| {
            let here = at;
            at += size;
            here
        };
        assert_eq!(transfer::REQUEST_TYPE, field(1));
        assert_eq!(transfer::REQUEST, field(1));
        assert_eq!(transfer::VALUE, field(2));
        assert_eq!(transfer::INDEX, field(2));
        assert_eq!(transfer::LENGTH, field(2));
        assert_eq!(transfer::TIMEOUT, field(4));
        let _wait_forever = field(1);
        assert_eq!(transfer::ENDPOINT, field(1));
        let _nt_status = field(4);
        let _usbd_status = field(4);
        let _iso_packet_offset = field(4);
        let _iso_packet_length = field(4);
        assert_eq!(transfer::BUFFER_OFFSET, field(4));
        assert_eq!(transfer::BUFFER_LENGTH, field(4));
        // Thirty-eight, which is what ORTEC's own library allocates. Making this
        // forty - the size it would be with the usual four-byte alignment - is
        // what stopped every transfer completing, and the failure was a silent
        // timeout rather than an error.
        assert_eq!(SINGLE_TRANSFER_LEN, at);
        assert_eq!(SINGLE_TRANSFER_LEN, 38);
    }
}

/// The header `SEND_EP0_CONTROL_TRANSFER` and `SEND_NON_EP0_TRANSFER` expect.
///
/// Thirty-eight bytes, then the payload. `BufferOffset` counts from the start of
/// the header, so a payload placed directly after it is at thirty-eight.
///
/// The size matters more than it looks: get it wrong and the driver reads
/// `BufferOffset` and `BufferLength` out of the wrong bytes, asks the controller
/// for a transfer that cannot happen, and every request simply never completes.
/// Two bytes too many is what kept this silent for a long time. The layout below
/// is not transcribed from a header but read out of ORTEC's own `DpmUsbAddIn`,
/// which allocates `0x26` bytes for the header and writes `BufferOffset` at
/// `0x1e` and `BufferLength` at `0x22` - see `docs/ortec-hardware.md`.
const SINGLE_TRANSFER_LEN: usize = 0x26;

/// Where each field sits, so the arithmetic is written once.
mod transfer {
    pub const REQUEST_TYPE: usize = 0;
    pub const REQUEST: usize = 1;
    pub const VALUE: usize = 2;
    pub const INDEX: usize = 4;
    pub const LENGTH: usize = 6;
    /// Seconds, not milliseconds, and zero means the driver does not time out.
    pub const TIMEOUT: usize = 8;
    pub const ENDPOINT: usize = 13;
    pub const BUFFER_OFFSET: usize = 0x1e;
    pub const BUFFER_LENGTH: usize = 0x22;
}

impl Device {
    /// Sends a standard control request on endpoint zero and returns the reply.
    ///
    /// The driver takes one buffer holding the header and the payload together,
    /// and writes the reply back into the payload part of that same buffer.
    pub fn control_transfer(
        &self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
    ) -> Result<Vec<u8>, String> {
        let mut packet = vec![0_u8; SINGLE_TRANSFER_LEN + length as usize];
        packet[transfer::REQUEST_TYPE] = request_type;
        packet[transfer::REQUEST] = request;
        packet[transfer::VALUE..transfer::VALUE + 2].copy_from_slice(&value.to_le_bytes());
        packet[transfer::INDEX..transfer::INDEX + 2].copy_from_slice(&index.to_le_bytes());
        packet[transfer::LENGTH..transfer::LENGTH + 2].copy_from_slice(&length.to_le_bytes());
        packet[transfer::TIMEOUT..transfer::TIMEOUT + 4].copy_from_slice(&5_u32.to_le_bytes());
        packet[transfer::ENDPOINT] = 0;
        packet[transfer::BUFFER_OFFSET..transfer::BUFFER_OFFSET + 4]
            .copy_from_slice(&(SINGLE_TRANSFER_LEN as u32).to_le_bytes());
        packet[transfer::BUFFER_LENGTH..transfer::BUFFER_LENGTH + 4]
            .copy_from_slice(&u32::from(length).to_le_bytes());

        let mut reply = packet.clone();
        let returned = self.control(ioctl::SEND_EP0_CONTROL_TRANSFER, &mut packet, &mut reply)?;
        if returned <= SINGLE_TRANSFER_LEN {
            return Ok(Vec::new());
        }
        Ok(reply[SINGLE_TRANSFER_LEN..returned].to_vec())
    }

    /// A bulk transfer on one endpoint.
    ///
    /// This is how the instrument is actually spoken to: the capture shows every
    /// real exchange on the two bulk endpoints and none on endpoint zero. `data`
    /// carries what to send for an out endpoint, and receives what came back for an
    /// in one; the returned length says how much of it is meaningful.
    pub fn bulk(&self, endpoint: u8, data: &mut [u8], milliseconds: u32) -> Result<usize, String> {
        self.bulk_with(endpoint, data, milliseconds, Recover::Yes)
    }

    fn bulk_with(
        &self,
        endpoint: u8,
        data: &mut [u8],
        milliseconds: u32,
        recover: Recover,
    ) -> Result<usize, String> {
        let mut packet = vec![0_u8; SINGLE_TRANSFER_LEN + data.len()];
        // The driver's own timeout is left at zero, as ORTEC leaves it: a bulk
        // read waits for the instrument to have something to say, and the bound
        // that matters is the one kept below, where the request can also be
        // withdrawn cleanly.
        packet[transfer::ENDPOINT] = endpoint;
        packet[transfer::BUFFER_OFFSET..transfer::BUFFER_OFFSET + 4]
            .copy_from_slice(&(SINGLE_TRANSFER_LEN as u32).to_le_bytes());
        packet[transfer::BUFFER_LENGTH..transfer::BUFFER_LENGTH + 4]
            .copy_from_slice(&(data.len() as u32).to_le_bytes());
        packet[SINGLE_TRANSFER_LEN..].copy_from_slice(data);

        // One buffer serving both directions, which is what CyAPI does and what
        // a buffered control code expects: Windows makes a single system
        // buffer, the driver reads the header out of it and writes the result
        // back into the same place.
        let mut overlapped = [0_usize; 5];
        overlapped[4] = self.event as usize;
        let mut got = 0_u32;
        let pointer: *mut c_void = packet.as_mut_ptr().cast();
        let length = packet.len() as u32;
        // One buffer serving both directions, which is what CyAPI does and what
        // a buffered control code expects: Windows makes a single system
        // buffer, the driver reads the header out of it and writes the result
        // back into the same place.
        // SAFETY: the buffer outlives the wait, which either completes the
        // request or withdraws it.
        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                ioctl::SEND_NON_EP0_TRANSFER,
                pointer,
                length,
                pointer,
                length,
                &mut got,
                overlapped.as_mut_ptr().cast(),
            )
        };
        let returned = self.finish(ok, &mut overlapped, &mut got, milliseconds + 1000, recover)?;
        let carried = returned.saturating_sub(SINGLE_TRANSFER_LEN).min(data.len());
        data[..carried]
            .copy_from_slice(&packet[SINGLE_TRANSFER_LEN..SINGLE_TRANSFER_LEN + carried]);
        Ok(carried)
    }

    /// The device descriptor, as the instrument reports itself.
    #[allow(dead_code)]
    pub fn device_descriptor(&self) -> Result<Vec<u8>, String> {
        // 0x80 device-to-host, 0x06 GET_DESCRIPTOR, descriptor type 1.
        self.control_transfer(0x80, 0x06, 0x0100, 0, 18)
    }

    /// The configuration descriptor and everything that follows it, which is
    /// where the endpoints are described.
    pub fn configuration_descriptor(&self) -> Result<Vec<u8>, String> {
        let head = self.control_transfer(0x80, 0x06, 0x0200, 0, 9)?;
        if head.len() < 4 {
            return Err("the configuration descriptor came back too short".into());
        }
        let total = u16::from_le_bytes([head[2], head[3]]);
        self.control_transfer(0x80, 0x06, 0x0200, 0, total)
    }
}

/// One endpoint, as its descriptor describes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Endpoint {
    /// Address, with bit 7 set for device-to-host.
    pub address: u8,
    /// 0 control, 1 isochronous, 2 bulk, 3 interrupt.
    pub kind: u8,
    /// Largest packet the endpoint will carry.
    pub max_packet: u16,
}

impl Endpoint {
    /// Whether this endpoint carries data from the instrument.
    pub fn is_in(&self) -> bool {
        self.address & 0x80 != 0
    }

    /// The transfer type, in a word.
    pub fn kind_name(&self) -> &'static str {
        match self.kind & 3 {
            0 => "control",
            1 => "isochronous",
            2 => "bulk",
            _ => "interrupt",
        }
    }
}

/// Walks a configuration descriptor and picks out the endpoints.
///
/// A descriptor block is a chain of records, each beginning with its own length
/// and type, so unknown records are stepped over rather than guessed at.
pub fn endpoints(descriptor: &[u8]) -> Vec<Endpoint> {
    let mut found = Vec::new();
    let mut at = 0;
    while at + 2 <= descriptor.len() {
        let length = descriptor[at] as usize;
        let kind = descriptor[at + 1];
        if length < 2 || at + length > descriptor.len() {
            break;
        }
        // 0x05 is an endpoint descriptor: address, attributes, max packet.
        if kind == 0x05 && length >= 6 {
            found.push(Endpoint {
                address: descriptor[at + 2],
                kind: descriptor[at + 3],
                max_packet: u16::from_le_bytes([descriptor[at + 4], descriptor[at + 5]]),
            });
        }
        at += length;
    }
    found
}
