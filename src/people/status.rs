//! Statuses: the positions a culture offers (elder, chief, priest, scribe,
//! artisan, warrior, bonded...), unlocked by the city's technologies, and
//! who holds them. Also personal knowledge over time: learning from parents
//! and teachers, and a city knowing a technology only while someone in it
//! does.
//!
//! Tuning: the `status` section of `data/people.json`.

use bevy_ecs::prelude::*;

use super::{ActionKind, StatusId};
use bevy_ecs::schedule::ScheduleConfigs;
use bevy_ecs::system::ScheduleSystem;

/// Inserts this module's resources (its section of `data/people.json`).
pub fn install(world: &mut World) {
    world.init_resource::<StatusCatalog>();
}

/// This module's systems, in order, for its slot in the turn.
pub fn systems() -> ScheduleConfigs<ScheduleSystem> {
    nothing_yet.into_configs()
}

fn nothing_yet() {}

/// What each status allows, read by `people::decide`. Until this module
/// fills it in, every status may do everything, with no preference.
#[derive(Resource, Debug, Clone, Default)]
pub struct StatusCatalog {}

impl StatusCatalog {
    /// Whether someone of `status` may take up `action`.
    pub fn allows(&self, _status: StatusId, _action: ActionKind) -> bool {
        true
    }

    /// How much more (or less) someone of `status` leans toward `action`,
    /// added to its score (0 neutral).
    pub fn leaning(&self, _status: StatusId, _action: ActionKind) -> i32 {
        0
    }
}
