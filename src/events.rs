//! A log of notable things that happened, for display and analysis.

use bevy_ecs::prelude::Resource;

use crate::city::CityId;
use crate::map::Position;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimEvent {
    CityFounded {
        turn: u64,
        id: CityId,
        name: String,
        position: Position,
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
