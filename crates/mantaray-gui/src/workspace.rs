//! Workspaces: what is on screen, for the job in front of you.
//!
//! Counting and interpreting are two different jobs, and they do not want the
//! same sidebar. While a count runs, what matters is the clock, the dead time,
//! the rate and how steady it has been - and the preset that will stop it.
//! Afterwards none of that changes again, and the room it takes is room the
//! region list and the nuclide lookup could have had.
//!
//! So the sidebar's sections are a set that a workspace chooses, the way a
//! scheme chooses colours. The built-in ones are presets; any of them can be
//! adjusted and kept under a name.
//!
//! This is deliberately not a scheme. A scheme is how the program looks and
//! travels between people; a workspace is what it shows and follows the task.
//! Somebody may well want the same colours for both jobs, and the same
//! workspace under two colour schemes.

use serde::{Deserialize, Serialize};

/// Which sections the sidebar shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sections {
    /// Start time, real and live time, dead time, counts and rate.
    #[serde(default = "yes")]
    pub counts: bool,
    /// The trace of rate and dead time over the run.
    #[serde(default = "yes")]
    pub stability: bool,
    /// The limits that will stop the count, and how far along it is.
    #[serde(default = "yes")]
    pub presets: bool,
    /// What the instrument itself is holding, and the button to fetch it.
    #[serde(default = "yes")]
    pub instrument: bool,
    /// Checking a nuclide by name against the spectrum.
    #[serde(default = "yes")]
    pub isotope: bool,
    /// The marked regions, with the energy and net area of each.
    #[serde(default = "yes")]
    pub regions: bool,
    /// The simulated clock speed, in builds that have a simulator.
    #[serde(default = "yes")]
    pub simulation: bool,
}

/// The default for a section that is shown unless a workspace says otherwise.
fn yes() -> bool {
    true
}

impl Default for Sections {
    fn default() -> Self {
        Self {
            counts: true,
            stability: true,
            presets: true,
            instrument: true,
            isotope: true,
            regions: true,
            simulation: true,
        }
    }
}

impl Sections {
    /// Every section, with a name and the flag that shows it, for the editor.
    pub fn each(&mut self) -> [(&'static str, &'static str, &mut bool); 7] {
        [
            (
                "Counts",
                "start time, real and live time, dead time, counts and rate",
                &mut self.counts,
            ),
            (
                "Stability",
                "how the rate and dead time have behaved over the run",
                &mut self.stability,
            ),
            (
                "Presets",
                "the limits that will stop the count",
                &mut self.presets,
            ),
            (
                "Instrument",
                "spectra held in the instrument, and the button to fetch them",
                &mut self.instrument,
            ),
            (
                "Isotope",
                "checking a nuclide by name against the spectrum",
                &mut self.isotope,
            ),
            (
                "Regions",
                "the marked regions, with the energy and net area of each",
                &mut self.regions,
            ),
            (
                "Simulation",
                "the simulated clock speed",
                &mut self.simulation,
            ),
        ]
    }
}

/// A named arrangement of what is on screen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    /// What to call it in the picker.
    pub name: String,
    /// Which sidebar sections it shows.
    #[serde(default)]
    pub sections: Sections,
}

impl Default for Workspace {
    /// Everything, until somebody says otherwise.
    ///
    /// Not one of the two useful ones. Both hide sections, and hiding a panel
    /// from somebody who has not asked for it is how a program earns a
    /// reputation for losing things. The arrangements are there to be chosen.
    fn default() -> Self {
        Self::everything()
    }
}

impl Workspace {
    /// A named workspace.
    pub fn new(name: impl Into<String>, sections: Sections) -> Self {
        Self {
            name: name.into(),
            sections,
        }
    }

    /// The built-in arrangements, for the picker.
    pub fn built_in() -> Vec<Workspace> {
        vec![Self::acquisition(), Self::analysis(), Self::everything()]
    }

    /// While a count is running: the clock, the rate, and what will stop it.
    ///
    /// The region list and the nuclide lookup are put away. Nothing is being
    /// interpreted yet, and until the count finishes the numbers in them are
    /// answers to a question nobody has asked.
    pub fn acquisition() -> Self {
        Self::new(
            "Acquisition",
            Sections {
                counts: true,
                stability: true,
                presets: true,
                instrument: true,
                isotope: false,
                regions: false,
                simulation: true,
            },
        )
    }

    /// After the count: the regions, the library and the nuclide lookup.
    ///
    /// The preset limits and the stability trace go away. Both describe a run
    /// that has already finished, and neither will change again.
    pub fn analysis() -> Self {
        Self::new(
            "Analysis",
            Sections {
                counts: true,
                stability: false,
                presets: false,
                instrument: false,
                isotope: true,
                regions: true,
                simulation: false,
            },
        )
    }

    /// Everything at once, which is where this started.
    pub fn everything() -> Self {
        Self::new("Everything", Sections::default())
    }
}
