//! A log of notable things that happened, for display and analysis.

use bevy_ecs::prelude::Resource;

use crate::city::CityId;
use crate::map::Position;
use crate::volcano::VolcanoId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimEvent {
    CityFounded {
        turn: u64,
        id: CityId,
        name: String,
        position: Position,
    },
    /// A volcano erupted. `cause` is its geological setting; `repose_years`
    /// how long it had been quiet (None for its first eruption on record).
    Eruption {
        turn: u64,
        volcano: VolcanoId,
        name: String,
        position: Position,
        vei: u8,
        cause: &'static str,
        repose_years: Option<u64>,
        casualties: u64,
    },
}

#[derive(Resource, Debug, Default)]
pub struct EventLog {
    events: Vec<SimEvent>,
}

impl EventLog {
    pub fn push(&mut self, event: SimEvent) {
        self.events.push(event);
    }

    pub fn events(&self) -> &[SimEvent] {
        &self.events
    }
}
