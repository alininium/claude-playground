//! Caravans: when enough people of a city choose to settle away on the same
//! day, they leave together on the next day as a caravan, which travels in
//! search of a place to settle and founds a new city there.
//!
//! Tuning: the `caravan` section of `data/people.json`.

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
