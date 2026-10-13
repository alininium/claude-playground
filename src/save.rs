//! Saving and loading worlds.
//!
//! A [`WorldSave`] holds everything about the planet: the map, the whole
//! climate state (tile profiles, temperatures, the oscillation and drift with
//! their random number streams, ice and snow, soil water, statistics and
//! normals, weather), biomes, fertility, volcanoes and their history, the
//! climate journal, and the configuration that shaped them. Societies
//! (cities) are not part of it: they are founded when a world is loaded,
//! from a seed stored in the save, exactly as they would have been in a
//! freshly generated world. So generating a world, saving it and loading it
//! gives a simulation identical to [`Simulation::new`] with the same config.
//!
//! Two formats:
//!
//! - **Binary** (default): a short header (magic, format version) followed
//!   by the world in `bincode`, deflate-compressed. Compact and fast.
//! - **JSON** (files ending in `.json`): `{"format": "societies-world",
//!   "version": …, "world": {…}}`, readable by any tool, several times
//!   larger. Loading detects the format from the content.
//!
//! [`Simulation::new`]: crate::Simulation::new

use std::io::{self, Read, Write};
use std::path::Path;

use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use serde::{Deserialize, Serialize};

use crate::biome::Biomes;
use crate::climate::Climate;
use crate::config::SimConfig;
use crate::fertility::Fertility;
use crate::journal::Journal;
use crate::map::{Map, Position};
use crate::rng::SimRng;
use crate::volcano::{Volcanism, Volcano};

const MAGIC: &[u8; 8] = b"SOCWORLD";
const JSON_FORMAT: &str = "societies-world";
/// Bumped whenever the saved data layout changes.
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldSave {
    /// The configuration the world was generated (and last run) with.
    pub config: SimConfig,
    pub turn: u64,
    pub map: Map,
    pub climate: Climate,
    pub biomes: Biomes,
    pub fertility: Fertility,
    pub volcanism: Volcanism,
    pub volcanoes: Vec<(Volcano, Position)>,
    pub journal: Journal,
    /// The simulation's main random number stream.
    pub rng: SimRng,
    /// Seed of the stream that founds the cities when the world is used.
    pub society_seed: u64,
}

impl WorldSave {
    pub fn to_bytes(&self) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        let mut encoder = DeflateEncoder::new(out, Compression::default());
        bincode::serialize_into(&mut encoder, self).map_err(io::Error::other)?;
        encoder.finish()
    }

    /// The world as JSON.
    pub fn to_json(&self) -> io::Result<Vec<u8>> {
        #[derive(Serialize)]
        struct Envelope<'a> {
            format: &'a str,
            version: u32,
            world: &'a WorldSave,
        }
        serde_json::to_vec(&Envelope {
            format: JSON_FORMAT,
            version: FORMAT_VERSION,
            world: self,
        })
        .map_err(io::Error::other)
    }

    pub fn from_json(bytes: &[u8]) -> io::Result<Self> {
        #[derive(Deserialize)]
        struct Envelope {
            format: String,
            version: u32,
            world: serde_json::Value,
        }
        let invalid = |message: String| io::Error::new(io::ErrorKind::InvalidData, message);
        let envelope: Envelope =
            serde_json::from_slice(bytes).map_err(|e| invalid(e.to_string()))?;
        if envelope.format != JSON_FORMAT {
            return Err(invalid(format!("not a world file: {}", envelope.format)));
        }
        if envelope.version != FORMAT_VERSION {
            return Err(invalid(format!(
                "world file version {}, expected {FORMAT_VERSION}",
                envelope.version
            )));
        }
        serde_json::from_value(envelope.world).map_err(|e| invalid(e.to_string()))
    }

    /// Reads either format (JSON if it starts with `{`).
    pub fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        if bytes.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{') {
            return Self::from_json(bytes);
        }
        let header = MAGIC.len() + 4;
        if bytes.len() < header || &bytes[..MAGIC.len()] != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not a world file",
            ));
        }
        let version = u32::from_le_bytes(bytes[MAGIC.len()..header].try_into().unwrap());
        if version != FORMAT_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("world file version {version}, expected {FORMAT_VERSION}"),
            ));
        }
        let mut data = Vec::new();
        DeflateDecoder::new(&bytes[header..]).read_to_end(&mut data)?;
        bincode::deserialize(&data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Writes the world: JSON if the file name ends in `.json`, else binary.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        let json = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"));
        let bytes = if json {
            self.to_json()?
        } else {
            self.to_bytes()?
        };
        let mut file = io::BufWriter::new(std::fs::File::create(path)?);
        file.write_all(&bytes)?;
        file.flush()
    }

    pub fn read(path: &Path) -> io::Result<Self> {
        Self::from_bytes(&std::fs::read(path)?)
    }
}
