//! The minimap's fog texture (T2-090, plan decision 4, I6), headless: a
//! line-of-sight disc lights the pixels inside it and nothing outside,
//! open river cells are water, crossings keep their zone colour, the
//! world-to-pixel mapping puts world `y` up, and the widget draws.

use std::path::PathBuf;
use std::sync::Arc;

use glam::Vec2;
use il_core::Scalar;
use il_sim_battle::LoadedMap;
use il_ui::{MapScale, MiniBlock, Minimap, MinimapInput, render_fog};

fn map() -> (Arc<il_data::Registries>, Arc<LoadedMap>) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../game");
    let regs = Arc::new(il_data::load_roots(&[root]).unwrap_or_else(|e| panic!("{e}")));
    let h = regs
        .maps
        .lookup(&il_data::ContentId::new("rome:test_field").unwrap())
        .expect("test map");
    let def = regs.maps.get(h);
    let map = LoadedMap::from_def(def, regs.rules.movement.zone_cell).expect("map loads");
    (regs, Arc::new(map))
}

fn palettes(regs: &il_data::Registries, map: &LoadedMap) -> (Vec<[u8; 3]>, Vec<bool>) {
    (
        map.zone_handles
            .iter()
            .map(|h| regs.zones.get(*h).colour.0)
            .collect(),
        map.zone_handles
            .iter()
            .map(|h| regs.zones.get(*h).crossing)
            .collect(),
    )
}

fn brightness(c: egui::Color32) -> u32 {
    u32::from(c.r()) + u32::from(c.g()) + u32::from(c.b())
}

#[test]
fn scale_maps_world_y_up_and_fits_the_long_side() {
    let (_, map) = map();
    let s = MapScale::for_map(&map);
    assert_eq!(s.width.max(s.height), il_ui::minimap::TEXTURE_LONG_SIDE);
    // The map's south-west corner is the bottom-left pixel.
    let (x, y) = s.to_px(Vec2::ZERO);
    assert!(x.abs() < 1e-3 && (y - s.height as f32).abs() < 1e-3);
    // The north-east corner is the top-right pixel.
    let (x, y) = s.to_px(Vec2::new(
        map.width.to_f32_render(),
        map.height.to_f32_render(),
    ));
    assert!((x - s.width as f32).abs() < 1.0 && y.abs() < 1.0);
    // A pixel's centre maps back into itself.
    let c = s.px_centre(10, 20);
    let (px, py) = s.to_px(c);
    assert_eq!((px.floor() as usize, py.floor() as usize), (10, 20));
}

#[test]
fn a_disc_lights_its_pixels_and_fog_darkens_the_rest() {
    let (regs, map) = map();
    let (colours, crossing) = palettes(&regs, &map);
    let s = MapScale::for_map(&map);
    let centre = Vec2::new(200.0, 200.0);
    let dark = render_fog(&MinimapInput {
        map: &map,
        zone_colours: &colours,
        zone_crossing: &crossing,
        discs: &[],
        blocks: &[],
        viewport: [Vec2::ZERO; 4],
    });
    let lit = render_fog(&MinimapInput {
        map: &map,
        zone_colours: &colours,
        zone_crossing: &crossing,
        discs: &[(centre, 50.0)],
        blocks: &[],
        viewport: [Vec2::ZERO; 4],
    });
    let at = |img: &egui::ColorImage, p: Vec2| {
        let (x, y) = s.to_px(p);
        img.pixels[y as usize * s.width + x as usize]
    };
    assert!(brightness(at(&lit, centre)) > brightness(at(&dark, centre)));
    let inside = centre + Vec2::new(30.0, 0.0);
    let outside = centre + Vec2::new(70.0, 0.0);
    assert!(brightness(at(&lit, inside)) > brightness(at(&dark, inside)));
    assert_eq!(
        at(&lit, outside),
        at(&dark, outside),
        "beyond the disc stays fogged"
    );
    // Fog is a darkening, never a colour change.
    let l = at(&lit, centre);
    let d = at(&dark, centre);
    assert!(u32::from(d.r()) <= u32::from(l.r()) && u32::from(d.b()) <= u32::from(l.b()));
}

#[test]
fn open_river_cells_are_water_and_crossings_keep_their_zone() {
    let (regs, map) = map();
    let (colours, crossing) = palettes(&regs, &map);
    let s = MapScale::for_map(&map);
    let img = render_fog(&MinimapInput {
        map: &map,
        zone_colours: &colours,
        zone_crossing: &crossing,
        discs: &[(Vec2::new(400.0, 300.0), 2000.0)],
        blocks: &[],
        viewport: [Vec2::ZERO; 4],
    });
    let water = egui::Color32::from_rgb(
        il_ui::minimap::WATER[0],
        il_ui::minimap::WATER[1],
        il_ui::minimap::WATER[2],
    );
    let mut river = 0;
    let mut crossing_px = 0;
    for y in 0..s.height {
        for x in 0..s.width {
            let p = s.px_centre(x, y);
            let wp = il_core::V2::from_f32_data(p.x, p.y);
            if !map.river_at(wp) {
                continue;
            }
            let zone = usize::from(map.zone_index_at(wp));
            let px = img.pixels[y * s.width + x];
            if crossing[zone] {
                crossing_px += 1;
                assert_ne!(px, water, "a crossing is not water");
            } else {
                river += 1;
                assert_eq!(px, water, "an open river cell is water");
            }
        }
    }
    assert!(river > 0, "the test map has a river");
    assert!(crossing_px > 0, "the test map has a ford or bridge");
}

#[test]
fn the_widget_draws_headless() {
    let (regs, map) = map();
    let (colours, crossing) = palettes(&regs, &map);
    let blocks = [
        MiniBlock {
            pos: Vec2::new(100.0, 100.0),
            tint: [200, 50, 50, 255],
            ghost: false,
            selected: true,
        },
        MiniBlock {
            pos: Vec2::new(500.0, 400.0),
            tint: [50, 50, 200, 255],
            ghost: true,
            selected: false,
        },
    ];
    let input = MinimapInput {
        map: &map,
        zone_colours: &colours,
        zone_crossing: &crossing,
        discs: &[(Vec2::new(100.0, 100.0), 80.0)],
        blocks: &blocks,
        viewport: [
            Vec2::new(50.0, 50.0),
            Vec2::new(250.0, 50.0),
            Vec2::new(250.0, 200.0),
            Vec2::new(50.0, 200.0),
        ],
    };
    let ctx = egui::Context::default();
    let mut minimap = Minimap::new();
    for _ in 0..3 {
        ctx.begin_pass(egui::RawInput::default());
        let action = minimap.show(&ctx, &input, &regs.locale);
        assert!(action.is_none(), "no pointer, no click");
        let mut out = ctx.end_pass();
        assert!(!out.shapes.is_empty());
        // The first pass uploads the minimap texture; nothing applies it here.
        assert!(!out.textures_delta.set.is_empty() || out.textures_delta.is_empty());
        out.textures_delta.clear();
    }
}
