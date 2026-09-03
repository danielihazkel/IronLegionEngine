//! Isometric camera with four snap rotations (T1-052, TDD §10.1,
//! REQ-RNDR-001, REQ-RNDR-005, ADR-015).
//!
//! World: x right, y forward, metres. Screen: pixels, y down. The view is
//! rotated clockwise by `rotation × 90°`, so world → screen applies
//! `R(−k·90°)`; a world facing `f` (eighths of a turn, counter-clockwise from
//! +x) therefore shows sheet row `(f − 2k) mod 8`.

use glam::Vec2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// World point at the screen centre.
    pub center: Vec2,
    /// Pixels per metre along the screen x axis.
    pub zoom: f32,
    /// Snap rotation, `0..=3` quarter turns clockwise.
    pub rotation: u8,
    /// Screen y pixels per world-forward pixel (the fixed pitch).
    pub pitch: f32,
    /// Screen y pixels per metre of elevation, relative to `zoom`.
    pub elevation: f32,
}

impl Camera {
    /// Strategic zoom: a 2 km field fits a 4k-wide screen.
    pub const MIN_ZOOM: f32 = 2.0;
    /// Tactical zoom: a soldier is about 60 px across.
    pub const MAX_ZOOM: f32 = 96.0;
    pub const DEFAULT_ZOOM: f32 = 12.0;

    pub fn new(center: Vec2) -> Self {
        Self {
            center,
            zoom: Self::DEFAULT_ZOOM,
            rotation: 0,
            pitch: 0.5,
            elevation: 0.8,
        }
    }

    /// Rotates a world-space offset into view space (`R(−k·90°)`).
    pub fn rotate_to_view(&self, d: Vec2) -> Vec2 {
        match self.rotation & 3 {
            0 => d,
            1 => Vec2::new(d.y, -d.x),
            2 => Vec2::new(-d.x, -d.y),
            _ => Vec2::new(-d.y, d.x),
        }
    }

    /// Inverse of [`rotate_to_view`](Self::rotate_to_view).
    pub fn rotate_to_world(&self, d: Vec2) -> Vec2 {
        match self.rotation & 3 {
            0 => d,
            1 => Vec2::new(-d.y, d.x),
            2 => Vec2::new(-d.x, -d.y),
            _ => Vec2::new(d.y, -d.x),
        }
    }

    /// Projects a world point at `height` metres onto a `screen`-sized
    /// viewport (pixels).
    pub fn world_to_screen(&self, p: Vec2, height: f32, screen: Vec2) -> Vec2 {
        let d = self.rotate_to_view(p - self.center);
        Vec2::new(
            screen.x * 0.5 + d.x * self.zoom,
            screen.y * 0.5 - d.y * self.zoom * self.pitch - height * self.zoom * self.elevation,
        )
    }

    /// Unprojects a screen pixel to the world plane at height zero.
    pub fn screen_to_world(&self, s: Vec2, screen: Vec2) -> Vec2 {
        let d = Vec2::new(
            (s.x - screen.x * 0.5) / self.zoom,
            (screen.y * 0.5 - s.y) / (self.zoom * self.pitch),
        );
        self.center + self.rotate_to_world(d)
    }

    /// Sheet row for a world facing under the current rotation.
    pub fn facing_index(&self, facing8: u8) -> u8 {
        (facing8 + 8 - 2 * (self.rotation & 3)) % 8
    }

    /// Pans so the world moves by `delta_px` on screen (drag semantics).
    pub fn pan_screen(&mut self, delta_px: Vec2) {
        let d = Vec2::new(
            -delta_px.x / self.zoom,
            delta_px.y / (self.zoom * self.pitch),
        );
        self.center += self.rotate_to_world(d);
    }

    /// Multiplies the zoom, keeping the world point under `anchor` fixed.
    pub fn zoom_at(&mut self, factor: f32, anchor: Vec2, screen: Vec2) {
        let before = self.screen_to_world(anchor, screen);
        self.zoom = (self.zoom * factor).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        let after = self.screen_to_world(anchor, screen);
        self.center += before - after;
    }

    /// Snaps the view by `steps` quarter turns (positive = clockwise).
    pub fn rotate(&mut self, steps: i8) {
        self.rotation = (i16::from(self.rotation) + i16::from(steps)).rem_euclid(4) as u8;
    }

    /// World-space axis-aligned bounds of the viewport at height zero, padded
    /// by `pad` metres, for culling.
    pub fn visible_bounds(&self, screen: Vec2, pad: f32) -> (Vec2, Vec2) {
        let corners = [
            Vec2::ZERO,
            Vec2::new(screen.x, 0.0),
            Vec2::new(0.0, screen.y),
            screen,
        ]
        .map(|c| self.screen_to_world(c, screen));
        let mut min = corners[0];
        let mut max = corners[0];
        for c in &corners[1..] {
            min = min.min(*c);
            max = max.max(*c);
        }
        (min - Vec2::splat(pad), max + Vec2::splat(pad))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Vec2 = Vec2::new(1280.0, 800.0);

    #[test]
    fn projection_round_trips_under_every_rotation() {
        for k in 0..4u8 {
            let mut cam = Camera::new(Vec2::new(300.0, 200.0));
            cam.rotation = k;
            cam.zoom = 7.5;
            for p in [
                Vec2::new(0.0, 0.0),
                Vec2::new(310.0, 205.0),
                Vec2::new(-40.0, 900.0),
            ] {
                let s = cam.world_to_screen(p, 0.0, SCREEN);
                let back = cam.screen_to_world(s, SCREEN);
                assert!((back - p).length() < 1e-3, "k={k} p={p} back={back}");
            }
        }
    }

    #[test]
    fn centre_maps_to_screen_centre_and_forward_is_up() {
        let cam = Camera::new(Vec2::new(10.0, 10.0));
        let c = cam.world_to_screen(cam.center, 0.0, SCREEN);
        assert_eq!(c, SCREEN * 0.5);
        let ahead = cam.world_to_screen(cam.center + Vec2::Y, 0.0, SCREEN);
        assert!(ahead.y < c.y, "world forward is screen up");
        assert!((c.y - ahead.y - cam.zoom * cam.pitch).abs() < 1e-4);
        let up = cam.world_to_screen(cam.center, 1.0, SCREEN);
        assert!(up.y < c.y, "elevation lifts the sprite");
    }

    #[test]
    fn facing_index_follows_the_tdd_formula() {
        let mut cam = Camera::new(Vec2::ZERO);
        for k in 0..4u8 {
            cam.rotation = k;
            for f in 0..8u8 {
                let expected = (i32::from(f) - 2 * i32::from(k)).rem_euclid(8) as u8;
                assert_eq!(cam.facing_index(f), expected, "k={k} f={f}");
            }
        }
        // A soldier facing +x (east) seen after one clockwise quarter turn
        // points up the screen: row 6 is screen-down... check row 2 = up.
        cam.rotation = 3;
        assert_eq!(cam.facing_index(0), 2);
    }

    #[test]
    fn zoom_at_keeps_the_anchor_fixed_and_clamps() {
        let mut cam = Camera::new(Vec2::new(50.0, 50.0));
        let anchor = Vec2::new(200.0, 150.0);
        let before = cam.screen_to_world(anchor, SCREEN);
        cam.zoom_at(1.5, anchor, SCREEN);
        let after = cam.screen_to_world(anchor, SCREEN);
        assert!((before - after).length() < 1e-3);
        cam.zoom_at(1e6, anchor, SCREEN);
        assert_eq!(cam.zoom, Camera::MAX_ZOOM);
        cam.zoom_at(0.0, anchor, SCREEN);
        assert_eq!(cam.zoom, Camera::MIN_ZOOM);
    }

    #[test]
    fn pan_and_rotate_wrap() {
        let mut cam = Camera::new(Vec2::ZERO);
        let p = Vec2::new(5.0, 5.0);
        let s0 = cam.world_to_screen(p, 0.0, SCREEN);
        cam.pan_screen(Vec2::new(30.0, -10.0));
        let s1 = cam.world_to_screen(p, 0.0, SCREEN);
        assert!((s1 - s0 - Vec2::new(30.0, -10.0)).length() < 1e-3);
        cam.rotate(-1);
        assert_eq!(cam.rotation, 3);
        cam.rotate(2);
        assert_eq!(cam.rotation, 1);
    }

    #[test]
    fn visible_bounds_contain_the_viewport() {
        let mut cam = Camera::new(Vec2::new(100.0, 100.0));
        cam.rotation = 1;
        let (min, max) = cam.visible_bounds(SCREEN, 0.0);
        for c in [Vec2::ZERO, SCREEN, Vec2::new(SCREEN.x, 0.0)] {
            let w = cam.screen_to_world(c, SCREEN);
            assert!(w.x >= min.x - 1e-3 && w.x <= max.x + 1e-3);
            assert!(w.y >= min.y - 1e-3 && w.y <= max.y + 1e-3);
        }
    }
}
