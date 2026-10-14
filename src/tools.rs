//! Tools: items (stone, copper, bronze tools...) made by people who craft
//! them, such as a blacksmith, and kept in their city's [`Stores`](crate::economy::Stores). People
//! take tools from the stores ([`Equipment`]), work better with them, and
//! wear them out.
//!
//! Tuning: the `tools` section of `data/people.json`.

use bevy_ecs::prelude::*;

use crate::people::ActionKind;
use bevy_ecs::schedule::ScheduleConfigs;
use bevy_ecs::system::ScheduleSystem;

/// Inserts this module's resources (its section of `data/people.json`).
pub fn install(world: &mut World) {
    world.init_resource::<ToolCatalog>();
}

/// This module's systems, in order, for its slot in the turn.
pub fn systems() -> ScheduleConfigs<ScheduleSystem> {
    nothing_yet.into_configs()
}

fn nothing_yet() {}

/// The item catalog, read by `economy` (and anyone else whose work tools
/// speed up). Until this module fills it in, tools make no difference.
#[derive(Resource, Debug, Clone, Default)]
pub struct ToolCatalog {}

impl ToolCatalog {
    /// How productive someone with `equipment` is at `action`, per mille
    /// (1000: as without tools).
    pub fn work_multiplier(&self, _equipment: &Equipment, _action: ActionKind) -> u32 {
        1000
    }
}

/// Index into the item catalog.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct ItemId(pub u16);

/// The tool a person carries, if any, and how worn it is (0 new, 1000
/// broken).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Equipment {
    pub tool: Option<ItemId>,
    pub wear: u16,
}
