//! The window's icon, drawn rather than shipped.
//!
//! Upstream embeds `.ico` resources through the app packaging; this port has no
//! resource pipeline, and a binary asset in the tree would be the only one
//! nobody can read. The icon is therefore computed: a dark rounded square with a
//! prompt chevron and an underscore, at the 64×64 that a window manager wants
//! (ADR 038).

use eframe::egui::IconData;

/// The size of the icon, in pixels.
pub const SIZE: usize = 64;

/// The icon's background.
const BACKGROUND: [u8; 3] = [0x0C, 0x0C, 0x0C];
/// The prompt's colour.
const FOREGROUND: [u8; 3] = [0x3B, 0x8E, 0xEA];

/// The window icon this build draws.
pub fn data() -> IconData {
    let mut rgba = vec![0u8; SIZE * SIZE * 4];
    let radius = 12.0;
    let stroke = 3.5;

    for y in 0..SIZE {
        for x in 0..SIZE {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let alpha = if inside_rounded_square(fx, fy, radius) { 255 } else { 0 };

            // A `>` and an underscore: the shape every terminal's title bar uses.
            let chevron = distance_to_segment(fx, fy, 18.0, 18.0, 34.0, 32.0)
                .min(distance_to_segment(fx, fy, 18.0, 46.0, 34.0, 32.0));
            let underscore = if (40.0..52.0).contains(&fx) {
                (43.0 - fy).abs()
            } else {
                f32::MAX
            };
            let ink = chevron.min(underscore) <= stroke;

            let colour = if ink { FOREGROUND } else { BACKGROUND };
            let offset = (y * SIZE + x) * 4;
            rgba[offset] = colour[0];
            rgba[offset + 1] = colour[1];
            rgba[offset + 2] = colour[2];
            rgba[offset + 3] = alpha;
        }
    }

    IconData {
        rgba,
        width: SIZE as u32,
        height: SIZE as u32,
    }
}

/// Whether a pixel is inside a square with rounded corners.
fn inside_rounded_square(x: f32, y: f32, radius: f32) -> bool {
    let max = SIZE as f32;
    if !(0.0..max).contains(&x) || !(0.0..max).contains(&y) {
        return false;
    }

    // The corner circle's centre, clamped so the straight edges are unaffected.
    let cx = x.clamp(radius, max - radius);
    let cy = y.clamp(radius, max - radius);
    let (dx, dy) = (x - cx, y - cy);

    dx * dx + dy * dy <= radius * radius
}

/// The distance from a point to a line segment.
fn distance_to_segment(px: f32, py: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    let (vx, vy) = (x2 - x1, y2 - y1);
    let (wx, wy) = (px - x1, py - y1);
    let length = vx * vx + vy * vy;
    let t = if length == 0.0 {
        0.0
    } else {
        ((wx * vx + wy * vy) / length).clamp(0.0, 1.0)
    };
    let (dx, dy) = (wx - t * vx, wy - t * vy);
    (dx * dx + dy * dy).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(icon: &IconData, x: usize, y: usize) -> [u8; 4] {
        let offset = (y * SIZE + x) * 4;
        [
            icon.rgba[offset],
            icon.rgba[offset + 1],
            icon.rgba[offset + 2],
            icon.rgba[offset + 3],
        ]
    }

    #[test]
    fn the_icon_is_a_square_of_rgba_pixels() {
        let icon = data();
        assert_eq!(icon.width, SIZE as u32);
        assert_eq!(icon.height, SIZE as u32);
        assert_eq!(icon.rgba.len(), SIZE * SIZE * 4);
    }

    #[test]
    fn the_corners_are_rounded_and_the_middle_is_painted() {
        let icon = data();

        assert_eq!(pixel(&icon, 0, 0)[3], 0, "a corner is transparent");
        assert_eq!(pixel(&icon, SIZE - 1, 0)[3], 0, "a corner is transparent");
        assert_eq!(pixel(&icon, SIZE / 2, SIZE / 2)[3], 255, "the middle is opaque");
    }

    #[test]
    fn the_prompt_is_drawn_in_the_foreground_colour() {
        let icon = data();
        let mut ink = 0;
        for y in 0..SIZE {
            for x in 0..SIZE {
                let [r, g, b, a] = pixel(&icon, x, y);
                if a == 255 && [r, g, b] == FOREGROUND {
                    ink += 1;
                }
            }
        }
        assert!(ink > 80, "the chevron and the underscore cover {ink} pixels");
    }

    #[test]
    fn the_background_fills_the_square() {
        let icon = data();
        let [r, g, b, a] = pixel(&icon, 4, SIZE / 2);
        assert_eq!(a, 255);
        assert_eq!([r, g, b], BACKGROUND);
    }
}
