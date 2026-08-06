//! A real instrument at the far end of a [`Transport`].
//!
//! # The wire dialect
//!
//! One ASCII command out, one line back. This is ortseam's own dialect, not a
//! claim about any manufacturer's: a real ORTEC MCB answers `SHOW_LIVE` with
//! `$G0007663632108` and rejects `SET_PRESET_LIVE` outright. Hardware is
//! reached through `ortseam-mcb`, which translates. What is written here is the
//! contract between ORTSEAM and whatever is at the far end of a
//! [`Transport`]:
//!
//! | Command | Response |
//! |---|---|
//! | `SHOW_CONFIGURATION` | `MODEL=.. SERIAL=.. FIRMWARE=.. CHANNELS=n [CAL=a,b,c]` |
//! | `SHOW_STATUS` | `RT=.. LT=.. DT=..% ICR=.. ACTIVE=0/1 TOTAL=n` |
//! | `SHOW_DATA` | `DATA n c0 c1 ... c(n-1)` |
//! | `START` `STOP` `CLEAR` | `OK` |
//! | `SET_PRESET_REAL s` etc. | `OK` |
//! | anything rejected | `ERR reason` |
//!
//! Everything here is exercised against a scripted transport and against a
//! served [`SimulatedMcb`](crate::SimulatedMcb) in the tests. Real hardware
//! arrives through [`BridgeTransport`](crate::BridgeTransport) speaking the
//! same lines, so this file needs no knowledge of it.

use ortseam_core::{AcquisitionMode, Spectrum};

use crate::error::DeviceError;
use crate::mcb::{Mcb, McbIdentity, McbProperties, McbStatus};
use crate::presets::Presets;
use crate::transport::Transport;

/// How often the remote is asked for fresh numbers, in seconds of poll time.
/// Commands (start, stop, presets) always go straight through.
const POLL_INTERVAL: f64 = 0.5;

/// An instrument reached over a transport, presented as an [`Mcb`].
pub struct RemoteMcb {
    transport: Box<dyn Transport>,
    identity: McbIdentity,
    properties: McbProperties,
    spectrum: Spectrum,
    status: McbStatus,
    mode: AcquisitionMode,
    locked_by: Option<(String, String)>,
    since_poll: f64,
}

impl RemoteMcb {
    /// Connects: asks the instrument what it is, and sizes the local mirror.
    pub fn connect(
        mut transport: Box<dyn Transport>,
        number: u16,
        name: &str,
    ) -> Result<Self, DeviceError> {
        let configuration = checked(transport.exchange("SHOW_CONFIGURATION")?)?;
        let field = |key: &str| -> Option<String> {
            configuration.split_whitespace().find_map(|word| {
                word.strip_prefix(key)
                    .and_then(|rest| rest.strip_prefix('='))
                    .map(str::to_string)
            })
        };
        let channels: usize = field("CHANNELS")
            .and_then(|value| value.parse().ok())
            .unwrap_or(1024);
        let identity = McbIdentity {
            number,
            name: name.to_string(),
            model: field("MODEL").unwrap_or_else(|| "remote MCB".into()),
            serial: field("SERIAL").unwrap_or_default(),
            firmware: field("FIRMWARE").unwrap_or_default(),
            description: format!("network instrument at {}", transport.peer()),
            channels,
            capabilities: Default::default(),
        };
        // An instrument that keeps its calibration host-side can hand it over
        // here; ORTEC's do, through the bridge. Without it the spectrum is
        // honestly uncalibrated rather than wrongly scaled.
        let calibration = field("CAL").and_then(|value| {
            let numbers: Vec<f64> = value
                .split(',')
                .filter_map(|number| number.parse().ok())
                .collect();
            match numbers[..] {
                [a, b, c] => Some(ortseam_core::EnergyCalibration::new([a, b, c], "keV")),
                [a, b] => Some(ortseam_core::EnergyCalibration::linear(a, b)),
                _ => None,
            }
        });
        let mut spectrum = Spectrum::new(channels);
        spectrum.energy_calibration = calibration;
        // The instrument's place in the pick list travels with its spectrum,
        // so a file saved from this window says which detector it came from
        // rather than claiming to be a buffer.
        spectrum.detector_id = number;
        spectrum.detector_name = name.to_string();
        spectrum.detector_description = name.to_string();
        let mut remote = Self {
            transport,
            identity,
            properties: McbProperties::default(),
            spectrum,
            status: McbStatus::default(),
            mode: AcquisitionMode::Pha,
            locked_by: None,
            since_poll: f64::MAX,
        };
        remote.refresh()?;
        Ok(remote)
    }

    /// One command out, one checked line back.
    fn command(&mut self, command: &str) -> Result<String, DeviceError> {
        checked(self.transport.exchange(command)?)
    }

    /// Pulls fresh status and data from the instrument.
    fn refresh(&mut self) -> Result<(), DeviceError> {
        self.since_poll = 0.0;
        let status = self.command("SHOW_STATUS")?;
        let number = |key: &str| -> Option<f64> {
            status.split_whitespace().find_map(|word| {
                word.strip_prefix(key)
                    .and_then(|rest| rest.strip_prefix('='))
                    .map(|value| value.trim_end_matches('%'))
                    .and_then(|value| value.parse().ok())
            })
        };
        self.status.real_time = number("RT").unwrap_or(self.status.real_time);
        self.status.live_time = number("LT").unwrap_or(self.status.live_time);
        self.status.dead_time_percent = number("DT").unwrap_or(0.0);
        self.status.input_count_rate = number("ICR").unwrap_or(0.0);
        self.status.active = number("ACTIVE").unwrap_or(0.0) != 0.0;
        self.status.total_counts = number("TOTAL").unwrap_or(0.0) as u64;

        let data = self.command("SHOW_DATA")?;
        let mut words = data.split_whitespace();
        if words.next() != Some("DATA") {
            return Err(DeviceError::Communication {
                detail: format!("expected DATA, got {data:.40?}"),
            });
        }
        // The count word arrives over a wire and is not to be trusted with an
        // allocation: the largest real MCB memory is 16k channels, so anything
        // past this is a corrupt line, not an instrument. One garbled reply
        // must be an error the caller can show, never an allocation abort and
        // never counts quietly read as zero.
        const MOST_CHANNELS: usize = 65536;
        let count: usize = words
            .next()
            .and_then(|word| word.parse().ok())
            .filter(|count| *count <= MOST_CHANNELS)
            .ok_or_else(|| DeviceError::Communication {
                detail: format!("DATA does not declare a believable channel count: {data:.60?}"),
            })?;
        let mut channels = Vec::with_capacity(count);
        for word in words.take(count) {
            channels.push(
                word.parse::<u64>()
                    .map_err(|_| DeviceError::Communication {
                        detail: format!("a channel in DATA is not a count: {word:.40?}"),
                    })?,
            );
        }
        if channels.len() != count {
            return Err(DeviceError::Communication {
                detail: format!(
                    "DATA declares {count} channels but carries {}",
                    channels.len()
                ),
            });
        }
        if count > 0 {
            if self.spectrum.len() != count {
                // The instrument's conversion gain changed underneath us. The
                // counts are new, but whose spectrum this is - detector,
                // calibration, start date - is not.
                let mut resized = Spectrum::new(count);
                resized.copy_descriptors_from(&self.spectrum);
                self.spectrum = resized;
            }
            self.spectrum.channels.copy_from_slice(&channels);
        }
        self.spectrum.real_time = self.status.real_time;
        self.spectrum.live_time = self.status.live_time;
        Ok(())
    }

    fn ensure_unlocked(&self) -> Result<(), DeviceError> {
        match &self.locked_by {
            Some((_, owner)) => Err(DeviceError::Locked {
                owner: owner.clone(),
            }),
            None => Ok(()),
        }
    }
}

/// Turns an `ERR reason` line into the error it is.
fn checked(response: String) -> Result<String, DeviceError> {
    match response.strip_prefix("ERR") {
        Some(reason) => Err(DeviceError::Command {
            command: String::new(),
            detail: reason.trim().to_string(),
        }),
        None => Ok(response),
    }
}

impl Mcb for RemoteMcb {
    fn identity(&self) -> &McbIdentity {
        &self.identity
    }

    fn properties(&self) -> &McbProperties {
        &self.properties
    }

    fn set_properties(&mut self, properties: McbProperties) -> Result<(), DeviceError> {
        self.ensure_unlocked()?;
        // Push the settings hardware understands; the rest stay in the mirror.
        self.command(&format!(
            "SET_HV {:.0}",
            properties.high_voltage.target_volts
        ))?;
        self.command(&format!(
            "SET_GAIN_COARSE {:.0}",
            properties.amplifier.coarse_gain
        ))?;
        self.set_presets(properties.presets)?;
        self.properties = properties;
        Ok(())
    }

    fn presets_mut(&mut self) -> &mut Presets {
        &mut self.properties.presets
    }

    fn set_presets(&mut self, presets: Presets) -> Result<(), DeviceError> {
        self.ensure_unlocked()?;
        self.command("SET_PRESET_CLEAR")?;
        if let Some(seconds) = presets.real_time {
            self.command(&format!("SET_PRESET_REAL {seconds}"))?;
        }
        if let Some(seconds) = presets.live_time {
            self.command(&format!("SET_PRESET_LIVE {seconds}"))?;
        }
        if let Some(counts) = presets.roi_peak {
            self.command(&format!("SET_PRESET_COUNT {counts}"))?;
        }
        if let Some(counts) = presets.roi_integral {
            self.command(&format!("SET_PRESET_INTEG {counts}"))?;
        }
        self.properties.presets = presets;
        Ok(())
    }

    fn start(&mut self) -> Result<(), DeviceError> {
        self.ensure_unlocked()?;
        self.command("START")?;
        self.status.active = true;
        // The instrument reports no measurement date of its own, so the moment
        // START was accepted is the record - as the simulator keeps it. A
        // resumed count keeps the original date; Clear resets it.
        if self.spectrum.start_time.is_none() {
            self.spectrum.start_time = Some(chrono::Local::now().naive_local());
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), DeviceError> {
        self.command("STOP")?;
        self.status.active = false;
        Ok(())
    }

    fn clear(&mut self) -> Result<(), DeviceError> {
        self.ensure_unlocked()?;
        self.command("CLEAR")?;
        self.spectrum.clear();
        self.status.total_counts = 0;
        Ok(())
    }

    fn is_active(&self) -> bool {
        self.status.active
    }

    fn status(&self) -> McbStatus {
        self.status
    }

    fn spectrum(&self) -> &Spectrum {
        &self.spectrum
    }

    fn spectrum_mut(&mut self) -> &mut Spectrum {
        &mut self.spectrum
    }

    fn poll(&mut self, elapsed_seconds: f64) -> Result<(), DeviceError> {
        self.since_poll += elapsed_seconds;
        if self.since_poll >= POLL_INTERVAL {
            self.refresh()?;
        }
        Ok(())
    }

    fn set_mode(&mut self, mode: AcquisitionMode) -> Result<(), DeviceError> {
        self.ensure_unlocked()?;
        self.mode = mode;
        Ok(())
    }

    fn mode(&self) -> AcquisitionMode {
        self.mode
    }

    fn send_message(&mut self, command: &str) -> Result<String, DeviceError> {
        self.command(command)
    }

    fn lock(&mut self, password: &str, owner: &str) -> Result<(), DeviceError> {
        self.locked_by = Some((password.to_string(), owner.to_string()));
        Ok(())
    }

    fn unlock(&mut self, password: &str) -> Result<(), DeviceError> {
        match &self.locked_by {
            Some((held, _)) if held == password => {
                self.locked_by = None;
                Ok(())
            }
            Some((_, owner)) => Err(DeviceError::Locked {
                owner: owner.clone(),
            }),
            None => Ok(()),
        }
    }

    fn is_locked(&self) -> bool {
        self.locked_by.is_some()
    }
}
