//! The world map. For now it is an empty grid of uniform terrain.

use bevy_ecs::prelude::{Component, Resource};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Terrain {
    Plains,
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Position {
    pub x: u32,
    pub y: u32,
}

#[derive(Resource, Debug, Clone)]
pub struct Map {
    width: u32,
    height: u32,
    tiles: Vec<Terrain>,
}

impl Map {
    pub fn new(width: u32, height: u32) -> Self {
        assert!(width > 0 && height > 0, "map must not be empty");
        Self {
            width,
            height,
            tiles: vec![Terrain::Plains; (width * height) as usize],
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn tile_count(&self) -> u32 {
        self.width * self.height
    }

    pub fn terrain(&self, pos: Position) -> Terrain {
        self.tiles[self.index(pos)]
    }

    /// Maps a linear tile index (row-major) back to a position.
    pub fn position_of(&self, index: u32) -> Position {
        Position {
            x: index % self.width,
            y: index / self.width,
        }
    }

    pub fn index(&self, pos: Position) -> usize {
        assert!(
            pos.x < self.width && pos.y < self.height,
            "position out of bounds"
        );
        (pos.y * self.width + pos.x) as usize
    }
}
