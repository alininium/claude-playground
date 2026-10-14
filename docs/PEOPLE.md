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

- **`economy::Outlook`** (a component on each city): `free_land`,
  `food_days` and `tools_available`. Written by `economy` and `tools`, read
  by `decide`, for example to choose `SettleAway` when there is no land.
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
