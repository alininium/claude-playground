# societies

A turn-based, **fully deterministic** simulation of societies — people, morals
and beliefs, economy, diplomacy, wars and philosophies — built in Rust on an
Entity-Component-System ([`bevy_ecs`](https://crates.io/crates/bevy_ecs), used
standalone without the rest of Bevy).

Current state: skeleton. An empty map on which cities are occasionally founded
with a population of 10 and then grow slowly (logistic growth toward a carrying
capacity).

## Running

```sh
cargo run --release -- --seed 42 --turns 300
cargo test
```

Flags: `--seed`, `--turns`, `--width`, `--height`. The run prints the event
log, an ASCII map (`o` < 100 people, `O` < 1000, `@` larger), every city, and a
state digest. The same flags always produce the same digest.

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
state digests on every turn.

## Layout

| File | Contents |
| --- | --- |
| `src/lib.rs` | `Simulation`: world setup, turn schedule, snapshots, digest, ASCII render |
| `src/config.rs` | `SimConfig`: all initial conditions and tuning knobs |
| `src/rng.rs` | `SimRng`: the deterministic RNG |
| `src/map.rs` | `Map` resource, `Terrain`, `Position` component |
| `src/city.rs` | City components and the `grow_cities` / `spawn_cities` systems |
| `src/turn.rs` | `Turn` counter |
| `src/events.rs` | `EventLog` of notable happenings |
| `src/main.rs` | Command-line runner |

A turn runs: `advance_turn` → `grow_cities` → `spawn_cities`.
