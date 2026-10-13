# societies

A turn-based, **fully deterministic** simulation of societies — people, morals
and beliefs, economy, diplomacy, wars and philosophies — built in Rust on an
Entity-Component-System ([`bevy_ecs`](https://crates.io/crates/bevy_ecs), used
standalone without the rest of Bevy).

Current state: a generated Earth-like world (continents, altitudes, rivers,
lakes and five biomes) with a living climate (seasons, day and night, an
El Niño-like oscillation, slow climate drift), a cryosphere (sea, lake and
river ice, snow, frozen ground, permafrost, glaciers and polar caps) and land
fertility. Cities are placed on fertile land when the world is created, each
with 10 people, and then grow slowly (logistic growth toward a carrying
capacity); no new cities are founded during play.

![Example world, seed 3](docs/example-map.png)

## Running

```sh
cargo run --release -- --seed 42 --years 5
cargo test
```

```sh
# Just look at a world: 512x256 tiles, written as a 2-pixels-per-tile PNG.
cargo run --release -- --seed 7 --width 512 --height 256 --turns 0 --quiet --png world.png --scale 2
```

```sh
# Temperature map on a northern mid-summer afternoon (day 136, turn 2 of 4).
cargo run --release -- --seed 3 --width 256 --height 128 --turns 545 --quiet --temp-png summer.png --scale 3
```

Flags: `--seed`, `--years` / `--days` / `--turns` (run length, default 2
years), `--width`, `--height`, `--land` (land share in 1/1000, default 330),
`--png FILE` (relief map with current ice and snow), `--temp-png FILE`
(temperature map), `--fertility-png FILE`, `--scale` (pixels per tile),
`--quiet`. Maps show the state at the end of the run. The run prints a terrain,
fertility and frozen-world summary, a climate report at the start of every
season, the event log, an ASCII map, every city with its current temperature,
and a state digest. The same flags always produce the same digest. Shares
are by true surface area: on this equirectangular map, polar rows hold far
less area than their tile count suggests.

ASCII legend: `~` water, `.` earth, `:` sand, `n` hills, `^` mountains,
`=` river, `#` glacier or frozen sea/lake, `*` snow, `o`/`O`/`@` city
(< 100, < 1000, larger).

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

## Time and climate

One turn is a quarter of a day: **morning, day, evening, night**. A year has
364 days (configurable) in four seasons; turn 0 is the first morning of
northern spring. `Date::from_turn` (`src/time.rs`) converts turns to dates;
southern-hemisphere seasons are the opposite.

Every tile has a temperature (tenths of a degree C) recomputed every turn
(`src/climate/`):

```
temperature = annual mean + seasonal swing + day/night offset
            + oscillation index × local response + global drift × polar amplification
```

- **Annual mean**: by latitude, minus 6.5 °C per km of altitude (lakes use
  their water surface); dry subtropical land runs hotter, deep high-latitude
  interiors colder.
- **Seasons**: a cosine over the year whose amplitude grows toward the poles
  and with *continentality*, and shrinks over oceans, lakes and near rivers.
  It is reversed in the south and lags the solstice (about 25 days inland, 40
  on lakes, 50 at sea).
- **Continentality**: distance from the ocean that maritime air must travel,
  with mountains above 1000 m as barriers. The lee of a range gets a harsher
  climate.
- **Day and night**: night is coldest and afternoon warmest. The range is
  large in deserts and on high ground (up to ~20 °C), small near water and
  ~1 °C at sea.
- **Planet-wide oscillation** (`oscillation.rs`): an ENSO-like recharge
  oscillator. The surface temperature anomaly of a tropical "warm pool" and
  the ocean heat content drive each other a quarter cycle apart. Damping and
  random daily wind bursts make warm (El Niño-like) and cold (La Niña-like)
  phases come every 2-7 years, each one different. Each tile has a fixed
  response: strongest in the tropical ocean near the warm pool, opposite on
  the far side of the planet, and a weak teleconnection pattern elsewhere.
- **Global drift**: a slow mean-reverting random walk (centuries, ±0.5 °C
  typical), amplified up to 2× toward the poles.

The oscillation and drift each have their own RNG stream, so climate draws
never shift other subsystems' random numbers. Tuning lives in `ClimateConfig`.

![Northern mid-summer afternoon, seed 3](docs/example-temperature.png)

### What is frozen

`src/climate/cryosphere.rs` tracks, per tile and per turn:

| | Freezes | Grows / melts |
| --- | --- | --- |
| **Sea ice** | below −1.8 °C | Stefan's law (thin ice grows fast, thick slowly), slower than lakes; melts at 15 mm/°C·day |
| **Lake ice** | below 0 °C | Stefan's law, α ≈ 2.4 cm/√(°C·day) |
| **River ice** | below 0 °C | like lakes, slower (flowing water) |
| **Snow** | precipitation at ≤ +1 °C | melts at 4 mm/°C·day |
| **Glaciers** | snow beyond 1 m water equivalent that outlasts the summers | melt at 7 mm/°C·day once bare; creep (≤ 10 mm/day) toward thinner ice and calve into seas and lakes |
| **Frozen ground** | frost depth by Stefan's law, 4× slower under snow | in summer an active layer thaws from the top |
| **Permafrost** | ground whose summer thaw never reached the bottom of the frost all year | |

Ice and snow reflect sunlight and cool their tile (glacier −4 °C, thick sea
ice −2 °C, snow −1.5 °C). That albedo feedback keeps the polar caps
self-sustaining. At world creation the climate is spun up for 4 years.
Glaciers are seeded after year 2 wherever snow survived the summer, with
300 years of that year's surplus. So the polar caps (perennial sea ice and
ice sheets) are in place at turn 0. Typical worlds end up with about 7% of
the ocean under sea ice, about 10% of land under glaciers and 15-20% in
permafrost, close to Earth's numbers.

## Fertility

`src/fertility.rs`: every land tile has a fertility from 0 to 1000:

```
fertility = soil × water × warmth
```

- **Soil** is fixed at world creation. By biome: plains 70%, hills 45%,
  sand 15%, mountains 8%. River tiles get +25% (alluvium) and land next to
  rivers or lakes +12%. Steep relief loses soil. Regional noise adds
  variation (think loess or volcanic ash).
- **Water** comes from rainfall, or from a river or lake to irrigate from.
  Very wet land is slightly leached.
- **Warmth** comes from last year's growing degree-days (warmth above
  5 °C), so a cold year (an oscillation cold phase, say) lowers the next
  year's fertility.
- **Ice**: permafrost cuts fertility to 30% (tundra). Glaciers and water are
  barren.

Fertility is recomputed at the start of every year. Starting cities are
placed with probability proportional to fertility and at least
`min_city_distance` tiles apart.

![Fertility, seed 3](docs/example-fertility.png)

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
rivers never end on dry land; `tests/climate.rs` checks the climate
(colder poles and mountains, harsher interiors, desert vs sea day/night range,
where the oscillation is felt, polar caps, seasonal sea ice, permafrost
zones, fertility rules).

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
| `src/city.rs` | City components, `found_initial_cities`, the `grow_cities` system |
| `src/time.rs` | `Turn` counter, `Date`, seasons and times of day |
| `src/climate/mod.rs` | `Climate` resource: tile climate profiles, temperatures, `update_climate` system |
| `src/climate/oscillation.rs` | ENSO-like oscillator and global drift |
| `src/climate/cryosphere.rs` | Sea/lake/river ice, snow, frozen ground, permafrost, glaciers |
| `src/fertility.rs` | `Fertility` resource and yearly `update_fertility` system |
| `src/climate/trig.rs` | Integer sine/cosine |
| `src/events.rs` | `EventLog` of notable happenings |
| `src/main.rs` | Command-line runner |

A turn runs: `advance_turn` → `update_climate` (temperatures, then ice and
snow) → `update_fertility` (at each new year) → `grow_cities`. City growth is
3% per year by default.
