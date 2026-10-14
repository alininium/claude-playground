# People: design and work plan

Every human is an ECS entity. This document sets out how people are
simulated, which module owns what, and the rules every change must keep.
`src/people/mod.rs` holds the shared components; read it first.

## Rules (non-negotiable)

1. **Determinism.** The same seed gives the same history, bit for bit, on
   any machine, in debug and release builds, and on any number of threads.
   - Integer and fixed-point math only. No floats in simulation state, no
     `HashMap` iteration order, no wall-clock time.
   - Every person draws random numbers from their own `PersonRng` and from
     nothing else. Settlement- or world-level randomness uses a dedicated
     `SimRng` stream, or a hash of person seeds and the turn (as
     `people::census` does). Never draw from another person's stream.
   - Order-sensitive work goes through cities by `CityId` and people by
     `PersonId`.
2. **Per-person steps vs resolve steps.**
   - A per-person step reads only that person plus shared read-only data,
     and writes only that person. Such steps may run in parallel with
     `people::parallel::for_each`.
   - Anything involving two or more people or shared stock (land, food,
     tools, partners, caravans) is a resolve step: sequential, in id order.
     Resolve steps for different cities may run in parallel only if they
     touch nothing shared.
   - The test `the_number_of_threads_changes_nothing` (tests/people.rs) must
     keep passing.
3. **Same parents, same child.** A child's seed is
   `rng::child_seed(mother.seed, father.seed, mother.births)`. Everything
   about the child at birth (sex, traits) comes from its own stream seeded
   with it. Nothing else may influence the child.
4. **Cost.** People with nothing to do must cost next to nothing:
   - Decisions happen when `Activity::until` is reached, at most once per
     day, or earlier if a need crosses an urgent threshold. Night is sleep.
   - Young children follow their mother and don't decide.
   - Prefer daily work to per-turn work, and lazy updates to sweeps. A turn
     is a quarter day; a year is 364 days.
   - Budget: a day with 10 000 people should take well under 10 ms in
     release. Add a case to `tests/performance.rs` when you add per-person
     work.
5. **Data-driven tuning.** Numbers that a designer may want to change live
   in the module's own section of `data/people.json`, parsed with
   `people::section`. Defaults must be realistic; cite sources in
   docs/MODEL.md for established facts and mark the rest as our own choices.

## The turn

The schedule (`Simulation::from_world`) runs the world (volcanoes, climate,
journal, biomes, fertility), then one slot per module, in this order:

| Slot | Module | Owns |
|---|---|---|
| `needs` | `src/people/needs.rs` | how needs grow over time and with the weather, and their satisfaction by rest, company, fun… |
| `decide` | `src/people/decide.rs` | choosing actions (utility scoring), the action catalog, the decision schedule |
| `economy` | `src/economy.rs` | territory, land use, food production, eating and hunger, `Stores::food`, `Outlook` |
| `tools` | `src/tools.rs` | the item catalog, crafting (smiths and others), `Stores::items`, `Equipment`, wear, `ToolCatalog` |
| `life` | `src/people/life.rs` | ageing, mortality, pairing up, pregnancy, births, caring for children |
| `status` | `src/people/status.rs` | the status catalog unlocked by technologies, who holds which status, learning technologies, `StatusCatalog` |
| `caravan` | `src/caravan.rs` | forming caravans from people settling away, travel, founding new cities |
| `census` | `src/people/mod.rs` | deaths owed by disasters, city head counts |

Each module has `install(world)` (its resources) and `systems()` (its
systems, in order). Change only your own module's files, your section of
`data/people.json`, your own test file `tests/<module>.rs`, and your section
of docs/MODEL.md. If you need something from another module, use the hooks
below. If a hook is missing, add the smallest one you need to *your* module
and say so in your summary.

## Hooks between modules

These exist already with neutral defaults, so modules can be built in
parallel:

- **`economy::Outlook`** (a component on each city): `free_land` (split
  into `free_farm` and `free_pasture`), `food_days`, `tools_available` and
  `off_season`. Written by `economy` and `tools`, read by `decide`: Farm
  needs room on the fields and a season in which they grow, Herd room on
  pasture, and `SettleAway` no land at all (or a city over its organisation
  limit); low `food_days` make food work urgent for everyone (providing).
- **`economy::Larder::residents`** and the city's `Knowledge`: read by
  `decide` for the organisation limit (`actions.organisation` in
  `data/people.json`).
- **`economy::Stores`** (a component on each city): `food`, owned by
  `economy`, and `items[ItemId]`, owned by `tools`.
- **`status::StatusCatalog`** (a resource): `allows(status, action)` and
  `leaning(status, action)`. Read by `decide`.
- **`tools::ToolCatalog`** (a resource): `work_multiplier(equipment,
  action)`, per mille. Read by `economy` (and anyone whose work tools speed
  up).
- **`Activity`** (on each person): the action chosen by `decide`. The
  resolve steps act on it: `economy` on Forage, Farm, Herd and Fish;
  `tools` on Craft; `life` on Court and CareForChildren; `caravan` on
  SettleAway.
- **Needs ownership:**

  | Need | Grows | Is satisfied by |
  |---|---|---|
  | Food | `needs` | `economy` (eating) |
  | Rest, Warmth, Safety, Company, Fun | `needs` | `needs` (by the action done) |
  | Family | `needs` | `life` (partner and children) |
  | Esteem | `needs` | `status` |

- **`city::NextCityId`** and **`people::NextPersonId`**: allocate ids for
  new cities (caravans) and people (births).
- **`Population::pending_deaths`**: deaths a disaster owes a city; the
  census picks who dies. `life` may reuse or replace this.

## Contracts found while merging

These come from how the modules turned out. Keep them in mind when changing
any of them.

- **Night is sleep.** On the Night turn `decide` sets every available
  person's activity to Sleep and records the day's action in
  `decide::Plan::last`. A resolve step that acts on "what people did today"
  should run on waking turns or read `Plan::last`, as `caravan` does: a
  caravan leaves with the adults whose `Plan::last` is SettleAway, and their
  `Plan::last` is cleared when they go, so they don't leave again on
  arrival.
- **Travellers.** People with a caravan have `Activity::until ==
  u64::MAX`, and their `Person::home` is the caravan's id plus
  `caravan::TRAVELLING` (2³¹), which no city has. Never index a city
  table by `home` without a bounds check; use `caravan::is_travelling`.
- **Leaving breaks ties to the city.** When someone leaves with a caravan,
  a partner who stays is no longer their partner (both may pair again), and
  any status not held for life stays with the city. Statuses never
  follow people abroad.
- **Everyone's stream moves.** `life` rolls survival from each person's
  own `PersonRng` daily, so tests should check behaviour (activities,
  needs), not that a stream stayed untouched.
- **Fire and shelter.** Warmth grows 4 a day per °C of night below 12 °C,
  and a night by the fire takes back 200, so only ice-sheet or tundra cold
  overwhelms people. With the first draft's 12 and 60, a fifth of the
  founders' children died of cold in the first year.
- **Spare tools.** With nothing short, crafters only make spares until a
  city holds half as many of an item as it has people (`spare_per_mille`).
  Without that limit, stores grew by thousands of tools a year.
- **Tests that pin people.** To hold people to an action, set
  `Activity::until` far ahead (not `u64::MAX`, which means away) and raise
  `decide::ActionCatalog::urgent` above 1000, or they re-decide as soon as
  a need becomes urgent.

`cargo run --release --example society -- [seed] [years]` prints a yearly
summary: people, births, deaths, food, land, crafting, statuses and tools.
It's the quickest way to see whether the modules still add up to a
plausible society.

## Starting state

Settlements start with 200–500 (civilized) or 50–200 (barbarian) people,
scaled by the fertility of their land (`SimConfig::city_population` and
`barbarian_population`). Founders come as households
(`people::founding`):

- ages follow a pre-modern pyramid
- women are paired with slightly older men
- children belong to mothers of a plausible age
- adults know about 70% of their settlement's technologies (if they know
  the prerequisites); children know only the baseline

## Tools

Tools (stone, copper, bronze…) are made by people whose action is Craft
and who know the right technologies: a smith with copper smelting makes
copper tools. Tools go into the city's `Stores::items`. People take one
into their `Equipment`, which multiplies their work through `ToolCatalog`,
and wear it out.

## Testing

Each module gets `tests/<module>.rs` in the **people** category of
`tools/test.sh`; add its file name to the `people` line there. Check
behaviour (hungry people seek food, overcrowding sends caravans out), not
just plumbing. Keep debug-mode test time low: use the fixtures (`common::sim`)
and short runs.
