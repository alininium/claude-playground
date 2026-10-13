//! Visual output: ASCII for the terminal, PNG for a proper look.
//! Rendering never feeds back into the simulation.

use std::io;
use std::path::Path;

use crate::CitySnapshot;
use crate::map::{Biome, Map, Water};

/// `~` water, `.` earth, `:` sand, `n` hills, `^` mountains, `=` river,
/// `o`/`O`/`@` cities by size.
pub fn ascii(map: &Map, cities: &[CitySnapshot]) -> String {
    let w = map.width() as usize;
    let mut grid: Vec<char> = map
        .tiles()
        .iter()
        .map(|t| match (t.biome, t.water) {
            (Biome::Water, _) => '~',
            (_, Water::River) => '=',
            (Biome::Earth, _) => '.',
            (Biome::Sand, _) => ':',
            (Biome::Hills, _) => 'n',
            (Biome::Mountains, _) => '^',
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

/// Writes a shaded relief map, `scale` pixels per tile.
pub fn png(map: &Map, cities: &[CitySnapshot], scale: u32, path: &Path) -> io::Result<()> {
    let (w, h) = (map.width(), map.height());
    let scale = scale.max(1);
    let (pw, ph) = (w * scale, h * scale);
    let mut tile_rgb = vec![[0u8; 3]; (w * h) as usize];
    for (i, rgb) in tile_rgb.iter_mut().enumerate() {
        *rgb = tile_color(map, i);
    }
    for c in cities {
        tile_rgb[map.index(c.position)] = [200, 30, 30];
    }

    let mut pixels = Vec::with_capacity((pw * ph * 3) as usize);
    for py in 0..ph {
        for px in 0..pw {
            pixels.extend_from_slice(&tile_rgb[((py / scale) * w + px / scale) as usize]);
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

fn tile_color(map: &Map, i: usize) -> [u8; 3] {
    let tile = &map.tiles()[i];
    let alt = tile.altitude;
    let base: [i32; 3] = match (tile.biome, tile.water) {
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
    };
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
