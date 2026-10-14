//! The life cycle: ageing and death (Siler mortality, worsened by hunger
//! and cold), pairing up (in the resolve step, per city, in id order),
//! pregnancy, and births. A child's seed comes from `rng::child_seed`, so
//! the same parents always have the same children; its traits from
//! `Traits::inherit`.
//!
//! Tuning: the `life` section of `data/people.json`.

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
