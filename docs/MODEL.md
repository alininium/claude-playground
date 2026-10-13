# The world model: laws, sources and our own simplifications

This document lists every formula in the simulation and where it comes from.
Each subsystem has two parts:

- **Established**: a physical law, a published algorithm, or an empirical
  rule from the scientific literature, with the source. Numbers taken from
  the literature are marked as such.
- **Our own**: modeling choices made for this project, such as
  simplifications, shapes, thresholds and constants chosen by hand. They were
  calibrated by comparing generated worlds against Earth statistics
  (listed at the end) and by looking at the maps. They are plausible, not
  measured, and they are the first place to look when tuning.

All simulation math is integer or fixed-point (16.16), so results are
bit-identical on every machine. Where a formula below uses real numbers, the
code uses a fixed-point equivalent.

---

## 1. Randomness and determinism

| | |
|---|---|
| Established | **xoshiro256\*\*** PRNG (Blackman & Vigna, *Scrambled linear pseudorandom number generators*, 2018), seeded with **SplitMix64** (Steele, Lea & Flood, *Fast splittable pseudorandom number generators*, 2014). Unbiased bounded integers by rejection sampling. **FNV-1a** hash (Fowler, Noll & Vo) for the state digest. |
| Our own | One root stream per world. The climate oscillator and the drift get their own child streams, so climate never shifts other subsystems' random numbers. ECS systems run single-threaded in a fixed order, and anything order-sensitive is sorted by a stable id. |

## 2. Terrain (`src/map/terrain.rs`, `src/map/noise.rs`)

| | |
|---|---|
| Established | **Value noise** with smoothstep interpolation, summed into **fractional Brownian motion** (octaves at double frequency and half amplitude) and **ridged multifractal** noise for mountain crests (Musgrave, in Ebert et al., *Texturing & Modeling: A Procedural Approach*, 1994/2003). **Domain warping** (offsetting the sample point by another noise field), as popularized by Iñigo Quilez. |
| Our own | **Continents:** a warped fBm field with a border falloff (an eighth of the map), so continents never touch the map edge. **Sea level:** chosen as the percentile that makes exactly `land_per_mille` (default 33%) of the map land. Earth is about 29%. **Land profile:** `2 + max/12·r + max/8·r²` meters, where `r` is 0 at the coast and 1 at the most inland point. On top of that come ±150 m regional bumps, ±40 m tile-scale roughness (fixed at 6 tiles per noise cell, so rivers meander at any map size), and mountains `max·ridge³` inside noisy "orogenic belts". **Sea floor:** a 20-170 m continental shelf, then a smoothstep drop to −5500 m. |

## 3. Water: oceans, lakes, rivers (`src/map/hydrology.rs`)

| | |
|---|---|
| Established | **Priority-Flood** depression filling and drainage ordering (Barnes, Lehman & Mulla, *Priority-Flood: An optimal depression-filling and watershed-labeling algorithm for digital elevation models*, Computers & Geosciences, 2014). **D8 steepest-descent** flow routing (O'Callaghan & Mark, 1984): each cell drains to the neighbor with the largest drop per distance (diagonals at √2). **Flow accumulation**: each cell passes its own runoff plus everything upstream to its receiver. |
| Our own | **Oceans:** below-sea-level basins that touch the map edge or are at least 1/256 of the map. **Lakes:** a depression becomes a lake only where water would stand at least 100 m deep, over 3 or more tiles. Shallower parts are filled in to flat land. **Flats:** on flat ground (lakes, filled pits), drainage follows the flood order, so nothing is ever trapped. **Rivers:** form where the upstream catchment exceeds 25 "tiles of full rainfall" (on a 256×128 map, scaled with map area). **Rainfall:** see §5. |

## 4. Biomes (`src/map/biomes.rs`)

Entirely **our own** rule set:

- **Water:** ocean or lake.
- **Mountains:** at least 1700 m.
- **Hills:** at least 800 m, or local relief of at least 450 m.
- **Sand:** coastal land at or below 15 m with moisture under 500 (beaches), or land with moisture under 220 and no river (deserts).
- **Earth:** everything else.

## 5. Climate (`src/climate/mod.rs`)

Each tile gets a static profile. Its temperature (tenths of a °C) is then:

```
T = annual mean + seasonal swing·cos(2π(t − solstice − lag)/year)
  + diurnal range·share(time of day) + oscillation index·local weight
  + drift·polar amplification − ice/snow albedo cooling
```

| Effect | Established | Our own |
|---|---|---|
| **Latitude** | Earth's zonal-mean surface temperature profile (about 27 °C at the equator, about −25 °C at the poles) and the growth of seasonal amplitude with latitude are standard climatology. | The two lookup tables (`ZONAL_MEAN`, `ZONAL_SWING`) are hand-fitted approximations of that climatology, not taken from a dataset. |
| **Altitude** | Environmental **lapse rate of 6.5 °C/km**, from the International Standard Atmosphere (ICAO / ISO 2533). Applied to the ground surface, or to the water surface for lakes. | n/a |
| **Seasons** | Insolation follows the sun's declination, which is close to a sinusoid over the year. The southern hemisphere is in antiphase. Water bodies lag the solstice by more than land. | A pure cosine. Lag of 25-40 days on land (more toward the coast), 40 on lakes, 50 at sea. Amplitude is 30% of the land value at sea, 60% on lakes, and `45% + 55%·continentality` on land, raised by up to 20% for dryness and cut by 10% next to rivers and lakes. |
| **Continentality** | Maritime vs continental climates. The concept of a continentality index goes back to Gorczyński (1920). | Computed as the cost of a shortest path (Dijkstra) from the ocean: 1 per tile (√2 diagonally), plus a mountain barrier cost for every 500 m of surface above 1000 m. This is full continentality at 1500 km. The map width is taken as a 40 000 km circumference. |
| **Day and night** | Diurnal ranges are largest in dry, clear, high places and smallest over water. | Shares of the daily range per turn: morning −15%, day +45%, evening +10%, night −40% (they average to zero). Base range 6 °C, +12 °C for full dryness, +2.5 °C per km of altitude, scaled by continentality, ×0.75 near fresh water. 1 °C at sea, 3 °C on lakes. |
| **Dryness** | Deserts are hotter than humid land at the same latitude. | +up to 2 °C on the annual mean, only below 40° latitude. Cold interiors get up to −4 °C at high latitude. |
| **Ocean heat transport** | Ocean currents carry heat poleward, so high-latitude seas are milder than the zonal mean (the North Atlantic is the textbook case). | The `OCEAN_WARMTH` table (up to +7.5 °C at 60-70°), plus an irregular noise pattern of warm and cold "currents" (strongest at 50-80°, half strength on coasts, fading inland). The pattern is what makes the sea-ice edge ragged. |
| **Rainfall** | The general circulation (Hadley, Ferrel and polar cells) gives wet tropics, dry subtropics around 30°, wet mid-latitudes and dry poles. Rainfall falls off inland. | `latitude_rainfall` band table, −65% at full inland distance (a fifth of the map width), ±45% noise. Rainfall is static: it does not change with seasons or the oscillation. |
| **Sine** | **Bhaskara I's rational approximation** of sine (7th century): `sin x ≈ 16x(π−x) / (5π² − 4x(π−x))`, max error about 0.0016. Used because `f64::sin` is not guaranteed to round identically on every platform. | n/a |

### 5.1 The ENSO-like oscillation (`src/climate/oscillation.rs`)

| | |
|---|---|
| Established | The **recharge oscillator** model of ENSO (Jin, *An equatorial ocean recharge paradigm for ENSO*, J. Atmos. Sci., 1997). An east-Pacific temperature anomaly `T` and a thermocline depth or heat content `h` drive each other a quarter cycle apart: `dT/dt = −λT + ωh`, `dh/dt = −ωT − λh`. Random **westerly wind bursts** keep the cycle going and make it irregular, the "stochastically forced, damped" view of ENSO. The stationary variance of such a linear system with noise σ on one component is `σ²/(4λ)` per component. That is standard linear stochastic-process theory, and it is used to set σ from the target amplitude. |
| Our own | **Parameters:** period 4 years, damping time 3 years, target standard deviation 1 °C. Warm or cold phase is declared at ±0.5 °C (inspired by NOAA's ±0.5 °C Niño-3.4 threshold, though not its 3-month averaging). **Integration:** semi-implicit Euler at daily steps, with the noise on `h`, so the surface index itself evolves smoothly. **Spatial footprint:** a smooth bell over the tropics (fading to 0 at 30°), strongest around a randomly placed "warm pool" longitude and reversed on the far side of the planet (east-west seesaw). A weak noise teleconnection pattern covers the extratropics. |

### 5.2 Slow climate drift

| | |
|---|---|
| Established | An **Ornstein-Uhlenbeck process**, `dx = −x/τ·dt + σ·dW`, with stationary standard deviation `σ·√(τ/2)`. Stochastic climate variability as integrated weather noise follows Hasselmann (*Stochastic climate models*, Tellus, 1976). **Polar amplification**: high latitudes warm and cool more than the global mean. |
| Our own | Timescale 150 years, standard deviation 0.5 °C, linear polar amplification from 1× at the equator to 2× at the poles. No radiative forcing, CO₂ or orbital cycles. With these defaults the drift is nearly invisible over 50 years (see the figures). |

## 6. What is frozen (`src/climate/cryosphere.rs`)

| Process | Established | Our own |
|---|---|---|
| **Freezing points** | Fresh water at 0 °C. Sea water at about **−1.8 °C** (salinity about 35 g/kg). | Ice forms when the tile's air temperature is below the freezing point. |
| **Ice growth on water** | **Stefan's law** (Stefan, 1891): with heat conducted through the ice, `h = α·√FDD`, where FDD is accumulated freezing degree-days. Incrementally, `dh = α²·dFDD / (2h)`. The empirical α for lakes is roughly 1.7-2.7 cm·(°C·day)^−½, depending on wind and snow (Michel, 1971; Ashton, 1986). | α = 2.4 cm for lakes. Sea ice grows at 70% of the lake rate (ocean heat, brine) and rivers at 80% (flowing water). Thickness is capped at 5 m. |
| **Snow and ice melt** | The **degree-day (temperature-index) melt model**: melt = DDF × positive degree-days. Literature ranges are roughly 3-6 mm/°C/day for snow and 6-20 for glacier ice (Hock, *Temperature index melt modelling in mountain areas*, J. Hydrology, 2003). | Snow 4 and glacier ice 7 mm/°C/day, which is within those ranges. Lake, river and sea ice 15 mm/°C/day, which is our own choice. Snow falls at or below +1 °C. Precipitation is 2 mm/year per moisture point (moisture 1000 gives 2000 mm/year). |
| **Frozen ground** | Frost penetration and active-layer thaw both follow the **Stefan solution**, `depth = α·√(degree-days)`. This is common in frozen-ground engineering and permafrost mapping. Snow cover insulates the ground. **Permafrost** is ground at or below 0 °C for at least two consecutive years (International Permafrost Association). | Frost α = 5 cm, cut 4× (α halved) under 10 cm of snow. Thaw α = 3 cm, as an active layer from the top. The ground counts as thawed only when the active layer reaches the bottom of the winter frost. Permafrost is flagged after **one** full year without thawing, not two. |
| **Glaciers** | Glaciers form where snow survives the summer and turns to firn and then ice. Real glacier flow follows Glen's flow law (strain rate ∝ stress³). | Snow beyond 1 m water equivalent converts to glacier ice. Instead of Glen's law, ice creeps by **capped linear diffusion**: (thickness difference)/20 000 per day, at most 10 mm of ice per neighbor per day (meters per year). Glaciers calve 1/20 000 of their thickness per day (same cap) into adjacent seas and lakes. A tile counts as glaciated at 1 m. |
| **Ice-albedo feedback** | Ice and snow reflect more sunlight, which cools the surface and helps ice persist. This feedback drives the classic energy-balance models (Budyko, 1969; Sellers, 1969). | Fixed local cooling instead of a radiation budget: glacier −4 °C, sea or lake ice thicker than 30 cm −2 °C, snow −1.5 °C, thin ice −1 °C. |
| **Spin-up** | Climate models are spun up to equilibrium before use. | 4 years of neutral climate at daily resolution before turn 0. After year 2, every land tile whose snow survived the summer gets 300 years' worth of that surplus as glacier ice (up to 3 km). This is how the polar caps exist at turn 0. They keep adjusting for a few decades afterward (see figure 4). |

## 7. Fertility (`src/fertility.rs`)

| | |
|---|---|
| Established | **Growing degree-days** (warmth accumulated above a base temperature, here 5 °C), a standard agronomic and ecological measure of the growing season. Combining limiting factors (soil, water, temperature) multiplicatively follows the spirit of agro-ecological zoning (for example FAO GAEZ). Floodplains have rich alluvial soils. Permafrost limits agriculture. |
| Our own | **Formula:** `fertility = soil × water × warmth`, each from 0 to 1000. **Soil by biome:** plains 70%, hills 45%, sand 15%, mountains 8%. River tiles get +25%, land next to rivers or lakes +12%, minus relief/5, ±15% regional noise. **Water:** a piecewise table of rainfall that saturates at moisture 650 (rivers and lakes guarantee at least 650) and drops 10% at 1000 for leaching. **Warmth:** a table of last year's growing degree-days (0 at 0, 50% at 1000, 100% from 2800 to 5000, 85% at 7000). **Ice:** permafrost ×0.3, glaciers 0. Recomputed every year. |

## 8. Cities (`src/city.rs`)

| | |
|---|---|
| Established | **Logistic growth** (Verhulst, 1838): `dP/dt = rP(1 − P/K)`. |
| Our own | r = 3% a year, K = 10 000, integrated exactly with integer remainders every turn. Starting cities (24) are placed only at world creation, with probability proportional to fertility + 1%, at least 4 tiles apart (Chebyshev distance), never on glaciers. |

## 9. Geometry

The map is **equirectangular**: every row has the same number of tiles, but a
row's true area is proportional to **cos(latitude)**. All planet-wide shares
(sea ice, glaciers, fertility) and the global mean temperature are
area-weighted by cos(latitude).

## 10. Calibration targets

Our own parameters were tuned until typical worlds matched these Earth
figures:

| Quantity | Earth (approximate) | Typical world |
|---|---|---|
| Land share | 29% | 33% (configurable) |
| Global mean surface temperature | about 15 °C | 16-17 °C |
| Sea ice, share of ocean area (annual mean) | about 7% | 6-9% |
| Glaciers, share of land area | about 10% | 8-12% |
| Permafrost, share of land area | about 15-24% of northern-hemisphere land (estimates vary) | 15-20% |
| ENSO period / typical anomaly | 2-7 years / about 1 °C | 2-7 years / about 1 °C |

## 11. Known simplifications

- Temperature has no weather (no day-to-day noise or storms), only the
  deterministic cycles plus the two global oscillations.
- Rainfall is static: no seasons, no oscillation effect, no rain shadow.
- No atmospheric or ocean circulation is simulated. Their effects are
  baked into lookup tables and noise patterns.
- Ice sheets do not change surface altitude, so there is no lapse-rate
  feedback.
- Rivers are one tile wide at every resolution.
- The map does not wrap east-west.

## Figures

From `examples/figures.rs` + `tools/plot_figures.py` (seed 3, 256×128, 50
years):

1. `figures/1-enso-index.png`: the oscillation index over 50 years, and how
   the global mean temperature and sea ice follow it.
2. `figures/2-enso-maps.png`: the oscillation's footprint, and the
   temperature anomaly at the strongest warm and cold phases.
3. `figures/3-fifty-years-ice.png`: the same winter day in year 1 and
   year 51, and where ice and snow cover changed.
4. `figures/4-fifty-years-trends.png`: the 50-year difference in annual mean
   temperature, plus glaciers, permafrost, fertility and population.
