//! How well a bot flies - one profile per skill level (see `Difficulty`),
//! read by everything a pilot does: how hard it turns and how precisely it
//! flies a path (plane::autopilot), how fast it goes when its speed is left
//! to it (`Pace::BySkill`), how quickly it answers a missile with flares
//! (ai_defense). Kept in `assets/pilot_skills.ron`, editable in game from
//! the editor (see flight_editor's Pilot skills window) - the numbers below
//! are used for any level the file doesn't have.

use std::sync::{LazyLock, RwLock};

use serde::{Deserialize, Serialize};

use super::map::Difficulty;

/// Where the profiles are kept.
pub const SKILLS_FILE: &str = "assets/pilot_skills.ron";

/// One skill level's pilot.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PilotSkill {
    /// The most G it pulls.
    pub max_g: f32,
    /// The most angle of attack it's willing to fly (deg) - it eases off
    /// the pull as it gets there.
    pub max_aoa_deg: f32,
    /// The steepest bank it turns with (deg) - a hard level turn needs it
    /// near 90 (9 G: about 84); past 90, rolled over to pull down.
    pub max_bank_deg: f32,
    /// How hard it banks into a turn - degrees of bank per degree it's off
    /// its heading.
    pub turn_eagerness: f32,
    /// How far from a path's point it starts turning for the next (m) -
    /// small: it overshoots and swings wide; big: it cuts in early.
    pub corner_lead_m: f32,
    /// How precisely it holds its heading, height and speed, 0..1 - below
    /// 1 it wanders around them, and its hands on the throttle are lazier.
    pub precision: f32,
    /// How fast its hands move the stick and throttle (per second).
    pub control_rate: f32,
    /// Its speed when the speed's left to it (`Pace::BySkill`), knots.
    pub cruise_kt: f32,
    /// With its speed left to it, it goes as fast as the plane will - full
    /// throttle, `cruise_kt` only as a floor.
    pub max_performance: bool,
    /// How long a missile's been coming before it's noticed (s).
    pub reaction_seconds: f32,
    /// Once it's reacted, how often it bursts flares again while a missile's
    /// still coming (s) - `None`: once for each missile.
    pub flare_burst_every: Option<f32>,
}

impl PilotSkill {
    /// The built-in profile for `difficulty`.
    pub const fn default_for(difficulty: Difficulty) -> PilotSkill {
        match difficulty {
            Difficulty::Beginner => PilotSkill {
                max_g: 4.0,
                max_aoa_deg: 12.0,
                max_bank_deg: 45.0,
                turn_eagerness: 1.0,
                corner_lead_m: 150.0,
                precision: 0.4,
                control_rate: 2.0,
                cruise_kt: 300.0,
                max_performance: false,
                reaction_seconds: 2.5,
                flare_burst_every: None,
            },
            Difficulty::Medium => PilotSkill {
                max_g: 6.0,
                max_aoa_deg: 18.0,
                max_bank_deg: 75.0,
                turn_eagerness: 1.5,
                corner_lead_m: 300.0,
                precision: 0.75,
                control_rate: 4.0,
                cruise_kt: 400.0,
                max_performance: false,
                reaction_seconds: 1.2,
                flare_burst_every: Some(3.0),
            },
            Difficulty::Ace => PilotSkill {
                max_g: 9.0,
                max_aoa_deg: 25.0,
                max_bank_deg: 120.0,
                turn_eagerness: 2.5,
                corner_lead_m: 600.0,
                precision: 1.0,
                control_rate: 7.0,
                cruise_kt: 520.0,
                max_performance: true,
                reaction_seconds: 0.4,
                flare_burst_every: Some(1.5),
            },
        }
    }
}

/// Every skill level's profile - `pilot_skills.ron`'s contents.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PilotSkills {
    #[serde(default = "beginner")]
    pub beginner: PilotSkill,
    #[serde(default = "medium")]
    pub medium: PilotSkill,
    #[serde(default = "ace")]
    pub ace: PilotSkill,
}

fn beginner() -> PilotSkill { PilotSkill::default_for(Difficulty::Beginner) }
fn medium() -> PilotSkill { PilotSkill::default_for(Difficulty::Medium) }
fn ace() -> PilotSkill { PilotSkill::default_for(Difficulty::Ace) }

impl Default for PilotSkills {
    fn default() -> Self {
        Self { beginner: beginner(), medium: medium(), ace: ace() }
    }
}

impl PilotSkills {
    pub fn get(&self, difficulty: Difficulty) -> PilotSkill {
        match difficulty {
            Difficulty::Beginner => self.beginner,
            Difficulty::Medium => self.medium,
            Difficulty::Ace => self.ace,
        }
    }

    pub fn get_mut(&mut self, difficulty: Difficulty) -> &mut PilotSkill {
        match difficulty {
            Difficulty::Beginner => &mut self.beginner,
            Difficulty::Medium => &mut self.medium,
            Difficulty::Ace => &mut self.ace,
        }
    }

    /// The file's profiles - the built-in ones for whatever it leaves out,
    /// or all of them when it's missing or doesn't read.
    pub fn load() -> PilotSkills {
        let Ok(text) = std::fs::read_to_string(SKILLS_FILE) else { return PilotSkills::default() };
        ron::from_str(&text).unwrap_or_else(|error| {
            eprintln!("{SKILLS_FILE} isn't valid ({error}) - using the built-in pilot skills");
            PilotSkills::default()
        })
    }

    /// Writes them to the file, replacing it.
    pub fn save(&self) -> Result<(), String> {
        let config = ron::ser::PrettyConfig::new().indentor("  ");
        let body = ron::ser::to_string_pretty(self, config).map_err(|error| format!("pilot skills: couldn't write them out: {error}"))?;
        std::fs::write(SKILLS_FILE, format!("{SKILLS_HEADER}{body}\n")).map_err(|error| format!("pilot skills: couldn't write {SKILLS_FILE}: {error}"))
    }
}

const SKILLS_HEADER: &str = "\
// How well a bot flies, per skill level (see play::pilot_skill::PilotSkill) -
// edit it in game from the editor's Pilot skills window, which saves here.
//   max_g             - the most G it pulls
//   max_aoa_deg       - the most angle of attack it flies (deg)
//   max_bank_deg      - its steepest bank in a turn (deg)
//   turn_eagerness    - degrees of bank per degree off its heading
//   corner_lead_m     - how far from a path point it turns for the next (m)
//   precision         - 0..1, how exactly it holds heading, height and speed
//   control_rate      - how fast its hands move (per second)
//   cruise_kt         - its speed when the speed's left to it (knots)
//   max_performance   - with its speed left to it, full throttle
//   reaction_seconds  - how long until it notices a missile (s)
//   flare_burst_every - how often it bursts flares again (s), None: once
";

/// The profiles in use - read from the file the first time they're needed,
/// changed live by the editor.
static SKILLS: LazyLock<RwLock<PilotSkills>> = LazyLock::new(|| RwLock::new(PilotSkills::load()));

/// Every profile as it is now.
pub fn skills() -> PilotSkills {
    *SKILLS.read().unwrap()
}

/// Replaces the profiles in use (the editor's changes) - not saved.
pub fn set_skills(skills: PilotSkills) {
    *SKILLS.write().unwrap() = skills;
}

impl Difficulty {
    /// This skill level's pilot, as the profiles have it now.
    pub fn skill(self) -> PilotSkill {
        skills().get(self)
    }
}
