//! Stage progression: which stages are unlocked and the best grade earned
//! on each. Persisted as RON next to the settings file.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::core::ai::MAX_STAGE;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Grade {
    D,
    C,
    B,
    A,
    S,
}

impl Grade {
    pub fn letter(self) -> &'static str {
        match self {
            Grade::S => "S",
            Grade::A => "A",
            Grade::B => "B",
            Grade::C => "C",
            Grade::D => "D",
        }
    }

    pub fn color(self) -> Color {
        match self {
            Grade::S => Color::srgb(1.0, 0.85, 0.2),
            Grade::A => Color::srgb(0.4, 0.95, 1.0),
            Grade::B => Color::srgb(0.4, 0.95, 0.5),
            Grade::C => Color::srgb(0.8, 0.8, 0.85),
            Grade::D => Color::srgb(0.7, 0.55, 0.45),
        }
    }

    fn rank(self) -> u32 {
        match self {
            Grade::D => 0,
            Grade::C => 1,
            Grade::B => 2,
            Grade::A => 3,
            Grade::S => 4,
        }
    }

    pub fn better_than(self, other: Grade) -> bool {
        self.rank() > other.rank()
    }
}

#[derive(Resource, Debug, Clone, Serialize, Deserialize)]
pub struct Progress {
    /// Highest stage the player may enter (1-based).
    pub unlocked: u32,
    pub grades: HashMap<u32, Grade>,
    /// Zone battle runs its own 30-stage campaign, tracked separately.
    #[serde(default = "first_stage")]
    pub zone_unlocked: u32,
    #[serde(default)]
    pub zone_grades: HashMap<u32, Grade>,
    /// Best sprint (40 lines) finish, in milliseconds.
    #[serde(default)]
    pub best_sprint_ms: Option<u64>,
    /// Best dig (clear the garbage) finish, in milliseconds.
    #[serde(default)]
    pub best_dig_ms: Option<u64>,
}

fn first_stage() -> u32 {
    1
}

impl Default for Progress {
    fn default() -> Self {
        Self {
            unlocked: 1,
            grades: HashMap::new(),
            zone_unlocked: 1,
            zone_grades: HashMap::new(),
            best_sprint_ms: None,
            best_dig_ms: None,
        }
    }
}

impl Progress {
    pub fn is_unlocked(&self, stage: u32) -> bool {
        stage <= self.unlocked
    }

    pub fn is_zone_unlocked(&self, stage: u32) -> bool {
        stage <= self.zone_unlocked
    }

    /// Record a stage clear; returns true if this grade is a new best.
    pub fn record_clear(&mut self, stage: u32, grade: Grade) -> bool {
        self.unlocked = self.unlocked.max((stage + 1).min(MAX_STAGE));
        let new_best = match self.grades.get(&stage) {
            Some(old) => grade.better_than(*old),
            None => true,
        };
        if new_best {
            self.grades.insert(stage, grade);
        }
        save_progress(self);
        new_best
    }

    /// Record a zone battle stage clear; returns true on a new best grade.
    pub fn record_zone_clear(&mut self, stage: u32, grade: Grade) -> bool {
        self.zone_unlocked = self.zone_unlocked.max((stage + 1).min(MAX_STAGE));
        let new_best = match self.zone_grades.get(&stage) {
            Some(old) => grade.better_than(*old),
            None => true,
        };
        if new_best {
            self.zone_grades.insert(stage, grade);
        }
        save_progress(self);
        new_best
    }

    /// Record a finished sprint run; returns true on a new best time.
    pub fn record_sprint(&mut self, ms: u64) -> bool {
        let new_best = self.best_sprint_ms.map_or(true, |best| ms < best);
        if new_best {
            self.best_sprint_ms = Some(ms);
        }
        save_progress(self);
        new_best
    }

    /// Record a finished dig run; returns true on a new best time.
    pub fn record_dig(&mut self, ms: u64) -> bool {
        let new_best = self.best_dig_ms.map_or(true, |best| ms < best);
        if new_best {
            self.best_dig_ms = Some(ms);
        }
        save_progress(self);
        new_best
    }
}

fn progress_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("bevytris").join("progress.ron"))
}

pub fn load_progress() -> Progress {
    let Some(path) = progress_path() else {
        return Progress::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => ron::from_str(&text).unwrap_or_else(|err| {
            warn!("failed to parse {path:?}: {err}; starting fresh");
            Progress::default()
        }),
        Err(_) => Progress::default(),
    }
}

pub fn save_progress(progress: &Progress) {
    let Some(path) = progress_path() else { return };
    let Some(dir) = path.parent() else { return };
    if let Err(err) = std::fs::create_dir_all(dir) {
        warn!("could not create config dir {dir:?}: {err}");
        return;
    }
    match ron::ser::to_string_pretty(progress, Default::default()) {
        Ok(text) => {
            if let Err(err) = std::fs::write(&path, text) {
                warn!("could not write progress to {path:?}: {err}");
            }
        }
        Err(err) => warn!("could not serialize progress: {err}"),
    }
}

/// Debug helper: BEVYTRIS_UNLOCK_ALL=1 opens every stage for this run
/// (not persisted unless a stage is cleared).
pub fn load_progress_with_env() -> Progress {
    let mut progress = load_progress();
    if std::env::var("BEVYTRIS_UNLOCK_ALL").is_ok() {
        progress.unlocked = MAX_STAGE;
        progress.zone_unlocked = MAX_STAGE;
    }
    progress
}
