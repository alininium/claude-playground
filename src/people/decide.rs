//! Decisions: when a person's activity ends (`Activity::until`) or a need
//! becomes urgent, they choose their next action from the catalog in the
//! `actions` section of `data/people.json`, scoring each by how urgent the
//! needs it serves are, their character (`Traits`), what they know
//! (`Skills`), their status, their age and sex, the season and what their
//! city offers. Per-person and parallel. Night is sleep.

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
