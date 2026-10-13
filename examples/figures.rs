//! Dumps the data behind the figures in `docs/figures/`: a 50-year run of
//! seed 3 on a 256x128 map. Render them with `tools/plot_figures.py`:
//!
//! ```sh
//! cargo run --release --example figures -- /tmp/fig
//! python3 tools/plot_figures.py /tmp/fig docs/figures
//! ```
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use societies::Simulation;
use societies::climate::cryosphere::{Cover, GLACIER};
use societies::config::SimConfig;
use societies::map::Water;
use societies::render;

fn write_i32(path: PathBuf, data: &[i32]) {
    let bytes: Vec<u8> = data.iter().flat_map(|v| v.to_le_bytes()).collect();
    fs::write(path, bytes).unwrap();
}

fn main() {
    let out = PathBuf::from(std::env::args().nth(1).expect("out dir"));
    fs::create_dir_all(&out).unwrap();
    let mut sim = Simulation::new(SimConfig {
        seed: 3,
        map_width: 256,
        map_height: 128,
        ..SimConfig::default()
    });
    let (w, h) = (sim.map().width(), sim.map().height());
    let n = sim.map().tile_count() as usize;
    let days = u64::from(sim.config().climate.days_per_year);
    let tpy = sim.config().turns_per_year();
    let snapshot_day = 318; // mid northern winter

    // Static fields.
    let water: Vec<i32> = sim
        .map()
        .tiles()
        .iter()
        .map(|t| match t.water {
            Water::Ocean => 0,
            Water::Lake => 2,
            _ => 1,
        })
        .collect();
    write_i32(out.join("water.bin"), &water);
    let weight: Vec<i32> = (0..n)
        .map(|i| i32::from(sim.climate().profile(i).oscillation_weight))
        .collect();
    write_i32(out.join("weight.bin"), &weight);
    let drift_weight: Vec<i32> = (0..n)
        .map(|i| i32::from(sim.climate().profile(i).drift_weight))
        .collect();
    write_i32(out.join("drift_weight.bin"), &drift_weight);
    let area: Vec<i64> = (0..n)
        .map(|i| sim.map().area_weight(sim.map().position_of(i as u32).y))
        .collect();
    fs::write(out.join("meta.txt"), format!("{w} {h} {days}\n")).unwrap();

    let mut daily = fs::File::create(out.join("daily.csv")).unwrap();
    writeln!(
        daily,
        "day,index_tenths,drift_tenths,global_mean_tenths,sea_ice_pct"
    )
    .unwrap();
    let mut annual = fs::File::create(out.join("annual.csv")).unwrap();
    writeln!(
        annual,
        "year,glacier_land_pct,permafrost_land_pct,fertility_land_mean,population"
    )
    .unwrap();

    let years = 50;
    let mut sums_first = vec![0i64; n];
    let mut sums_last = vec![0i64; n];
    // Share (percent) of the true area of tiles matching `of` that also match `part`.
    let share = |of: &dyn Fn(usize) -> bool, part: &dyn Fn(usize) -> bool| {
        let (mut a, mut b) = (0i64, 0i64);
        for (i, &tile_area) in area.iter().enumerate() {
            if of(i) {
                a += tile_area;
                if part(i) {
                    b += tile_area;
                }
            }
        }
        b as f64 * 100.0 / a.max(1) as f64
    };
    let land = |i: usize| water[i] == 1;

    for turn in 1..=(years * tpy + snapshot_day * 4 + 1) {
        if turn == snapshot_day * 4 + 1 || turn == years * tpy + snapshot_day * 4 + 1 {
            let label = if turn < tpy { "year1" } else { "year51" };
            let cities = sim.cities();
            render::png(
                sim.map(),
                sim.climate(),
                sim.biomes(),
                &cities,
                3,
                &out.join(format!("relief_{label}.png")),
            )
            .unwrap();
            let cover: Vec<i32> = (0..n)
                .map(|i| sim.climate().cryosphere().cover(sim.map(), i) as i32)
                .collect();
            write_i32(out.join(format!("cover_{label}.bin")), &cover);
        }
        if turn > years * tpy + snapshot_day * 4 + 1 {
            break;
        }
        sim.step();
        let year = (turn - 1) / tpy; // 0-based year this turn belongs to
        let temps = sim.climate().temperatures();
        if year == 0 {
            for (s, &t) in sums_first.iter_mut().zip(temps) {
                *s += i64::from(t);
            }
        } else if year == years - 1 {
            for (s, &t) in sums_last.iter_mut().zip(temps) {
                *s += i64::from(t);
            }
        }
        if turn % 4 == 2 {
            // Afternoon sample, once a day.
            let c = sim.climate();
            let sea_ice = share(&|i| water[i] == 0, &|i| {
                c.cryosphere().cover(sim.map(), i) == Cover::SeaIce
            });
            writeln!(
                daily,
                "{},{},{},{},{:.3}",
                turn / 4,
                c.oscillation().index_tenths(),
                c.drift().anomaly_tenths(),
                c.global_mean(),
                sea_ice
            )
            .unwrap();
        }
        if turn % tpy == 1 && turn > 1 || turn == 1 {
            let c = sim.climate();
            let glacier = share(&land, &|i| c.cryosphere().tile(i).glacier >= GLACIER);
            let permafrost = share(&land, &|i| c.cryosphere().tile(i).permafrost);
            let f = sim.fertility();
            let (mut fs_, mut a) = (0i64, 0i64);
            for i in (0..n).filter(|&i| land(i)) {
                fs_ += i64::from(f.fertility(i)) * area[i];
                a += area[i];
            }
            let pop: u64 = sim.cities().iter().map(|c| c.population).sum();
            writeln!(
                annual,
                "{},{:.3},{:.3},{:.2},{}",
                year + 1,
                glacier,
                permafrost,
                fs_ as f64 / a as f64 / 10.0,
                pop
            )
            .unwrap();
        }
    }
    let mean = |s: &[i64]| {
        s.iter()
            .map(|&v| (v / tpy as i64) as i32)
            .collect::<Vec<_>>()
    };
    write_i32(out.join("annual_mean_year1.bin"), &mean(&sums_first));
    write_i32(out.join("annual_mean_year50.bin"), &mean(&sums_last));
    println!("done: {} days", years * days + snapshot_day);
}
