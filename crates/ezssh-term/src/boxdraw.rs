//! Built-in drawing for box-drawing (U+2500–U+257F), block elements
//! (U+2580–U+259F) and the four Powerline separators (U+E0B0–U+E0B3).
//!
//! Font glyphs for these rarely fill the cell exactly, which leaves gaps
//! between rows in tmux borders, htop meters and TUI frames. Alacritty,
//! kitty and WezTerm draw them as shapes; so do we.
//!
//! Straight lines and blocks are emitted as rectangles in whole device
//! pixels into an un-feathered [`Mesh`]: egui's anti-aliasing feathers
//! every filled rect, and overlapping half-transparent edges show up as
//! bright ticks along a line at fractional cell widths.

use egui::{Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke, pos2};

/// Arm weights for U+2500..=U+257F in (up, right, down, left) order:
/// 0 none, 1 light, 2 heavy, 3 double. Dashed lines are drawn solid.
/// Entries for the rounded corners and diagonals are handled separately.
#[rustfmt::skip]
const LINES: [[u8; 4]; 128] = [
    [0,1,0,1],[0,2,0,2],[1,0,1,0],[2,0,2,0],[0,1,0,1],[0,2,0,2],[1,0,1,0],[2,0,2,0], // 2500
    [0,1,0,1],[0,2,0,2],[1,0,1,0],[2,0,2,0],[0,1,1,0],[0,2,1,0],[0,1,2,0],[0,2,2,0], // 2508
    [0,0,1,1],[0,0,1,2],[0,0,2,1],[0,0,2,2],[1,1,0,0],[1,2,0,0],[2,1,0,0],[2,2,0,0], // 2510
    [1,0,0,1],[1,0,0,2],[2,0,0,1],[2,0,0,2],[1,1,1,0],[1,2,1,0],[2,1,1,0],[1,1,2,0], // 2518
    [2,1,2,0],[2,2,1,0],[1,2,2,0],[2,2,2,0],[1,0,1,1],[1,0,1,2],[2,0,1,1],[1,0,2,1], // 2520
    [2,0,2,1],[2,0,1,2],[1,0,2,2],[2,0,2,2],[0,1,1,1],[0,1,1,2],[0,2,1,1],[0,2,1,2], // 2528
    [0,1,2,1],[0,1,2,2],[0,2,2,1],[0,2,2,2],[1,1,0,1],[1,1,0,2],[1,2,0,1],[1,2,0,2], // 2530
    [2,1,0,1],[2,1,0,2],[2,2,0,1],[2,2,0,2],[1,1,1,1],[1,1,1,2],[1,2,1,1],[1,2,1,2], // 2538
    [2,1,1,1],[1,1,2,1],[2,1,2,1],[2,1,1,2],[2,2,1,1],[1,1,2,2],[1,2,2,1],[2,2,1,2], // 2540
    [1,2,2,2],[2,1,2,2],[2,2,2,1],[2,2,2,2],[0,1,0,1],[0,2,0,2],[1,0,1,0],[2,0,2,0], // 2548
    [0,3,0,3],[3,0,3,0],[0,3,1,0],[0,1,3,0],[0,3,3,0],[0,0,1,3],[0,0,3,1],[0,0,3,3], // 2550
    [1,3,0,0],[3,1,0,0],[3,3,0,0],[1,0,0,3],[3,0,0,1],[3,0,0,3],[1,3,1,0],[3,1,3,0], // 2558
    [3,3,3,0],[1,0,1,3],[3,0,3,1],[3,0,3,3],[0,3,1,3],[0,1,3,1],[0,3,3,3],[1,3,0,3], // 2560
    [3,1,0,1],[3,3,0,3],[1,3,1,3],[3,1,3,1],[3,3,3,3],[0,0,0,0],[0,0,0,0],[0,0,0,0], // 2568 (256D.. rounded)
    [0,0,0,0],[0,0,0,0],[0,0,0,0],[0,0,0,0],[0,0,0,1],[1,0,0,0],[0,1,0,0],[0,0,1,0], // 2570
    [0,0,0,2],[2,0,0,0],[0,2,0,0],[0,0,2,0],[0,2,0,1],[1,0,2,0],[0,1,0,2],[2,0,1,0], // 2578
];

/// True if [`draw`] handles `c`.
pub fn handles(c: char) -> bool {
    matches!(c, '\u{2500}'..='\u{259F}' | '\u{E0B0}'..='\u{E0B3}')
}

/// A cell in whole device pixels.
#[derive(Clone, Copy, Debug)]
struct PxCell {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    ppp: f32,
    /// Light stroke width in device pixels, the same for every cell of a
    /// frame (see [`stroke_px`]).
    light: i32,
}

/// Light stroke width in device pixels for a cell `cell_w` points wide:
/// about 1/8 of the cell, at least one pixel. Computed once per frame and
/// passed to [`draw`]: deriving it per cell lets rounding noise in each
/// cell's width (at fractional scales the nominal width can sit exactly on
/// .5 px) give neighbouring cells different stroke widths.
pub fn stroke_px(cell_w: f32, pixels_per_point: f32) -> i32 {
    // The small bias makes a nominal .5 tie round up whatever the float noise.
    ((cell_w * pixels_per_point / 8.0) + 1e-3).round().max(1.0) as i32
}

impl PxCell {
    fn new(cell: Rect, ppp: f32, light: i32) -> Self {
        let p = |v: f32| (v * ppp).round() as i32;
        PxCell { x0: p(cell.min.x), y0: p(cell.min.y), x1: p(cell.max.x), y1: p(cell.max.y), ppp, light: light.max(1) }
    }

    fn rect(&self, x0: i32, y0: i32, x1: i32, y1: i32) -> Rect {
        let f = |v: i32| v as f32 / self.ppp;
        Rect::from_min_max(pos2(f(x0.min(x1)), f(y0.min(y1))), pos2(f(x0.max(x1)), f(y0.max(y1))))
    }

    fn as_rect(&self) -> Rect {
        self.rect(self.x0, self.y0, self.x1, self.y1)
    }

    /// Light stroke ≈ 1/8 of the cell width, at least one device pixel.
    fn light(&self) -> i32 {
        self.light
    }
}

/// Draw `c` into `cell` (the full cell rect; two cells wide for wide
/// characters) with light strokes `light_px` device pixels wide (from
/// [`stroke_px`]). Rectangles go into `mesh`, curves and diagonals to
/// `painter`. Returns false if `c` is not one of ours.
pub fn draw(
    mesh: &mut Mesh,
    painter: &Painter,
    cell: Rect,
    c: char,
    color: Color32,
    pixels_per_point: f32,
    light_px: i32,
) -> bool {
    let px = PxCell::new(cell, pixels_per_point, light_px);
    let cp = c as u32;
    match cp {
        0x2500..=0x257F => {
            match cp {
                0x256D..=0x2570 => rounded(painter, px, cp, color),
                0x2571..=0x2573 => diagonal(painter, px, cp, color),
                _ => lines(mesh, px, LINES[(cp - 0x2500) as usize], color),
            }
            true
        }
        0x2580..=0x259F => {
            blocks(mesh, px, cp, color);
            true
        }
        0xE0B0..=0xE0B3 => {
            powerline(painter, px, cp, color);
            true
        }
        _ => false,
    }
}

fn lines(mesh: &mut Mesh, c: PxCell, arms: [u8; 4], color: Color32) {
    let light = c.light();
    let heavy = light * 2;
    let cx = (c.x0 + c.x1) / 2;
    let cy = (c.y0 + c.y1) / 2;
    // Offset of each stroke of a double line from the centre.
    let d = light.max(2);
    let width = |w: u8| if w == 2 { heavy } else { light };
    let [up, right, down, left] = arms;
    let any_double_v = up == 3 || down == 3;
    let any_double_h = left == 3 || right == 3;
    let mut fill = |x0: i32, y0: i32, x1: i32, y1: i32| mesh.add_colored_rect(c.rect(x0, y0, x1, y1), color);
    // A stroke of thickness `t` centred on pixel boundary `at`.
    let span = |at: i32, t: i32| (at - t / 2, at - t / 2 + t);

    // Horizontal arms.
    for (w, dir) in [(left, -1), (right, 1)] {
        if w == 0 {
            continue;
        }
        let edge = if dir < 0 { c.x0 } else { c.x1 };
        if w == 3 {
            // Two strokes at cy ± d. Each starts past the centre when the
            // perpendicular arm on its side is double (keeps the gap open),
            // at the single line when that arm is single, else overshoots
            // to close the outer corner.
            for (side_arm, y) in [(up, cy - d), (down, cy + d)] {
                let start = match side_arm {
                    3 => cx + dir * d,
                    0 => cx - dir * d,
                    _ => cx - dir * (light / 2),
                };
                let (y0, y1) = span(y, light);
                fill(edge, y0, start, y1);
            }
        } else {
            let (y0, y1) = span(cy, width(w));
            let ext = if any_double_v { d + light / 2 + 1 } else { (width(up.max(down)) + 1) / 2 };
            fill(edge, y0, cx - dir * ext, y1);
        }
    }
    // Vertical arms.
    for (w, dir) in [(up, -1), (down, 1)] {
        if w == 0 {
            continue;
        }
        let edge = if dir < 0 { c.y0 } else { c.y1 };
        if w == 3 {
            for (side_arm, x) in [(left, cx - d), (right, cx + d)] {
                let start = match side_arm {
                    3 => cy + dir * d,
                    0 => cy - dir * d,
                    _ => cy - dir * (light / 2),
                };
                let (x0, x1) = span(x, light);
                fill(x0, edge, x1, start);
            }
        } else {
            let (x0, x1) = span(cx, width(w));
            let ext = if any_double_h { d + light / 2 + 1 } else { (width(left.max(right)) + 1) / 2 };
            fill(x0, edge, x1, cy - dir * ext);
        }
    }
}

fn rounded(painter: &Painter, c: PxCell, cp: u32, color: Color32) {
    let r = c.as_rect();
    let light = c.light() as f32 / c.ppp;
    // Stroke centre: the same pixel span the straight lines use.
    let cx = ((c.x0 + c.x1) / 2 - c.light() / 2) as f32 / c.ppp + light / 2.0;
    let cy = ((c.y0 + c.y1) / 2 - c.light() / 2) as f32 / c.ppp + light / 2.0;
    let radius = (r.width().min(r.height()) / 2.0).max(light);
    // (horizontal direction, vertical direction) of the two arms.
    let (hx, vy) = match cp {
        0x256D => (1.0, 1.0),   // ╭ right + down
        0x256E => (-1.0, 1.0),  // ╮ left + down
        0x256F => (-1.0, -1.0), // ╯ left + up
        _ => (1.0, -1.0),       // ╰ right + up
    };
    let h_edge = if hx > 0.0 { r.max.x } else { r.min.x };
    let v_edge = if vy > 0.0 { r.max.y } else { r.min.y };
    let arc_center = pos2(cx + hx * radius, cy + vy * radius);
    let mut pts: Vec<Pos2> = vec![pos2(cx, v_edge)];
    const STEPS: usize = 10;
    for i in 0..=STEPS {
        // From the vertical arm round to the horizontal arm.
        let t = i as f32 / STEPS as f32 * std::f32::consts::FRAC_PI_2;
        pts.push(pos2(arc_center.x - hx * radius * t.cos(), arc_center.y - vy * radius * t.sin()));
    }
    pts.push(pos2(h_edge, cy));
    painter.add(Shape::line(pts, Stroke::new(light, color)));
}

fn diagonal(painter: &Painter, c: PxCell, cp: u32, color: Color32) {
    let r = c.as_rect();
    let stroke = Stroke::new(c.light() as f32 / c.ppp, color);
    if cp == 0x2571 || cp == 0x2573 {
        painter.line_segment([r.left_bottom(), r.right_top()], stroke);
    }
    if cp == 0x2572 || cp == 0x2573 {
        painter.line_segment([r.left_top(), r.right_bottom()], stroke);
    }
}

fn blocks(mesh: &mut Mesh, c: PxCell, cp: u32, color: Color32) {
    let (w, h) = ((c.x1 - c.x0) as f32, (c.y1 - c.y0) as f32);
    let rect = |fx0: f32, fy0: f32, fx1: f32, fy1: f32| {
        c.rect(
            c.x0 + (w * fx0).round() as i32,
            c.y0 + (h * fy0).round() as i32,
            c.x0 + (w * fx1).round() as i32,
            c.y0 + (h * fy1).round() as i32,
        )
    };
    let mut fill = |r: Rect, col: Color32| mesh.add_colored_rect(r, col);
    match cp {
        0x2580 => fill(rect(0.0, 0.0, 1.0, 0.5), color),
        0x2581..=0x2588 => {
            let eighths = (cp - 0x2580) as f32;
            fill(rect(0.0, 1.0 - eighths / 8.0, 1.0, 1.0), color);
        }
        0x2589..=0x258F => {
            let eighths = (0x2590 - cp) as f32;
            fill(rect(0.0, 0.0, eighths / 8.0, 1.0), color);
        }
        0x2590 => fill(rect(0.5, 0.0, 1.0, 1.0), color),
        0x2591..=0x2593 => {
            let alpha = (cp - 0x2590) as f32 * 0.25;
            fill(rect(0.0, 0.0, 1.0, 1.0), color.gamma_multiply(alpha));
        }
        0x2594 => fill(rect(0.0, 0.0, 1.0, 0.125), color),
        0x2595 => fill(rect(0.875, 0.0, 1.0, 1.0), color),
        _ => {
            // Quadrants: bits UL, UR, LL, LR.
            let q: u8 = match cp {
                0x2596 => 0b0010,
                0x2597 => 0b0001,
                0x2598 => 0b1000,
                0x2599 => 0b1011,
                0x259A => 0b1001,
                0x259B => 0b1110,
                0x259C => 0b1101,
                0x259D => 0b0100,
                0x259E => 0b0110,
                _ => 0b0111, // 259F
            };
            for (bit, r) in [
                (0b1000, rect(0.0, 0.0, 0.5, 0.5)),
                (0b0100, rect(0.5, 0.0, 1.0, 0.5)),
                (0b0010, rect(0.0, 0.5, 0.5, 1.0)),
                (0b0001, rect(0.5, 0.5, 1.0, 1.0)),
            ] {
                if q & bit != 0 {
                    fill(r, color);
                }
            }
        }
    }
}

fn powerline(painter: &Painter, c: PxCell, cp: u32, color: Color32) {
    let r = c.as_rect();
    let stroke = Stroke::new(c.light() as f32 / c.ppp, color);
    let (l, rt, t, b) = (r.min.x, r.max.x, r.min.y, r.max.y);
    let mid = r.center().y;
    match cp {
        0xE0B0 => painter.add(Shape::convex_polygon(vec![pos2(l, t), pos2(rt, mid), pos2(l, b)], color, Stroke::NONE)),
        0xE0B2 => painter.add(Shape::convex_polygon(vec![pos2(rt, t), pos2(rt, b), pos2(l, mid)], color, Stroke::NONE)),
        0xE0B1 => painter.add(Shape::line(vec![pos2(l, t), pos2(rt, mid), pos2(l, b)], stroke)),
        _ => painter.add(Shape::line(vec![pos2(rt, t), pos2(l, mid), pos2(rt, b)], stroke)),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pixel coverage of the mesh for one character in a cell at the origin.
    fn coverage(c: char, w: f32, h: f32, ppp: f32) -> Vec<Vec<bool>> {
        let ctx = egui::Context::default();
        let painter = Painter::new(ctx, egui::LayerId::background(), Rect::EVERYTHING);
        let mut mesh = Mesh::default();
        assert!(draw(&mut mesh, &painter, Rect::from_min_size(Pos2::ZERO, egui::vec2(w, h)), c, Color32::WHITE, ppp, stroke_px(w, ppp)));
        let (pw, ph) = ((w * ppp).round() as usize, (h * ppp).round() as usize);
        let mut grid = vec![vec![false; pw]; ph];
        for quad in mesh.vertices.chunks(4) {
            let xs = quad.iter().map(|v| v.pos.x * ppp);
            let ys = quad.iter().map(|v| v.pos.y * ppp);
            let (x0, x1) = (xs.clone().fold(f32::MAX, f32::min), xs.fold(f32::MIN, f32::max));
            let (y0, y1) = (ys.clone().fold(f32::MAX, f32::min), ys.fold(f32::MIN, f32::max));
            assert_eq!(x0, x0.round(), "rect edge off the pixel grid");
            assert_eq!(y0, y0.round(), "rect edge off the pixel grid");
            for row in grid.iter_mut().take(y1 as usize).skip(y0 as usize) {
                for px in row.iter_mut().take(x1 as usize).skip(x0 as usize) {
                    *px = true;
                }
            }
        }
        grid
    }

    #[test]
    fn horizontal_line_spans_the_whole_cell_on_the_pixel_grid() {
        // A fractional cell width at a fractional scale, like 12 pt at 1.5×.
        let g = coverage('─', 7.2, 16.0, 1.5);
        let lit_rows: Vec<usize> = (0..g.len()).filter(|&y| g[y].iter().any(|&p| p)).collect();
        assert_eq!(lit_rows.len(), 1, "one device-pixel row for a light line at this size");
        assert!(g[lit_rows[0]].iter().all(|&p| p), "line has a gap");
    }

    #[test]
    fn stroke_px_is_stable_at_the_half_pixel_tie() {
        // 7.2 pt cells at 5/3 scale are 12 px wide: 1.5 px nominal stroke,
        // and float noise in 7.2 * 1.6667 lands on either side of 1.5.
        let ppp = 5.0 / 3.0;
        let a = stroke_px(7.2, ppp);
        for col in 0..200 {
            let x0 = 3.3 + col as f32 * 7.2;
            let w = (x0 + 7.2) - x0;
            assert_eq!(stroke_px(w, ppp), a, "column {col}");
        }
    }

    #[test]
    fn stroke_width_is_the_same_in_every_cell() {
        // 11.55 px cells at an offset where rounding makes some 11 and some 12 px wide.
        let ctx = egui::Context::default();
        let painter = Painter::new(ctx, egui::LayerId::background(), Rect::EVERYTHING);
        let mut heights = std::collections::BTreeSet::new();
        for col in 0..12 {
            let mut mesh = Mesh::default();
            let x = 3.3 + col as f32 * 11.55;
            draw(&mut mesh, &painter, Rect::from_min_size(egui::pos2(x, 0.0), egui::vec2(11.55, 20.0)), '─', Color32::WHITE, 1.0, stroke_px(11.55, 1.0));
            let ys: Vec<f32> = mesh.vertices.iter().map(|v| v.pos.y).collect();
            let h = ys.iter().cloned().fold(f32::MIN, f32::max) - ys.iter().cloned().fold(f32::MAX, f32::min);
            heights.insert(h as i32);
        }
        assert_eq!(heights.len(), 1, "stroke heights differ between cells: {heights:?}");
    }

    #[test]
    fn cross_joins_without_holes() {
        let g = coverage('┼', 9.0, 18.0, 1.0);
        let cy = g.iter().position(|r| r.iter().all(|&p| p)).expect("full row");
        let cx = (0..g[0].len()).find(|&x| g.iter().all(|r| r[x])).expect("full column");
        assert!(g[cy][cx]);
    }

    #[test]
    fn full_block_fills_cell() {
        let g = coverage('█', 7.2, 16.0, 1.5);
        assert!(g.iter().all(|r| r.iter().all(|&p| p)));
    }
}
