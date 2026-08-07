//! What a `.JOB` needs from the application.
//!
//! Every method has a default that reports the command as unsupported, so a host
//! only implements what it can do. The command-line tool implements the file and
//! acquisition commands; the desktop application implements the window ones too.

use crate::command::{BeepSpec, ListSlice};
use crate::error::JobError;
use crate::vars::JobVariables;

/// The application a job drives.
pub trait JobHost {
    /// Variables the job can read; the host updates them on every read.
    fn variables(&mut self) -> &mut JobVariables;

    /// `SET_DETECTOR` / `SET_BUFFER`. `None` selects the previous detector.
    fn select_detector(&mut self, _number: Option<u16>) -> Result<(), JobError> {
        Err(JobError::unsupported("SET_DETECTOR"))
    }

    /// `START`.
    fn start(&mut self) -> Result<(), JobError> {
        Err(JobError::unsupported("START"))
    }

    /// `STOP`.
    fn stop(&mut self) -> Result<(), JobError> {
        Err(JobError::unsupported("STOP"))
    }

    /// `CLEAR`.
    fn clear(&mut self) -> Result<(), JobError> {
        Err(JobError::unsupported("CLEAR"))
    }

    /// `WAIT` with no argument: wait until the detector stops.
    fn wait_for_stop(&mut self) -> Result<(), JobError> {
        Err(JobError::unsupported("WAIT"))
    }

    /// `WAIT <seconds>`.
    fn wait_seconds(&mut self, _seconds: f64) -> Result<(), JobError> {
        Err(JobError::unsupported("WAIT"))
    }

    /// `WAIT "program"`.
    fn wait_program(&mut self, _program: &str) -> Result<(), JobError> {
        Err(JobError::unsupported("WAIT"))
    }

    /// `WAIT_CHANGER`.
    fn wait_changer(&mut self) -> Result<(), JobError> {
        Err(JobError::unsupported("WAIT_CHANGER"))
    }

    /// `WAIT_AUTO`.
    fn wait_optimize(&mut self) -> Result<(), JobError> {
        Err(JobError::unsupported("WAIT_AUTO"))
    }

    /// `WAIT_PZ`.
    fn wait_pole_zero(&mut self) -> Result<(), JobError> {
        Err(JobError::unsupported("WAIT_PZ"))
    }

    /// `FILL_BUFFER`.
    fn fill_buffer(&mut self) -> Result<(), JobError> {
        Err(JobError::unsupported("FILL_BUFFER"))
    }

    /// `SAVE`.
    fn save(&mut self, _path: Option<&str>) -> Result<(), JobError> {
        Err(JobError::unsupported("SAVE"))
    }

    /// `SAVE_ROI`.
    fn save_rois(&mut self, _path: &str) -> Result<(), JobError> {
        Err(JobError::unsupported("SAVE_ROI"))
    }

    /// `RECALL`.
    fn recall(&mut self, _path: &str) -> Result<(), JobError> {
        Err(JobError::unsupported("RECALL"))
    }

    /// `RECALL_CALIB`.
    fn recall_calibration(&mut self, _path: &str) -> Result<(), JobError> {
        Err(JobError::unsupported("RECALL_CALIB"))
    }

    /// `RECALL_ROI`.
    fn recall_rois(&mut self, _path: &str) -> Result<(), JobError> {
        Err(JobError::unsupported("RECALL_ROI"))
    }

    /// `STRIP`.
    fn strip(&mut self, _factor: f64, _path: Option<&str>) -> Result<(), JobError> {
        Err(JobError::unsupported("STRIP"))
    }

    /// `SET_NAME_STRIP`.
    fn set_strip_name(&mut self, _path: &str) -> Result<(), JobError> {
        Err(JobError::unsupported("SET_NAME_STRIP"))
    }

    /// `SMOOTH`.
    fn smooth(&mut self) -> Result<(), JobError> {
        Err(JobError::unsupported("SMOOTH"))
    }

    /// `MARK_PEAKS`.
    fn mark_peaks(&mut self) -> Result<(), JobError> {
        Err(JobError::unsupported("MARK_PEAKS"))
    }

    /// `REPORT`.
    fn report(&mut self, _path: Option<&str>) -> Result<(), JobError> {
        Err(JobError::unsupported("REPORT"))
    }

    /// `DESCRIBE_SAMPLE`.
    fn describe_sample(&mut self, _description: &str) -> Result<(), JobError> {
        Err(JobError::unsupported("DESCRIBE_SAMPLE"))
    }

    /// `LOAD_LIBRARY`.
    fn load_library(&mut self, _path: &str) -> Result<(), JobError> {
        Err(JobError::unsupported("LOAD_LIBRARY"))
    }

    /// `SET_PRESET_REAL`.
    fn set_preset_real(&mut self, _seconds: f64) -> Result<(), JobError> {
        Err(JobError::unsupported("SET_PRESET_REAL"))
    }

    /// `SET_PRESET_LIVE`.
    fn set_preset_live(&mut self, _seconds: f64) -> Result<(), JobError> {
        Err(JobError::unsupported("SET_PRESET_LIVE"))
    }

    /// `SET_PRESET_COUNT`.
    fn set_preset_peak(&mut self, _counts: u64) -> Result<(), JobError> {
        Err(JobError::unsupported("SET_PRESET_COUNT"))
    }

    /// `SET_PRESET_INTEG`.
    fn set_preset_integral(&mut self, _counts: u64) -> Result<(), JobError> {
        Err(JobError::unsupported("SET_PRESET_INTEG"))
    }

    /// `SET_PRESET_UNCERTAINTY`.
    fn set_preset_uncertainty(
        &mut self,
        _limit: f64,
        _low: usize,
        _high: usize,
    ) -> Result<(), JobError> {
        Err(JobError::unsupported("SET_PRESET_UNCERTAINTY"))
    }

    /// `SET_PRESET_CLEAR`.
    fn clear_presets(&mut self) -> Result<(), JobError> {
        Err(JobError::unsupported("SET_PRESET_CLEAR"))
    }

    /// `SET_LIST` and `SET_PHA`.
    fn set_list_mode(&mut self, _list: bool) -> Result<(), JobError> {
        Err(JobError::unsupported("SET_LIST"))
    }

    /// `SET_RANGE`.
    fn set_list_range(&mut self, _slice: &ListSlice) -> Result<(), JobError> {
        Err(JobError::unsupported("SET_RANGE"))
    }

    /// `SEND_MESSAGE`.
    fn send_message(&mut self, _command: &str) -> Result<String, JobError> {
        Err(JobError::unsupported("SEND_MESSAGE"))
    }

    /// `LOCK`.
    fn lock(&mut self, _password: &str, _owner: &str, _name: Option<&str>) -> Result<(), JobError> {
        Err(JobError::unsupported("LOCK"))
    }

    /// `UNLOCK`.
    fn unlock(&mut self, _password: &str) -> Result<(), JobError> {
        Err(JobError::unsupported("UNLOCK"))
    }

    /// `ASK_PASSWORD`: returns the password and owner to record.
    fn ask_password(&mut self) -> Result<(String, String), JobError> {
        Err(JobError::unsupported("ASK_PASSWORD"))
    }

    /// `CHANGE_SAMPLE`.
    fn change_sample(&mut self) -> Result<(), JobError> {
        Err(JobError::unsupported("CHANGE_SAMPLE"))
    }

    /// `BEEP`.
    fn beep(&mut self, _spec: &BeepSpec) -> Result<(), JobError> {
        Err(JobError::unsupported("BEEP"))
    }

    /// `RUN` and `RUN_MINIMIZED`.
    fn run_program(&mut self, _program: &str, _minimized: bool) -> Result<(), JobError> {
        Err(JobError::unsupported("RUN"))
    }

    /// `EXPORT`.
    fn export(&mut self, _path: &str) -> Result<(), JobError> {
        Err(JobError::unsupported("EXPORT"))
    }

    /// `IMPORT`.
    fn import(&mut self, _path: &str) -> Result<(), JobError> {
        Err(JobError::unsupported("IMPORT"))
    }

    /// `CLOSEBUFFERS`.
    fn close_buffers(&mut self) -> Result<(), JobError> {
        Err(JobError::unsupported("CLOSEBUFFERS"))
    }

    /// `CLOSEMCBS`.
    fn close_detectors(&mut self) -> Result<(), JobError> {
        Err(JobError::unsupported("CLOSEMCBS"))
    }

    /// `VIEW`.
    fn view_stored(&mut self, _index: usize) -> Result<(), JobError> {
        Err(JobError::unsupported("VIEW"))
    }

    /// How many spectra the instrument holds, for `LOOP SPECTRA`.
    fn stored_spectra(&mut self) -> Result<usize, JobError> {
        Err(JobError::unsupported("LOOP SPECTRA"))
    }

    /// `START_OPTIMIZE`.
    fn start_optimize(&mut self) -> Result<(), JobError> {
        Err(JobError::unsupported("START_OPTIMIZE"))
    }

    /// `START_PZ` and `STOP_PZ`.
    fn pole_zero(&mut self, _start: bool) -> Result<(), JobError> {
        Err(JobError::unsupported("START_PZ"))
    }

    /// `ZOOM` and `ZOOM:`.
    fn zoom(
        &mut self,
        _rectangle: Option<(i32, i32, i32, i32)>,
        _state: i32,
    ) -> Result<(), JobError> {
        Err(JobError::unsupported("ZOOM"))
    }

    /// `QUIT`.
    fn quit(&mut self) -> Result<(), JobError> {
        Ok(())
    }

    /// Called for every command, before it runs, so a host can log progress.
    fn trace(&mut self, _line: usize, _command: &str) {}
}
