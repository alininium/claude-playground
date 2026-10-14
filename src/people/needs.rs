//! Needs: how each person's needs grow over time (hunger, tiredness,
//! cold from the tile's temperature, loneliness, boredom...) and fall when
//! satisfied. Per-person and parallel (see `people::parallel`), ideally lazy:
//! people with nothing to decide should cost next to nothing.
//!
//! Tuning: the `needs` section of `data/people.json`.

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::ScheduleConfigs;
use bevy_ecs::system::ScheduleSystem;

/// Inserts this module's resources (its section of `data/people.json`).
pub fn install(_world: &mut World) {}

/// This module's systems, in order, for its slot in the turn.
pub fn systems() -> ScheduleConfigs<ScheduleSystem> {
    nothing_yet.into_configs()
}

fn nothing_yet() {}
