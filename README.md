# societies

A turn-based, **fully deterministic** simulation of societies — people, morals
and beliefs, economy, diplomacy, wars and philosophies — built in Rust on an
Entity-Component-System ([`bevy_ecs`](https://crates.io/crates/bevy_ecs), used
standalone without the rest of Bevy).

Current state: a generated Earth-like world (continents, altitudes, rivers,
lakes and five biomes) on which cities are occasionally founded with a
population of 10 and then grow slowly (logistic growth toward a carrying
capacity).

![Example world, seed 3](docs/example-map.png)

## Running

```sh
cargo run --release -- --seed 42 --turns 300
cargo test
```

```sh
# Just look at a world: 512x256 tiles, written as a 2-pixels-per-tile PNG.
cargo run --release -- --seed 7 --width 512 --height 256 --turns 0 --quiet --png world.png --scale 2
```

Flags: `--seed`, `--turns`, `--width`, `--height`, `--land` (land share in
1/1000, default 330), `--png FILE`, `--scale` (pixels per tile), `--quiet`.
The run prints a terrain summary, the event log, an ASCII map, every city, and
a state digest. The same flags always produce the same digest.

ASCII legend: `~` water, `.` earth, `:` sand, `n` hills, `^` mountains,
`=` river, `o`/`O`/`@` city (< 100, < 1000, larger).

## World generation

Every tile has an altitude (meters relative to sea level; the bed for oceans
and lakes), one of five biomes (water, earth, sand, hills, mountains), a water
feature (dry, river, lake, ocean), rainfall, accumulated river flow and the
tile it drains into. Generation runs in three stages (`src/map/`):

1. **Terrain** (`terrain.rs`): a domain-warped fractal field places the
   continents and fades out toward the map border. Sea level is picked so
   that exactly `land_per_mille` of the map is land. Land rises from the coast
   inland, with ridged-noise mountain ranges in a few orogenic belts and
   tile-scale roughness. The sea floor has a shallow continental shelf
   dropping into the deep ocean.
2. **Hydrology** (`hydrology.rs`): large basins below sea level become ocean.
   Rainfall follows Earth's latitude bands (wet tropics, dry subtropics, wet
   temperate belt, dry poles), drops off away from the coast, and varies with
   noise. A priority flood from the sea finds depressions: deep ones become
   lakes and shallow ones are filled in. Every land tile drains by steepest
   descent, or across flats toward the sea, and rain accumulated downstream
   forms rivers.
3. **Biomes** (`biomes.rs`): water = ocean or lake; mountains = high;
   hills = moderately high or rugged; sand = dry beaches and deserts (dry
   land with no river); earth = everything else.

All of it is integer / fixed-point math, so a seed produces the same world on
every machine. Tuning knobs live in `TerrainConfig` (`src/map/mod.rs`).

## Determinism rules

Every run is a pure function of its `SimConfig`, so experiments with different
initial conditions are reproducible. To keep it that way, every system must:

- Draw randomness only from the single `SimRng` resource (a self-implemented
  xoshiro256**, pinned by tests so dependency upgrades can't change it).
- Run on the single-threaded executor, in the explicit `.chain()` order defined
  in `Simulation::new` (`bevy_ecs`'s `multi_threaded` feature is disabled).
- Never let ECS query iteration order affect outcomes — sort by a stable
  simulation id (e.g. `CityId`) before drawing random numbers or breaking ties.
- Use integer / fixed-point math for simulation state, not floats.
- Avoid wall-clock time and `HashMap`/`HashSet` iteration with random hashers.

`tests/determinism.rs` checks that two runs with the same seed have identical
state digests (map included) on every turn; `tests/terrain.rs` checks the
generated world: land share, all biomes present, every tile drains to the sea,
rivers never end on dry land.

## Layout

| File | Contents |
| --- | --- |
| `src/lib.rs` | `Simulation`: world setup, turn schedule, snapshots, digest, ASCII render |
| `src/config.rs` | `SimConfig`: all initial conditions and tuning knobs |
| `src/rng.rs` | `SimRng`: the deterministic RNG |
| `src/map/mod.rs` | `Map` resource, `Tile`, `Biome`, `Water`, `TerrainConfig`, `Position` |
| `src/map/noise.rs` | Fixed-point value noise, fractal and ridged variants |
| `src/map/terrain.rs` | Stage 1: continents and altitudes |
| `src/map/hydrology.rs` | Stage 2: oceans, rainfall, lakes, rivers |
| `src/map/biomes.rs` | Stage 3: biome classification |
| `src/render.rs` | ASCII and PNG rendering |
| `src/city.rs` | City components and the `grow_cities` / `spawn_cities` systems |
| `src/turn.rs` | `Turn` counter |
| `src/events.rs` | `EventLog` of notable happenings |
| `src/main.rs` | Command-line runner |

A turn runs: `advance_turn` → `grow_cities` → `spawn_cities`.
