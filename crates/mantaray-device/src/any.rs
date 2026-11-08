//! One name for every kind of instrument the application can hold.
//!
//! The GUI keeps a plain `Vec<AnyMcb>`: the simulator and a network instrument
//! sit side by side in the pick list and take the same commands. Simulator-only
//! affordances (the source editor, the sample-changer lines) ask politely with
//! [`AnyMcb::as_simulated_mut`] and disable themselves when the answer is no.

use mantaray_core::{AcquisitionMode, Spectrum};

use crate::error::DeviceError;
use crate::mcb::{Admitted, Mcb, McbIdentity, McbProperties, McbStatus};
use crate::presets::Presets;
use crate::remote::RemoteMcb;
use crate::simulator::SimulatedMcb;

/// A simulated or remote instrument, as one type.
///
/// The simulator is much the larger variant, but these live one per detector in
/// a short Vec - boxing would buy an indirection on every spectrum access and
/// save almost nothing.
#[allow(clippy::large_enum_variant)]
pub enum AnyMcb {
    /// The built-in physics simulator.
    Simulated(SimulatedMcb),
    /// A real instrument over a transport.
    Remote(RemoteMcb),
}

impl AnyMcb {
    /// The simulator inside, when this is one.
    pub fn as_simulated(&self) -> Option<&SimulatedMcb> {
        match self {
            Self::Simulated(mcb) => Some(mcb),
            Self::Remote(_) => None,
        }
    }

    /// The simulator inside, mutably, when this is one.
    pub fn as_simulated_mut(&mut self) -> Option<&mut SimulatedMcb> {
        match self {
            Self::Simulated(mcb) => Some(mcb),
            Self::Remote(_) => None,
        }
    }

    /// The instrument as the trait, for the few generic call sites.
    pub fn as_mcb_mut(&mut self) -> &mut dyn Mcb {
        match self {
            Self::Simulated(mcb) => mcb,
            Self::Remote(mcb) => mcb,
        }
    }
}

impl From<SimulatedMcb> for AnyMcb {
    fn from(mcb: SimulatedMcb) -> Self {
        Self::Simulated(mcb)
    }
}

impl From<RemoteMcb> for AnyMcb {
    fn from(mcb: RemoteMcb) -> Self {
        Self::Remote(mcb)
    }
}

macro_rules! delegate {
    ($self:ident, $mcb:ident => $body:expr) => {
        match $self {
            AnyMcb::Simulated($mcb) => $body,
            AnyMcb::Remote($mcb) => $body,
        }
    };
}

/// Only the backends' own methods are passed on: the accessors, the readings
/// and the `do_*` steps. Every command method (start, stop, presets,
/// settings, raw messages) is the trait's provided one, so a command to an
/// `AnyMcb` passes the same gate as one to the backend itself.
impl Mcb for AnyMcb {
    fn identity(&self) -> &McbIdentity {
        delegate!(self, mcb => mcb.identity())
    }

    fn set_name(&mut self, name: &str) {
        delegate!(self, mcb => mcb.set_name(name))
    }

    fn properties(&self) -> &McbProperties {
        delegate!(self, mcb => mcb.properties())
    }

    fn do_set_properties(
        &mut self,
        properties: McbProperties,
        admitted: Admitted,
    ) -> Result<(), DeviceError> {
        delegate!(self, mcb => mcb.do_set_properties(properties, admitted))
    }

    fn do_set_high_voltage(&mut self, volts: f64, admitted: Admitted) -> Result<(), DeviceError> {
        delegate!(self, mcb => mcb.do_set_high_voltage(volts, admitted))
    }

    fn do_set_coarse_gain(&mut self, gain: f64, admitted: Admitted) -> Result<(), DeviceError> {
        delegate!(self, mcb => mcb.do_set_coarse_gain(gain, admitted))
    }

    fn do_write_presets(
        &mut self,
        presets: Presets,
        admitted: Admitted,
    ) -> Result<(), DeviceError> {
        delegate!(self, mcb => mcb.do_write_presets(presets, admitted))
    }

    fn do_store_host_presets(
        &mut self,
        presets: Presets,
        admitted: Admitted,
    ) -> Result<(), DeviceError> {
        delegate!(self, mcb => mcb.do_store_host_presets(presets, admitted))
    }

    fn read_counting(&mut self) -> Result<bool, DeviceError> {
        delegate!(self, mcb => mcb.read_counting())
    }

    fn ensure_unlocked(&self) -> Result<(), DeviceError> {
        delegate!(self, mcb => mcb.ensure_unlocked())
    }

    fn do_start(&mut self, admitted: Admitted) -> Result<(), DeviceError> {
        delegate!(self, mcb => mcb.do_start(admitted))
    }

    fn do_stop(&mut self, admitted: Admitted) -> Result<(), DeviceError> {
        delegate!(self, mcb => mcb.do_stop(admitted))
    }

    fn do_clear(&mut self, admitted: Admitted) -> Result<(), DeviceError> {
        delegate!(self, mcb => mcb.do_clear(admitted))
    }

    fn is_active(&self) -> bool {
        delegate!(self, mcb => mcb.is_active())
    }

    fn status(&self) -> McbStatus {
        delegate!(self, mcb => mcb.status())
    }

    fn spectrum(&self) -> &Spectrum {
        delegate!(self, mcb => mcb.spectrum())
    }

    fn spectrum_mut(&mut self) -> &mut Spectrum {
        delegate!(self, mcb => mcb.spectrum_mut())
    }

    fn poll(&mut self, elapsed_seconds: f64) -> Result<(), DeviceError> {
        delegate!(self, mcb => mcb.poll(elapsed_seconds))
    }

    fn poll_freshened(&self) -> bool {
        delegate!(self, mcb => mcb.poll_freshened())
    }

    fn do_set_mode(
        &mut self,
        mode: AcquisitionMode,
        admitted: Admitted,
    ) -> Result<(), DeviceError> {
        delegate!(self, mcb => mcb.do_set_mode(mode, admitted))
    }

    fn mode(&self) -> AcquisitionMode {
        delegate!(self, mcb => mcb.mode())
    }

    fn list_data(&self) -> Option<&mantaray_formats::ListModeFile> {
        delegate!(self, mcb => mcb.list_data())
    }

    fn uncorrected_spectrum(&self) -> &Spectrum {
        delegate!(self, mcb => mcb.uncorrected_spectrum())
    }

    fn error_spectrum(&self) -> Option<&Spectrum> {
        delegate!(self, mcb => mcb.error_spectrum())
    }

    fn do_start_optimize(&mut self, admitted: Admitted) -> Result<(), DeviceError> {
        delegate!(self, mcb => mcb.do_start_optimize(admitted))
    }

    fn optimizing(&self) -> bool {
        delegate!(self, mcb => mcb.optimizing())
    }

    fn do_set_pole_zero_running(
        &mut self,
        run: bool,
        admitted: Admitted,
    ) -> Result<(), DeviceError> {
        delegate!(self, mcb => mcb.do_set_pole_zero_running(run, admitted))
    }

    fn pole_zeroing(&self) -> bool {
        delegate!(self, mcb => mcb.pole_zeroing())
    }

    fn pole_zero_error(&self) -> f64 {
        delegate!(self, mcb => mcb.pole_zero_error())
    }

    fn stored_spectra(&self) -> usize {
        delegate!(self, mcb => mcb.stored_spectra())
    }

    fn stored_spectrum(&self, index: usize) -> Option<&Spectrum> {
        delegate!(self, mcb => mcb.stored_spectrum(index))
    }

    fn do_store_spectrum(&mut self, admitted: Admitted) -> Result<usize, DeviceError> {
        delegate!(self, mcb => mcb.do_store_spectrum(admitted))
    }

    fn do_erase_stored(&mut self, admitted: Admitted) -> Result<(), DeviceError> {
        delegate!(self, mcb => mcb.do_erase_stored(admitted))
    }

    fn do_send_message(
        &mut self,
        command: &str,
        admitted: Admitted,
    ) -> Result<String, DeviceError> {
        delegate!(self, mcb => mcb.do_send_message(command, admitted))
    }

    fn lock(&mut self, password: &str, owner: &str) -> Result<(), DeviceError> {
        delegate!(self, mcb => mcb.lock(password, owner))
    }

    fn unlock(&mut self, password: &str) -> Result<(), DeviceError> {
        delegate!(self, mcb => mcb.unlock(password))
    }

    fn is_locked(&self) -> bool {
        delegate!(self, mcb => mcb.is_locked())
    }

    fn reading_untrusted(&self) -> Option<&str> {
        delegate!(self, mcb => mcb.reading_untrusted())
    }

    fn readings(&self) -> u64 {
        delegate!(self, mcb => mcb.readings())
    }

    fn reading_at(&self) -> Option<chrono::NaiveDateTime> {
        delegate!(self, mcb => mcb.reading_at())
    }
}
