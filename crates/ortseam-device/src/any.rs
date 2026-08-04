//! One name for every kind of instrument the application can hold.
//!
//! The GUI keeps a plain `Vec<AnyMcb>`: the simulator and a network instrument
//! sit side by side in the pick list and take the same commands. Simulator-only
//! affordances (the source editor, the sample-changer lines) ask politely with
//! [`AnyMcb::as_simulated_mut`] and disable themselves when the answer is no.

use ortseam_core::{AcquisitionMode, Spectrum};

use crate::error::DeviceError;
use crate::mcb::{Mcb, McbIdentity, McbProperties, McbStatus};
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

impl Mcb for AnyMcb {
    fn identity(&self) -> &McbIdentity {
        delegate!(self, mcb => mcb.identity())
    }

    fn properties(&self) -> &McbProperties {
        delegate!(self, mcb => mcb.properties())
    }

    fn set_properties(&mut self, properties: McbProperties) -> Result<(), DeviceError> {
        delegate!(self, mcb => mcb.set_properties(properties))
    }

    fn presets_mut(&mut self) -> &mut Presets {
        delegate!(self, mcb => mcb.presets_mut())
    }

    fn set_presets(&mut self, presets: Presets) -> Result<(), DeviceError> {
        delegate!(self, mcb => mcb.set_presets(presets))
    }

    fn start(&mut self) -> Result<(), DeviceError> {
        delegate!(self, mcb => mcb.start())
    }

    fn stop(&mut self) -> Result<(), DeviceError> {
        delegate!(self, mcb => mcb.stop())
    }

    fn clear(&mut self) -> Result<(), DeviceError> {
        delegate!(self, mcb => mcb.clear())
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

    fn set_mode(&mut self, mode: AcquisitionMode) -> Result<(), DeviceError> {
        delegate!(self, mcb => mcb.set_mode(mode))
    }

    fn mode(&self) -> AcquisitionMode {
        delegate!(self, mcb => mcb.mode())
    }

    fn list_data(&self) -> Option<&ortseam_formats::ListModeFile> {
        delegate!(self, mcb => mcb.list_data())
    }

    fn uncorrected_spectrum(&self) -> &Spectrum {
        delegate!(self, mcb => mcb.uncorrected_spectrum())
    }

    fn error_spectrum(&self) -> Option<&Spectrum> {
        delegate!(self, mcb => mcb.error_spectrum())
    }

    fn start_optimize(&mut self) -> Result<(), DeviceError> {
        delegate!(self, mcb => mcb.start_optimize())
    }

    fn optimizing(&self) -> bool {
        delegate!(self, mcb => mcb.optimizing())
    }

    fn set_pole_zero_running(&mut self, run: bool) -> Result<(), DeviceError> {
        delegate!(self, mcb => mcb.set_pole_zero_running(run))
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

    fn store_spectrum(&mut self) -> Result<usize, DeviceError> {
        delegate!(self, mcb => mcb.store_spectrum())
    }

    fn erase_stored(&mut self) {
        delegate!(self, mcb => mcb.erase_stored())
    }

    fn send_message(&mut self, command: &str) -> Result<String, DeviceError> {
        delegate!(self, mcb => mcb.send_message(command))
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
}
