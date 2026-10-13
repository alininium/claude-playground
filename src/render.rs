//! Visual output: ASCII for the terminal, PNG for a proper look.
//! Rendering never feeds back into the simulation.

use std::io;
use std::path::Path;

use crate::CitySnapshot;
use crate::climate::Climate;
use crate::climate::cryosphere::Cover;
use crate::fertility::Fertility;
use crate::map::{Biome, Map, Water};

/// `~` water, `.` earth, `:` sand, `n` hills, `^` mountains, `=` river,
/// `#` glacier or ice-covered water, `*` snow-covered land,
/// `o`/`O`/`@` cities by size.
pub fn ascii(map: &Map, climate: &Climate, cities: &[CitySnapshot]) -> String {
    let w = map.width() as usize;
    let mut grid: Vec<char> = map
        .tiles()
        .iter()
        .enumerate()
        .map(|(i, t)| match climate.cryosphere().cover(map, i) {
            Cover::Glacier | Cover::SeaIce | Cover::LakeIce => '#',
            Cover::Snow => '*',
            _ => match (t.biome, t.water) {
                (Biome::Water, _) => '~',
                (_, Water::River) => '=',
                (Biome::Earth, _) => '.',
                (Biome::Sand, _) => ':',
                (Biome::Hills, _) => 'n',
                (Biome::Mountains, _) => '^',
            },
        })
        .collect();
    for c in cities {
        grid[map.index(c.position)] = city_glyph(c.population);
    }
    grid.chunks(w)
        .map(|row| row.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn city_glyph(population: u64) -> char {
    match population {
        0..100 => 'o',
        100..1000 => 'O',
        _ => '@',
    }
}

/// Writes a shaded relief map with the current ice and snow, `scale` pixels
/// per tile.
pub fn png(
    map: &Map,
    climate: &Climate,
    cities: &[CitySnapshot],
    scale: u32,
    path: &Path,
) -> io::Result<()> {
    let mut tile_rgb: Vec<[u8; 3]> = (0..map.tile_count() as usize)
        .map(|i| tile_color(map, climate, i))
        .collect();
    for c in cities {
        tile_rgb[map.index(c.position)] = [200, 30, 30];
    }
    write_png(map, &tile_rgb, scale, path)
}

/// Writes a temperature map, `scale` pixels per tile. Coastlines are drawn
/// dark so the continents stay recognizable.
pub fn temperature_png(map: &Map, climate: &Climate, scale: u32, path: &Path) -> io::Result<()> {
    let colors: Vec<[u8; 3]> = (0..map.tile_count() as usize)
        .map(|i| {
            let coast = map.tiles()[i].water != Water::Ocean
                && map
                    .neighbors(i)
                    .any(|nb| map.tiles()[nb].water == Water::Ocean);
            if coast {
                [30, 30, 30]
            } else {
                temperature_color(climate.temperature(i))
            }
        })
        .collect();
    write_png(map, &colors, scale, path)
}

/// Color ramp from -40 °C (purple) through 0 °C (white) to +45 °C (dark red).
fn temperature_color(tenths: i16) -> [u8; 3] {
    const STOPS: [(i32, [i32; 3]); 7] = [
        (-400, [90, 0, 130]),
        (-200, [40, 60, 200]),
        (-50, [140, 200, 245]),
        (0, [245, 245, 245]),
        (150, [130, 210, 90]),
        (300, [245, 170, 40]),
        (450, [150, 0, 0]),
    ];
    let t = i32::from(tenths).clamp(STOPS[0].0, STOPS[STOPS.len() - 1].0);
    for pair in STOPS.windows(2) {
        let ((t0, c0), (t1, c1)) = (pair[0], pair[1]);
        if t <= t1 {
            return [0, 1, 2].map(|k| (c0[k] + (c1[k] - c0[k]) * (t - t0) / (t1 - t0)) as u8);
        }
    }
    [150, 0, 0]
}

fn write_png(map: &Map, colors: &[[u8; 3]], scale: u32, path: &Path) -> io::Result<()> {
    let w = map.width();
    let scale = scale.max(1);
    let (pw, ph) = (w * scale, map.height() * scale);
    let mut pixels = Vec::with_capacity((pw * ph * 3) as usize);
    for py in 0..ph {
        for px in 0..pw {
            pixels.extend_from_slice(&colors[((py / scale) * w + px / scale) as usize]);
        }
    }
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(io::BufWriter::new(file), pw, ph);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(io::Error::other)?;
    writer.write_image_data(&pixels).map_err(io::Error::other)?;
    Ok(())
}

/// Writes a fertility map: barren land brown, fertile land deep green.
pub fn fertility_png(map: &Map, fertility: &Fertility, scale: u32, path: &Path) -> io::Result<()> {
    let colors: Vec<[u8; 3]> = (0..map.tile_count() as usize)
        .map(|i| {
            let tile = &map.tiles()[i];
            if matches!(tile.water, Water::Ocean | Water::Lake) {
                return [40, 60, 110];
            }
            let f = i32::from(fertility.fertility(i));
            let lerp = |a: i32, b: i32| (a + (b - a) * f / 1000) as u8;
            [lerp(170, 20), lerp(150, 120), lerp(120, 20)]
        })
        .collect();
    write_png(map, &colors, scale, path)
}

fn tile_color(map: &Map, climate: &Climate, i: usize) -> [u8; 3] {
    let tile = &map.tiles()[i];
    let alt = tile.altitude;
    let ice: Option<[i32; 3]> = match climate.cryosphere().cover(map, i) {
        Cover::Glacier => Some([235, 242, 250]),
        Cover::SeaIce => Some([215, 228, 240]),
        Cover::LakeIce | Cover::RiverIce => Some([200, 225, 245]),
        Cover::Snow => Some([245, 247, 250]),
        _ => None,
    };
    let base: [i32; 3] = ice.unwrap_or(match (tile.biome, tile.water) {
        (_, Water::Ocean) => {
            let d = (-alt).clamp(0, 5000);
            [70 - d * 50 / 5000, 130 - d * 90 / 5000, 200 - d * 90 / 5000]
        }
        (_, Water::Lake) => [80, 150, 210],
        (_, Water::River) => [60, 120, 210],
        (Biome::Earth, _) => [90 + alt.min(800) / 20, 150 - alt.min(800) / 20, 70],
        (Biome::Sand, _) => [222, 200, 140],
        (Biome::Hills, _) => [150, 140, 90],
        (Biome::Mountains, _) => {
            let t = (alt - 1500).clamp(0, 3000);
            [
                130 + t * 115 / 3000,
                120 + t * 125 / 3000,
                115 + t * 130 / 3000,
            ]
        }
        (Biome::Water, _) => [70, 130, 200],
    });
    // Hillshade: light from the north-west.
    let shade = if matches!(tile.water, Water::Ocean | Water::Lake) {
        0
    } else {
        let pos = map.position_of(i as u32);
        let at = |dx: i32, dy: i32| {
            let x = (pos.x as i32 + dx).clamp(0, map.width() as i32 - 1) as u32;
            let y = (pos.y as i32 + dy).clamp(0, map.height() as i32 - 1) as u32;
            map.tiles()[(y * map.width() + x) as usize].altitude.max(0)
        };
        ((at(1, 1) - at(-1, -1)) / 25).clamp(-45, 45)
    };
    base.map(|c| (c + shade).clamp(0, 255) as u8)
}
