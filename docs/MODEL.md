# The world model: laws, sources and our own simplifications

This document lists every formula in the simulation and where it comes from.
Each subsystem has two parts:

- **Established**: a physical law, a published algorithm, or an empirical
  rule from the scientific literature, with the source. Numbers taken from
  the literature are marked as such.
- **Our own**: modeling choices made for this project, such as
  simplifications, shapes, thresholds and constants chosen by hand. They were
  calibrated by comparing generated worlds against Earth statistics
  (listed in §14) and by looking at the maps. They are plausible, not
  measured, and they are the first place to look when tuning.

All simulation math is integer or fixed-point (16.16), so results are
bit-identical on every machine. Where a formula below uses real numbers, the
code uses a fixed-point equivalent.

---

## 1. Randomness and determinism

| | |
|---|---|
| Established | **xoshiro256\*\*** PRNG (Blackman & Vigna, *Scrambled linear pseudorandom number generators*, 2018), seeded with **SplitMix64** (Steele, Lea & Flood, *Fast splittable pseudorandom number generators*, 2014). Unbiased bounded integers by rejection sampling. **FNV-1a** hash (Fowler, Noll & Vo) for the state digest. |
| Our own | One root stream per world. The climate oscillator, the drift, the weather, the volcanoes and city founding each get their own child stream, so one subsystem never shifts another's random numbers. ECS systems run single-threaded in a fixed order, and anything order-sensitive is sorted by a stable id. |

## 2. Terrain (`src/map/terrain.rs`, `src/map/noise.rs`)

| | |
|---|---|
| Established | **Value noise** with smoothstep interpolation, summed into **fractional Brownian motion** (octaves at double frequency and half amplitude) and **ridged multifractal** noise for mountain crests (Musgrave, in Ebert et al., *Texturing & Modeling: A Procedural Approach*, 1994/2003). **Domain warping** (offsetting the sample point by another noise field), as popularized by Iñigo Quilez. |
| Our own | **Continents:** a warped fBm field with a border falloff (an eighth of the map), so continents never touch the map edge. **Sea level:** chosen as the percentile that makes exactly `land_per_mille` (default 33%) of the map land. Earth is about 29%. **Land profile:** `2 + max/12·r + max/8·r²` meters, where `r` is 0 at the coast and 1 at the most inland point. On top of that come ±150 m regional bumps, ±40 m tile-scale roughness (fixed at 6 tiles per noise cell, so rivers meander at any map size), and mountains `max·ridge³` inside noisy "orogenic belts". **Sea floor:** a 20-170 m continental shelf, then a smoothstep drop to −5500 m. |

## 3. Rainfall, oceans, lakes, rivers (`src/map/hydrology.rs`)

| | |
|---|---|
| Established | **General circulation:** Hadley, Ferrel and polar cells give wet tropics, dry subtropics around 30°, wet mid-latitudes and dry poles, with **prevailing winds** blowing westward below 30° (trade winds), eastward from 30° to 60° (westerlies) and westward beyond (polar easterlies). Moist air comes off the sea and dries as it crosses land. Mountains force air up, so their windward slopes get **orographic rain** and the lee lies in a **rain shadow**. The tropical rain belt (ITCZ) follows the **thermal equator**, which is displaced toward the hemisphere with more land (land heats more than sea) and wanders with longitude, so tropical biomes are never straight bands. **Cold upwelling currents** along subtropical west coasts make coastal deserts (Atacama, Namib, Baja California). **Priority-Flood** depression filling and drainage ordering (Barnes, Lehman & Mulla, *Priority-Flood: An optimal depression-filling and watershed-labeling algorithm for digital elevation models*, Computers & Geosciences, 2014). **D8 steepest-descent** flow routing (O'Callaghan & Mark, 1984): each cell drains to the neighbor with the largest drop per distance (diagonals at √2). **Flow accumulation**: each cell passes its own runoff plus everything upstream to its receiver. |
| Our own | **Thermal equator:** per tile, a pull toward the hemisphere with more tropical land at that longitude (land imbalance within ±30°, over a window of ±1/24 of the map width, up to about 10°) plus a noise wander of typically ±5-8° varying over a few hundred kilometers, clamped to ±15°. Tropical latitudes are shifted by it fully within 25° of the equator, fading out by 45°. **Regional land share:** land within ±1/24 of the map width (about 1700 km), used for monsoon strength (§6). **Moisture (0-1000):** a latitude band table (`latitude_rainfall`), read at the shifted tropical latitude. It is reduced by up to 60% with distance from the sea, measured as (upwind land distance + 2 × straight-line distance)/3 and saturating at a fifth of the map width. Rain shadow takes up to −60% when the highest ground upwind exceeds the tile's own height by 300 m or more (full at 2800 m). Orographic rain adds up to +30% for 1 km of rise against the wind. Coastal land with sea within two tiles to the west, at 12-32° from the equator, loses up to 45% (cold currents). Then ±45% noise. Winds are purely east-west and the map edge counts as open sea. **Oceans:** below-sea-level basins that touch the map edge or are at least 1/256 of the map. **Lakes:** a depression becomes a lake only where water would stand at least 100 m deep, over 3 or more tiles. Shallower parts are filled in to flat land. **Flats:** on flat ground (lakes, filled pits), drainage follows the flood order, so nothing is ever trapped. **Rivers:** form where the upstream catchment exceeds 25 "tiles of full rainfall" (on a 256×128 map, scaled with map area). |

## 4. Landform and soil (`src/map/landform.rs`)

Fixed at world creation. Entirely **our own** rule set:

- **Landform:**
  - Water: ocean or lake.
  - Mountains: at least 1700 m.
  - Hills: at least 800 m, or local relief of at least 450 m.
  - Plains: everything else.
- **Soil:**
  - Alluvial: river tiles, and floodplains next to a river below 500 m.
  - Rocky: mountains, or relief of at least 900 m.
  - Sandy: coastal land at or below 15 m with moisture under 500 (beaches), or land with moisture under 220 (dunes, desert sand).
  - Loam: everything else.

## 5. Climate: temperature (`src/climate/mod.rs`)

Each tile gets a static profile. Its temperature (tenths of a °C) at a turn is:

```
T = annual mean + seasonal swing·cos(2π(t − solstice − lag)/year)
  + diurnal range·daylight factor·share(time of day)
  + oscillation index·local weight + drift·polar amplification
  + weather anomaly − volcanic aerosol cooling − ice/snow albedo cooling
```

| Effect | Established | Our own |
|---|---|---|
| **Latitude** | Earth's zonal-mean surface temperature profile (about 27 °C at the equator, about −25 °C at the poles) and the growth of seasonal amplitude with latitude are standard climatology. | The two lookup tables (`ZONAL_MEAN`, `ZONAL_SWING`) are hand-fitted approximations of that climatology, not taken from a dataset. They are read at the latitude shifted by the thermal equator (§3), so tropical isotherms bend too. |
| **Altitude** | Environmental **lapse rate of 6.5 °C/km**, from the International Standard Atmosphere (ICAO / ISO 2533). Applied to the ground surface, or to the water surface for lakes. | n/a |
| **Seasons** | Insolation follows the sun's declination, which is close to a sinusoid over the year. The southern hemisphere is in antiphase. Water bodies lag the solstice by more than land. | A pure cosine. Lag of 25-40 days on land (more toward the coast), 40 on lakes, 50 at sea. Amplitude is 30% of the land value at sea, 60% on lakes, and `45% + 55%·continentality` on land, raised by up to 20% for dryness and cut by 10% next to rivers and lakes. |
| **Continentality** | Maritime vs continental climates. The concept of a continentality index goes back to Gorczyński (1920). | Computed as the cost of a shortest path (Dijkstra) from the ocean: 1 per tile (√2 diagonally), plus a mountain barrier cost for every 500 m of surface above 1000 m. This is full continentality at 1500 km. The map width is taken as a 40 000 km circumference. |
| **Day and night** | Diurnal ranges are largest in dry, clear, high places and smallest over water. With the sun always up or always down, there is little daily cycle. | Shares of the daily range per turn: morning −15%, day +45%, evening +10%, night −40% (they average to zero). Base range 6 °C, +12 °C for full dryness, +2.5 °C per km of altitude, scaled by continentality, ×0.75 near fresh water. 1 °C at sea, 3 °C on lakes. Daylight factor: `0.3 + 0.7 · 4L(24−L)/24²` for day length L hours (full at 12 h, 30% in polar night or midnight sun). |
| **Day length** | Solar declination `δ = 23.44° · sin(2π·day/year)` (a first-order approximation that ignores orbital eccentricity) and the **sunrise equation** `cos H₀ = −tan φ · tan δ`, day length `= 2H₀`. Polar night and midnight sun when `|cos H₀| ≥ 1`. arccos by **Abramowitz & Stegun 4.4.45** (error < 7·10⁻⁵ rad). | Day 0 is the northern vernal equinox. |
| **Dryness** | Deserts are hotter than humid land at the same latitude. | +up to 2 °C on the annual mean, only below 40° latitude. Cold interiors get up to −4 °C at high latitude. |
| **Ocean heat transport** | Ocean currents carry heat poleward, so high-latitude seas are milder than the zonal mean (the North Atlantic is the textbook case). | The `OCEAN_WARMTH` table (up to +7.5 °C at 60-70°), plus an irregular noise pattern of warm and cold "currents" (strongest at 50-80°, half strength on coasts, fading inland). The pattern is what makes the sea-ice edge ragged. |
| **Sine** | **Bhaskara I's rational approximation** of sine (7th century): `sin x ≈ 16x(π−x) / (5π² − 4x(π−x))`, max error about 0.0016. Used because `f64::sin` is not guaranteed to round identically on every platform. | n/a |

### 5.1 The ENSO-like oscillation (`src/climate/oscillation.rs`)

| | |
|---|---|
| Established | The **recharge oscillator** model of ENSO (Jin, *An equatorial ocean recharge paradigm for ENSO*, J. Atmos. Sci., 1997). An east-Pacific temperature anomaly `T` and a thermocline depth or heat content `h` drive each other a quarter cycle apart: `dT/dt = −λT + ωh`, `dh/dt = −ωT − λh`. Random **westerly wind bursts** keep the cycle going and make it irregular, the "stochastically forced, damped" view of ENSO. The stationary variance of such a linear system with noise σ on one component is `σ²/(4λ)` per component. That is standard linear stochastic-process theory, and it is used to set σ from the target amplitude. During El Niño the central and eastern tropical Pacific gets wetter, while the western side (Indonesia, Australia) dries. |
| Our own | **Parameters:** period 4 years, damping time 3 years, target standard deviation 1 °C. Warm or cold phase is declared at ±0.5 °C (inspired by NOAA's ±0.5 °C Niño-3.4 threshold, though not its 3-month averaging). **Integration:** semi-implicit Euler at daily steps, with the noise on `h`, so the surface index itself evolves smoothly. **Spatial footprint:** a smooth bell over the tropics (fading to 0 at 30°), strongest around a randomly placed "warm pool" longitude and reversed on the far side of the planet (east-west seesaw). A weak noise teleconnection pattern covers the extratropics. **Rain:** precipitation changes by 40% of the local temperature weight per °C of index, clamped to ×0.2..×3. |

### 5.2 Slow climate drift

| | |
|---|---|
| Established | An **Ornstein-Uhlenbeck process**, `dx = −x/τ·dt + σ·dW`, with stationary standard deviation `σ·√(τ/2)`. Stochastic climate variability as integrated weather noise follows Hasselmann (*Stochastic climate models*, Tellus, 1976). **Polar amplification**: high latitudes warm and cool more than the global mean. |
| Our own | Timescale 150 years, standard deviation 0.8 °C, linear polar amplification from 1× at the equator to 2× at the poles. No radiative forcing, CO₂ or orbital cycles. With these defaults the drift is small over 50 years (a few tenths of a degree) and reaches about ±1–2 °C over centuries. |

### 5.3 Weather (`src/climate/weather.rs`)

| | |
|---|---|
| Established | Synoptic weather systems are about 1000-2000 km across, persist for several days, and vary most at mid and high latitudes over land. |
| Our own | Two smooth noise fields (temperature, precipitation) of about 27 cells across the planet. A new independent field starts every 4 days and each day blends linearly toward the next, so spells build up and fade. Typical temperature anomaly: 1 °C in the tropics rising to 5 °C beyond 50°, ×0.4 at sea and `×(0.6 + 0.4·continentality)` on land. Precipitation multiplier: 0.1 to 2.2 around 1. Sampled on a coarse grid (four points per finest feature) and interpolated bilinearly, for speed. |

## 6. Climate: precipitation (`src/climate/mod.rs`)

| | |
|---|---|
| Established | Earth's rainfall distribution is strongly skewed: a few rainforests get over 2000 mm while deserts get under 250. **Monsoons**: the tropical rain belt (ITCZ) follows the sun, giving the outer tropics a wet summer and a dry winter. **Mediterranean climates**: on subtropical west coasts, storms come in off the sea in winter and the subtropical high brings dry summers. Continental interiors get mostly **convective summer rain**. |
| Our own | **Long-term amount:** `m + m²/1000` mm/year for moisture `m` (1000 gives 2000 mm, 500 gives 750 mm, 150 gives 170 mm). **Seasonal cycle:** `1 + s·cos(...)` peaking 30 days after the northern summer solstice, where `s` (in the local hemisphere's sense) adds three terms. Monsoon: up to 65% at 10-22° from the thermal equator, 0 at it and at 30°, scaled by the regional land share (×0.15 over open sea, ×0.4 at 30% land, ×1 at 60%, ×1.4 at 85%): big continents get strong wet and dry seasons and turn to savanna, while maritime tropics stay wet all year and keep their rainforest. Mediterranean: −80% on windward coasts (full where the wind crosses no land, none beyond 8 tiles of land) between 25° and 48°, full at 33-42°. Convective: +45% × continentality between 30° and 90°. Then the oscillation (§5.1) and the weather (§5.3) multiply it. |

## 7. Soil water, rivers and floods (`src/climate/soil.rs`)

| | |
|---|---|
| Established | The **bucket model** of soil water (Manabe, *Climate and the ocean circulation*, Monthly Weather Review, 1969): soil holds water up to a field capacity, evaporation draws it down, the excess runs off. **Temperature-based potential evapotranspiration**, in the spirit of Hamon (1961) and Thornthwaite (1948): it rises with temperature and day length. Actual evaporation falls below potential as the soil dries. **Bankfull discharge**: a river overflows when flow exceeds what its channel carries, roughly its typical yearly peak. |
| Our own | **Capacity:** alluvial 200 mm, loam 150, sandy 60, rocky 50. Only precipitation above +1 °C (rain) and meltwater (§8) enter the soil. **Evapotranspiration:** `PET = 0.2 mm/°C/day × T × daylight/12 h`, and `AET = PET × min(1, water / 70% of capacity)`. **Routing:** runoff is routed down the whole drainage network once a day with no travel time. **Bankfull:** each tile's level climbs a quarter of the way toward any higher discharge and sinks back by 1/1820 a day (about five years). A **flood** is a day with discharge above 2× bankfull. |

## 8. What is frozen (`src/climate/cryosphere.rs`)

| Process | Established | Our own |
|---|---|---|
| **Freezing points** | Fresh water at 0 °C. Sea water at about **−1.8 °C** (salinity about 35 g/kg). | Ice forms when the tile's air temperature is below the freezing point. |
| **Ice growth on water** | **Stefan's law** (Stefan, 1891): with heat conducted through the ice, `h = α·√FDD`, where FDD is accumulated freezing degree-days. Incrementally, `dh = α²·dFDD / (2h)`. The empirical α for lakes is roughly 1.7-2.7 cm·(°C·day)^−½, depending on wind and snow (Michel, 1971; Ashton, 1986). | α = 2.4 cm for lakes. Sea ice grows at 70% of the lake rate (ocean heat, brine) and rivers at 80% (flowing water). Thickness is capped at 5 m. |
| **Snow and ice melt** | The **degree-day (temperature-index) melt model**: melt = DDF × positive degree-days. Literature ranges are roughly 3-6 mm/°C/day for snow and 6-20 for glacier ice (Hock, *Temperature index melt modelling in mountain areas*, J. Hydrology, 2003). | Snow 4 and glacier ice 7 mm/°C/day, which is within those ranges. Lake, river and sea ice 15 mm/°C/day, which is our own choice. Snow falls at or below +1 °C, from the precipitation of §6. Meltwater feeds the soil and rivers (§7). |
| **Frozen ground** | Frost penetration and active-layer thaw both follow the **Stefan solution**, `depth = α·√(degree-days)`. This is common in frozen-ground engineering and permafrost mapping. Snow cover insulates the ground. **Permafrost** is ground at or below 0 °C for at least two consecutive years (International Permafrost Association). | Frost α = 5 cm, cut 4× (α halved) under 10 cm of snow. Thaw α = 3 cm, as an active layer from the top. The ground counts as thawed only when the active layer reaches the bottom of the winter frost. Permafrost is flagged after **one** full year without thawing, not two. |
| **Glaciers** | Glaciers form where snow survives the summer and turns to firn and then ice. Real glacier flow follows Glen's flow law (strain rate ∝ stress³). | Snow beyond 1 m water equivalent converts to glacier ice. Instead of Glen's law, ice creeps by **capped linear diffusion**: (thickness difference)/20 000 per day, at most 10 mm of ice per neighbor per day (meters per year). Glaciers calve 1/20 000 of their thickness per day (same cap) into adjacent seas and lakes. A tile counts as glaciated at 1 m. |
| **Ice-albedo feedback** | Ice and snow reflect more sunlight, which cools the surface and helps ice persist. This feedback drives the classic energy-balance models (Budyko, 1969; Sellers, 1969). | Fixed local cooling instead of a radiation budget: glacier −4 °C, sea or lake ice thicker than 30 cm −2 °C, snow −1.5 °C, thin ice −1 °C. |
| **Spin-up** | Climate models are spun up to equilibrium before use. Ice-sheet models take thousands of years to equilibrate, so they use **accelerated (asynchronous) spin-up**: the slow ice is stepped with long time steps against a fixed climate. | 4 years of neutral climate (no oscillation or drift; no weather) at daily resolution before turn 0. The ice sheets are settled after year 2 and again at the end. First a **probe** finds each land tile's yearly surface mass balance (snowfall minus melt) as if it were under a glacier. The probe reproduces a real run: quarter-day steps with the daily cycle, weather, and glacier albedo, averaged over 3 years of weather. Then the glacier model (same creep and calving as the daily one) runs forward in **one-year steps** from that fixed balance until the total ice changes by less than 0.1% per century. That usually takes 500–1000 years and at most `glacier_settle_years` (20 000). The polar caps therefore start in equilibrium. Without the oscillation and drift, the ice volume changes by well under 1% over the first decades, against about −16% per decade with the old seeding. |

## 9. Yearly statistics and climate normals (`src/climate/stats.rs`)

| | |
|---|---|
| Established | **Climate normals**: long-term averages that a year's weather is judged against (WMO uses 30-year normals). **Growing degree-days** above a base temperature (here 5 °C) measure the growing season. **Frost-free season**: the time between the last spring and the first autumn frost. |
| Our own | Each year accumulates, per tile: temperature and precipitation per season, growing degree-days, soil wetness while warmer than 5 °C, flood days, and the longest run of frost-free nights. The run is counted around the turn of the year, so southern summers are not cut in two. Normals are an **exponential moving average with a ~10-year time constant**, not a 30-year window. The first year sets them directly. |

## 10. Biomes (`src/biome.rs`)

| | |
|---|---|
| Established | The **Köppen-Geiger climate classification** as formulated by Peel, Finlayson & McMahon (*Updated world map of the Köppen-Geiger climate classification*, HESS, 2007). E: warmest month < 10 °C. B (arid): annual precipitation < 10·P<sub>th</sub>, with P<sub>th</sub> = 2·MAT (+28 if ≥ 70% of rain falls in summer, +14 if neither half dominates). BW (desert) is under half of that. Hot when MAT ≥ 18 °C. A: coldest month ≥ 18 °C, with Af when the driest month ≥ 60 mm and Am when the driest month ≥ 100 − P/25. Cs: driest summer month < 40 mm and < ⅓ of the wettest winter month. C: coldest month > 0 °C. D: coldest month ≤ 0 °C. Typical vegetation of each climate (rainforest in Af, savanna in Aw, taiga in Dfc, and so on). |
| Our own | **Mapping climates to 16 biomes:** EF/glacier → ice sheet, ET → tundra (alpine on mountains), BW → hot/cold desert, BS → savanna (hot) or grassland, Af → tropical rainforest, Am → tropical seasonal forest, Aw → savanna, Cs → Mediterranean shrubland. Other C and D climates → temperate forest, or boreal forest in D when the warmest month is < 18 °C. **Additions not in Köppen:** rainforest (C with ≥ 2000 mm) and grassland (C or D with < 500 mm). Wetland: river tiles at or below 60 m with ≥ 4× the river threshold of flow, outside deserts and polar climates. Monthly values come from seasonal normals: the warmest and coldest month are `mean ± 0.989 × seasonal amplitude`, and the driest month is ⅓ of the driest season. Classified yearly from the normals, so biomes shift slowly. |

## 11. Fertility (`src/fertility.rs`)

| | |
|---|---|
| Established | **Growing degree-days** and **frost-free season length** both limit crops. Combining limiting factors (soil, water, temperature) multiplicatively follows the spirit of agro-ecological zoning (for example FAO GAEZ). Floodplains have rich alluvial soils, grassland chernozems are among the best soils, rainforest and boreal soils are poor, and wetlands need draining. Permafrost limits agriculture. Volcanic soils (andisols) are fertile in the long run, while fresh ash fall ruins crops. |
| Our own | **Formula:** `fertility = soil × water × warmth × vegetation`, each from 0 to 1000. **Soil:** plains 70%, hills 45%, mountains 8%. Alluvial +25%, sandy −45%, rocky −20%, lake shore +12%, minus relief/5, ±15% regional noise, +12% within 2 tiles of a volcano. **Water:** a table of long-term rainfall that saturates at moisture 650 (rivers and lakes guarantee at least 650) and drops 10% at 1000 for leaching. Without irrigation it is also multiplied by a **drought factor** from last year's growing-season soil wetness (25% when bone dry, 60% at 25%, full at 50% of capacity). **Warmth:** the lower of a growing-degree-day table (0 at 0, 50% at 1000, 100% from 2800 to 5000, 85% at 7000) and a **season-length** table (0 at 0 frost-free days, 15% at 60, 50% at 100, full at 150). **Vegetation:** grassland 100%, temperate forest and deserts 90%, savanna, seasonal forest and Mediterranean 82%, temperate rainforest 77%, tropical rainforest 68%, boreal 64%, wetland 55%, tundra and alpine 36%. **Ice and ash:** permafrost ×0.3, glaciers 0, then × (1 − ash damage). Recomputed every year. |

## 12. Volcanoes (`src/volcano.rs`)

| | |
|---|---|
| Established | **Volcanic arcs** form above subduction zones along continental margins (the Andes, Japan, Indonesia) and produce explosive eruptions. **Hotspots** (mantle plumes: Hawaii, Iceland, Yellowstone) are mostly effusive. The **Volcanic Explosivity Index** (Newhall & Self, 1982) sets erupted tephra volume at ≥ 0.01 km³ for VEI 3, rising tenfold per step to ≥ 100 km³ for VEI 7. Approximate global frequencies are a few VEI 3 a year, a VEI 4 every few to ten years, a VEI 5 every ~50 years, a VEI 6 every ~100-200 years and a VEI 7 about once per millennium. Large eruptions put **sulfate aerosols** into the stratosphere: Pinatubo (VEI 6, 1991) reached a global aerosol optical depth of about 0.15 and cooled the planet about 0.5 °C, and Tambora (VEI 7, 1815) caused the 1816 "year without a summer". Stratospheric aerosols wash out with an e-folding time of about a year. Tropical eruptions spread aerosols over both hemispheres, extratropical ones mostly over their own. Larger eruptions need larger magma chambers. |
| Our own | **Placement:** arcs on land ≥ 1200 m within 4 tiles of the ocean, 10 per 10 000 tiles. Hotspots anywhere, 2 per 10 000 tiles, up to VEI 4. All at least 3 tiles apart. **Magma:** heavy-tailed recharge (log-uniform over three orders of magnitude), sharing the long-term planet-wide erupted volume implied by the eruption rates. Arcs start with up to 5000 years of stored magma, hotspots up to 300. **Eruptions:** each day, per VEI 3-7, a chance from the planet-wide rate (default per millennium: 2000, 150, 20, 8, 1). The volcano is drawn by stored magma among those with enough magma for that size; if none qualifies, nothing erupts. **Aerosols:** global optical depth 0.01, 0.05, 0.15, 0.5 for VEI 4-7, split per hemisphere (tropical within 20°: both get the full value; otherwise 170% / 30%). Cooling is 3 °C per unit optical depth, approached with a 60-day lag, and decays with a one-year e-folding time. **Ash:** radius 15, 50, 150, 400 and 1000 km for VEI 3-7. Fertility damage at the volcano is 20%, 50%, 80%, 90% and 95%, halving toward the edge, with an e-folding time of 250 days. **Casualties:** 2%, 10%, 30%, 60% and 90% of a city's population at the volcano, halving toward the edge of the ash radius. |

## 13. Climate journal (`src/journal.rs`)

Entirely **our own** reporting rules, built on the normals of §9. Written once
per season. Regional weather is judged on a 16 × 8 grid of regions with at
least 12 land tiles each, and only after 20 years of record:

| Happening | Minor | Moderate | Major | Severe | Cataclysmic |
|---|---|---|---|---|---|
| Heat wave / cold spell (regional season mean vs normal) | ±1.5 °C | ±2.5 | ±3.5 | ±5 | ±8 |
| Drought (regional season rain, % of normal; ≥ 50 mm expected) | ≤ 60% | ≤ 45 | ≤ 30 | ≤ 20 | ≤ 10 |
| Deluge (% of normal) | ≥ 170% | ≥ 200 | ≥ 250 | ≥ 300 | ≥ 400 |
| Floods (river flood days in a region) | ≥ 40 | ≥ 100 | ≥ 250 | ≥ 600 | ≥ 1500 |
| Warm / cold oscillation phase (index) | ±1.0 °C | ±1.5 | ±2.0 | ±2.5 | ±3.0 |
| Volcanic winter (hemispheric aerosol cooling) | 0.3 °C | 0.6 | 1.0 | 1.5 | 2.5 |
| Eruption | VEI 3 with deaths | VEI 4 | VEI 5 | VEI 6 | VEI 7 |

Eruptions with 1000+ deaths are raised one level. Oscillation episodes and
volcanic winters are logged when they reach a new level, not every season.
When more than four regions report the same kind of anomaly in one season,
they are merged into one widespread entry (highest severity, worst region's
value, number of regions, and the hemisphere if they share one).

## 14. Cities (`src/city.rs`)

| | |
|---|---|
| Established | **Logistic growth** (Verhulst, 1838): `dP/dt = rP(1 − P/K)`. |
| Established | The first cities (around 4500 BCE: Eridu and Uruk, the Nile valley, Mehrgarh, the Yellow and Yangtze rivers, Çatalhöyük, Varna) grew on fertile river floodplains and lake or sea shores, in warm-temperate to subtropical climates. The richest were rivers through dry country, where irrigation produced surpluses. Everyone else lived in villages and tribes with simpler technology. |
| Our own | r = 3% a year, K = 10 000, integrated exactly with integer remainders every turn. Settlements are placed only at world creation, never on glaciers or neighbouring tiles, and at least 800 km apart: the distance is measured on the globe, east-west scaled by cos(latitude). **Placement order:** 6 civilized cities first, then 10 barbarian tribes. **Civilized cities** need a cradle site, drawn with probability ∝ score². A cradle site has: no rainforest, boreal, tundra, cold desert, alpine or ice; a mean temperature of 10–25 °C; and a river, lake or coast on or next to the tile. **Score:** fertility (0–1000) + 500 for a river + 200 for a lake + 100 for a coast + 300 for alluvial soil + 300 for a river through desert, savanna, grassland or Mediterranean shrubland. Once cradle sites run out, cities go anywhere fertile. **Barbarian tribes** go anywhere settleable, ∝ fertility + 1%. They draw their starting technologies at `start.share × barbarian_share[category]` (see `data/technologies.json`). |

## 15. Geometry

The map is **equirectangular**: every row has the same number of tiles, but a
row's true area is proportional to **cos(latitude)**. All planet-wide shares
(sea ice, glaciers, biomes, fertility) and the global mean temperature are
area-weighted by cos(latitude).

## 16. Calibration targets

Our own parameters were tuned until typical worlds matched these Earth
figures (shares by true area; three 256×128 worlds):

| Quantity | Earth (approximate) | Typical world |
|---|---|---|
| Land share | 29% | 33% (configurable) |
| Global mean surface temperature | about 15 °C | 16-17 °C |
| Sea ice, share of ocean area (annual mean) | about 7% | 7-9% |
| Glaciers, share of land area | about 10% | 7-11% |
| Permafrost, share of land area | about 15-24% of northern-hemisphere land (estimates vary) | 16-21% |
| Arid land (Köppen B: deserts, steppe, hot semi-arid) | about 30% | 15-25% (deserts 6-12%) |
| Tropical biomes (Köppen A) | about 19% | 35-50% (these worlds have much tropical land; seed-dependent; rainforest 16-20%) |
| Mediterranean (Cs) | about 2-3% | about 0.5% |
| ENSO period / typical anomaly | 2-7 years / about 1 °C | 2-7 years / about 1 °C |
| Eruptions VEI 3 / 4 / 5 / 6 per century | about 200 / 15 / 2 / 0.8 | 190 / 13 / 3 / 1.7 (300-year run) |

## 17. Known simplifications

- Winds are purely east-west, and there is no atmospheric or ocean
  circulation model. Their effects are baked into lookup tables and noise
  patterns.
- Rivers route their water instantly (no travel time) and are one tile wide
  at every resolution.
- Ice sheets do not change surface altitude, so there is no lapse-rate
  feedback.
- Volcanoes do not change the terrain.
- Climate normals are a 10-year moving average, and regional extremes are
  judged after 20 years of record. Early regional records are noisy.
- Permafrost ratchets: once a cold winter freezes the ground deeper than
  the next summer thaws, it stays frozen (there is no heat from below). So
  the permafrost area rises for the first decade or two after world
  creation (16% → 20% of land in figure 4).
- The map does not wrap east-west.

## People: tools (`src/tools.rs`)

| | |
|---|---|
| Established | Ground and flaked stone tools were the everyday kit of the first farmers; harvesting with flint-bladed sickles was several times faster than stripping by hand (Harlan, 1967, reaping wild einkorn). Nets, lines and hooks were in use long before farming. Copper tools appear in the 5th millennium BCE (Varna, Mehrgarh, Mesopotamia), first hammered from native copper and then smelted and cast; arsenical and tin bronze followed and were harder and kept an edge longer. |
| Established | Tree-felling experiments find metal axes cutting several times faster than stone ones (Saraydar and Shimada, 1971; Mathieu and Meyer, 1997), while for most other work the gain from a better tool is much smaller. |
| Our own | **Catalog** (`tools` in `data/people.json`), listed from least to most valued: stone tools (1 person-day to make, last 90 working days), flint sickle (needs cereal cultivation; 1 day, 120 days), fishing gear (needs fishing nets; 3 days, 120 days), copper tools (native copper or copper smelting; 4 days, 300 days), bronze tools (arsenical or tin bronze; 5 days, 600 days). **Output with the tool** (per mille, by work): stone 1050–1150, copper 1100–1300, bronze 1150–1450, the most for building and the least for herding and fishing; sickle 1300 for farming; fishing gear 1400 for fishing. Ores are not on the map yet, so metal tools need only the knowledge. |
| Our own | **The day** is counted at midday: whoever is foraging, farming, herding, fishing, building or crafting then has worked that day (people away with a caravan do nothing). City by city, in id order, a worker whose tool does not help with today's work takes the best one in the stores for it (putting theirs back, wear forgotten), and the tool they use wears by `1000 / durability` a day (the remainder spread by day number, so a tool lasts exactly its durability in working days) and is thrown away when broken. **Crafters** then put a day's labour (times their own tool's multiplier for crafting) into an item, pooled per city and item so half-done work carries over. Each makes the most valued item they personally know how to make that the city is short of: short means fewer in the stores (counting better items that do the same main job as well) than 10% of the workers it would help. If nothing is short, they make the most valued item anyone working there could use. Knowing copper smelting makes someone a smith in effect; no status is needed. |

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
