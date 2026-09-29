//! Box-drawing, block and Powerline characters drawn as geometry instead of
//! font glyphs. Fonts draw them shorter than the cell (and Consolas has no
//! quadrant blocks at all), which leaves gaps between rows and columns;
//! filling the cell edge to edge joins neighbours the way Hyper does.
use gpui::{Bounds, Hsla, PathBuilder, Pixels, Window, fill, point, px, size};

/// Arms of U+2500–U+257F as 0xURDL nibbles (up, right, down, left):
/// 1 light, 2 heavy, 3 double. Zero marks the dashes, arcs and diagonals.
#[rustfmt::skip]
const ARMS: [u16; 128] = [
    // U+2500
    0x0101, 0x0202, 0x1010, 0x2020, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0110, 0x0210, 0x0120, 0x0220,
    // U+2510
    0x0011, 0x0012, 0x0021, 0x0022, 0x1100, 0x1200, 0x2100, 0x2200, 0x1001, 0x1002, 0x2001, 0x2002, 0x1110, 0x1210, 0x2110, 0x1120,
    // U+2520
    0x2120, 0x2210, 0x1220, 0x2220, 0x1011, 0x1012, 0x2011, 0x1021, 0x2021, 0x2012, 0x1022, 0x2022, 0x0111, 0x0112, 0x0211, 0x0212,
    // U+2530
    0x0121, 0x0122, 0x0221, 0x0222, 0x1101, 0x1102, 0x1201, 0x1202, 0x2101, 0x2102, 0x2201, 0x2202, 0x1111, 0x1112, 0x1211, 0x1212,
    // U+2540
    0x2111, 0x1121, 0x2121, 0x2112, 0x2211, 0x1122, 0x1221, 0x2212, 0x1222, 0x2122, 0x2221, 0x2222, 0x0000, 0x0000, 0x0000, 0x0000,
    // U+2550
    0x0303, 0x3030, 0x0310, 0x0130, 0x0330, 0x0013, 0x0031, 0x0033, 0x1300, 0x3100, 0x3300, 0x1003, 0x3001, 0x3003, 0x1310, 0x3130,
    // U+2560
    0x3330, 0x1013, 0x3031, 0x3033, 0x0313, 0x0131, 0x0333, 0x1303, 0x3101, 0x3303, 0x1313, 0x3131, 0x3333, 0x0000, 0x0000, 0x0000,
    // U+2570
    0x0000, 0x0000, 0x0000, 0x0000, 0x0001, 0x1000, 0x0100, 0x0010, 0x0002, 0x2000, 0x0200, 0x0020, 0x0201, 0x1020, 0x0102, 0x2010,
];

/// Filled quadrants of U+2596–U+259F: 1 upper left, 2 upper right,
/// 4 lower left, 8 lower right.
const QUADRANTS: [u8; 10] = [4, 8, 1, 13, 9, 7, 11, 2, 6, 14];

/// Handle length of a cubic Bézier that follows a quarter circle.
const KAPPA: f32 = 0.5523;

/// A rectangle in device pixels, and how opaque it is.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    alpha: f32,
}

/// Whether `c` is drawn here rather than by the font.
pub fn covers(c: char) -> bool {
    matches!(c as u32, 0x2500..=0x259F | 0xE0B0..=0xE0B7)
}

/// `bounds` with its edges moved to whole device pixels, so cells that share
/// an edge in layout also share it on screen.
pub fn snap(bounds: Bounds<Pixels>, scale: f32) -> Bounds<Pixels> {
    let [x0, y0, x1, y1] = device(bounds, scale);
    logical(x0, y0, x1, y1, scale)
}

/// Draws `c` over the whole of `cell`. Returns false for characters the font
/// should draw instead.
pub fn paint(
    c: char,
    cell: Bounds<Pixels>,
    color: Hsla,
    font_size: f32,
    window: &mut Window,
) -> bool {
    if !covers(c) {
        return false;
    }
    let scale = window.scale_factor();
    let edges = device(cell, scale);
    let light = ((font_size * scale / 14.).round() as i32).max(1);
    if let Some(rects) = rects(c, edges, light) {
        for r in rects {
            if r.x1 > r.x0 && r.y1 > r.y0 {
                window.paint_quad(fill(
                    logical(r.x0, r.y0, r.x1, r.y1, scale),
                    color.opacity(r.alpha),
                ));
            }
        }
    } else if let Some(path) = path(c, edges, light, scale).and_then(|p| p.build().ok()) {
        window.paint_path(path, color);
    }
    true
}

fn device(bounds: Bounds<Pixels>, scale: f32) -> [i32; 4] {
    let edge = |v: Pixels| (f32::from(v) * scale).round() as i32;
    [
        edge(bounds.left()),
        edge(bounds.top()),
        edge(bounds.right()),
        edge(bounds.bottom()),
    ]
}

fn logical(x0: i32, y0: i32, x1: i32, y1: i32, scale: f32) -> Bounds<Pixels> {
    Bounds::new(
        point(px(x0 as f32 / scale), px(y0 as f32 / scale)),
        size(px((x1 - x0) as f32 / scale), px((y1 - y0) as f32 / scale)),
    )
}

/// The rectangles that make up `c`, or `None` when it is drawn as a path.
fn rects(c: char, cell: [i32; 4], light: i32) -> Option<Vec<Rect>> {
    let code = c as u32;
    let mut out = Vec::new();
    match code {
        0x2504..=0x250B => {
            let i = code - 0x2504;
            dashes(
                &mut out,
                cell,
                (i / 2) % 2 == 0,
                if i < 4 { 3 } else { 4 },
                light * (1 + (i % 2) as i32),
            );
        }
        0x254C..=0x254F => {
            let i = code - 0x254C;
            dashes(&mut out, cell, i < 2, 2, light * (1 + (i % 2) as i32));
        }
        0x2500..=0x257F => {
            let arms = ARMS[(code - 0x2500) as usize];
            if arms == 0 {
                return None;
            }
            lines(&mut out, cell, arms, light);
        }
        0x2580..=0x259F => blocks(&mut out, cell, code),
        _ => return None,
    }
    Some(out)
}

/// Straight lines from the middle of the cell to its edges. Arms meet so the
/// corners close: a light arm reaches across a crossing heavy line, and double
/// lines form inner and outer corners (╔) or stop at the nearer line (╟).
fn lines(out: &mut Vec<Rect>, [x0, y0, x1, y1]: [i32; 4], arms: u16, light: i32) {
    let [up, right, down, left] = [
        (arms >> 12) & 0xF,
        (arms >> 8) & 0xF,
        (arms >> 4) & 0xF,
        arms & 0xF,
    ];
    let (w, h) = (x1 - x0, y1 - y0);
    let single = |weight: u16| weight == 1 || weight == 2;
    let thick = |weight: u16| if weight == 2 { light * 2 } else { light };
    // Left or top edge of a line `t` thick through the middle of the cell.
    let vx = |t: i32| x0 + (w - t) / 2;
    let hy = |t: i32| y0 + (h - t) / 2;
    let (mx, my) = (x0 + w / 2, y0 + h / 2);
    // A double line is two light lines with a light-wide gap: a and b are the
    // left edges of the vertical pair, p and q the top edges of the horizontal.
    let a = x0 + (w - 3 * light) / 2;
    let b = a + 2 * light;
    let p = y0 + (h - 3 * light) / 2;
    let q = p + 2 * light;
    // The widest single line along each axis, which the other axis reaches across.
    let vertical = [up, down]
        .into_iter()
        .filter(|&w| single(w))
        .map(thick)
        .max();
    let horizontal = [left, right]
        .into_iter()
        .filter(|&w| single(w))
        .map(thick)
        .max();
    let double_v = up == 3 || down == 3;
    let double_h = left == 3 || right == 3;
    let mut rect = |x0, y0, x1, y1| {
        out.push(Rect {
            x0,
            y0,
            x1,
            y1,
            alpha: 1.,
        })
    };

    if single(right) {
        let t = thick(right);
        let start = if double_v {
            if left != 0 {
                mx
            } else if up != 0 && down != 0 {
                b
            } else {
                a
            }
        } else {
            vertical.map_or(mx, vx)
        };
        rect(start, hy(t), x1, hy(t) + t);
    } else if right == 3 {
        let (top, bottom) = if double_v {
            (if up == 3 { b } else { a }, if down == 3 { b } else { a })
        } else {
            let start = vertical.map_or(mx, vx);
            (start, start)
        };
        rect(top, p, x1, p + light);
        rect(bottom, q, x1, q + light);
    }

    if single(left) {
        let t = thick(left);
        let end = if double_v {
            if right != 0 {
                mx
            } else if up != 0 && down != 0 {
                a + light
            } else {
                b + light
            }
        } else {
            vertical.map_or(mx, |v| vx(v) + v)
        };
        rect(x0, hy(t), end, hy(t) + t);
    } else if left == 3 {
        let (top, bottom) = if double_v {
            (
                if up == 3 { a + light } else { b + light },
                if down == 3 { a + light } else { b + light },
            )
        } else {
            let end = vertical.map_or(mx, |v| vx(v) + v);
            (end, end)
        };
        rect(x0, p, top, p + light);
        rect(x0, q, bottom, q + light);
    }

    if single(up) {
        let t = thick(up);
        let end = if double_h {
            if down != 0 {
                my
            } else if left != 0 && right != 0 {
                p + light
            } else {
                q + light
            }
        } else {
            horizontal.map_or(my, |v| hy(v) + v)
        };
        rect(vx(t), y0, vx(t) + t, end);
    } else if up == 3 {
        let (first, second) = if double_h {
            (
                if left == 3 { p + light } else { q + light },
                if right == 3 { p + light } else { q + light },
            )
        } else {
            let end = horizontal.map_or(my, |v| hy(v) + v);
            (end, end)
        };
        rect(a, y0, a + light, first);
        rect(b, y0, b + light, second);
    }

    if single(down) {
        let t = thick(down);
        let start = if double_h {
            if up != 0 {
                my
            } else if left != 0 && right != 0 {
                q
            } else {
                p
            }
        } else {
            horizontal.map_or(my, hy)
        };
        rect(vx(t), start, vx(t) + t, y1);
    } else if down == 3 {
        let (first, second) = if double_h {
            (
                if left == 3 { q } else { p },
                if right == 3 { q } else { p },
            )
        } else {
            let start = horizontal.map_or(my, hy);
            (start, start)
        };
        rect(a, first, a + light, y1);
        rect(b, second, b + light, y1);
    }
}

/// `count` dashes along the middle of the cell, `t` thick.
fn dashes(out: &mut Vec<Rect>, [x0, y0, x1, y1]: [i32; 4], horizontal: bool, count: i32, t: i32) {
    let (from, to) = if horizontal { (x0, x1) } else { (y0, y1) };
    let across = if horizontal {
        y0 + (y1 - y0 - t) / 2
    } else {
        x0 + (x1 - x0 - t) / 2
    };
    for i in 0..count {
        let start = from + (to - from) * i / count;
        let end = from + (to - from) * (i + 1) / count;
        let gap = (((end - start) * 2 + 2) / 5).max(1);
        let (s, e) = (start + gap / 2, end - (gap - gap / 2));
        out.push(if horizontal {
            Rect {
                x0: s,
                y0: across,
                x1: e,
                y1: across + t,
                alpha: 1.,
            }
        } else {
            Rect {
                x0: across,
                y0: s,
                x1: across + t,
                y1: e,
                alpha: 1.,
            }
        });
    }
}

/// U+2580–U+259F: eighths, halves, shades and quadrants.
fn blocks(out: &mut Vec<Rect>, [x0, y0, x1, y1]: [i32; 4], code: u32) {
    let (w, h) = (x1 - x0, y1 - y0);
    // n eighths across or down from the left or top edge, never empty.
    let x = |n: i32| x0 + (w * n / 8).max(1);
    let y = |n: i32| y0 + (h * n / 8).max(1);
    let mut rect = |x0, y0, x1, y1, alpha| {
        out.push(Rect {
            x0,
            y0,
            x1,
            y1,
            alpha,
        })
    };
    match code {
        0x2580 => rect(x0, y0, x1, y(4), 1.),
        0x2581..=0x2587 => rect(x0, y(8 - (code - 0x2580) as i32), x1, y1, 1.),
        0x2588 => rect(x0, y0, x1, y1, 1.),
        0x2589..=0x258F => rect(x0, y0, x((0x2590 - code) as i32), y1, 1.),
        0x2590 => rect(x(4), y0, x1, y1, 1.),
        0x2591..=0x2593 => rect(x0, y0, x1, y1, (code - 0x2590) as f32 * 0.25),
        0x2594 => rect(x0, y0, x1, y(1), 1.),
        0x2595 => rect(x(7), y0, x1, y1, 1.),
        _ => {
            let quadrants = QUADRANTS[(code - 0x2596) as usize];
            let (mx, my) = (x(4), y(4));
            for (bit, [a, b, c, d]) in [
                (1, [x0, y0, mx, my]),
                (2, [mx, y0, x1, my]),
                (4, [x0, my, mx, y1]),
                (8, [mx, my, x1, y1]),
            ] {
                if quadrants & bit != 0 {
                    rect(a, b, c, d, 1.);
                }
            }
        }
    }
}

/// Rounded corners, diagonals and Powerline separators.
fn path(c: char, [x0, y0, x1, y1]: [i32; 4], light: i32, scale: f32) -> Option<PathBuilder> {
    let pt = |x: f32, y: f32| point(px(x / scale), px(y / scale));
    let (l, t, r, b) = (x0 as f32, y0 as f32, x1 as f32, y1 as f32);
    let line = light as f32;
    let stroke = || PathBuilder::stroke(px(line / scale));
    // The centre of a light line, where the straight arms of ─ and │ run.
    let cx = (x0 + (x1 - x0 - light) / 2) as f32 + line / 2.;
    let cy = (y0 + (y1 - y0 - light) / 2) as f32 + line / 2.;
    let ym = (t + b) / 2.;
    let code = c as u32;
    let mut path = match code {
        0x256D..=0x2570 => {
            let (dx, dy) = match code {
                0x256D => (1., 1.),
                0x256E => (-1., 1.),
                0x256F => (-1., -1.),
                _ => (1., -1.),
            };
            let ex = if dx > 0. { r } else { l };
            let ey = if dy > 0. { b } else { t };
            let radius = (ex - cx).abs().min((ey - cy).abs());
            let mut path = stroke();
            path.move_to(pt(cx, ey));
            path.line_to(pt(cx, cy + dy * radius));
            path.cubic_bezier_to(
                pt(cx + dx * radius, cy),
                pt(cx, cy + dy * radius * (1. - KAPPA)),
                pt(cx + dx * radius * (1. - KAPPA), cy),
            );
            path.line_to(pt(ex, cy));
            path
        }
        0x2571..=0x2573 => {
            let mut path = stroke();
            if code != 0x2572 {
                path.move_to(pt(r, t));
                path.line_to(pt(l, b));
            }
            if code != 0x2571 {
                path.move_to(pt(l, t));
                path.line_to(pt(r, b));
            }
            path
        }
        0xE0B0 | 0xE0B2 => {
            let (base, tip) = if code == 0xE0B0 { (l, r) } else { (r, l) };
            let mut path = PathBuilder::fill();
            path.add_polygon(&[pt(base, t), pt(tip, ym), pt(base, b)], true);
            path
        }
        0xE0B1 | 0xE0B3 => {
            // Inset by half a line so the stroke stays inside the cell.
            let (base, tip) = if code == 0xE0B1 {
                (l, r - line / 2.)
            } else {
                (r, l + line / 2.)
            };
            let mut path = stroke();
            path.add_polygon(&[pt(base, t), pt(tip, ym), pt(base, b)], false);
            path
        }
        0xE0B4..=0xE0B7 => {
            let filled = code % 2 == 0;
            let inset = if filled { 0. } else { line / 2. };
            let (base, dir) = if code <= 0xE0B5 { (l, 1.) } else { (r, -1.) };
            let (rx, ry) = (r - l - inset, (b - t) / 2.);
            let mut path = if filled {
                PathBuilder::fill()
            } else {
                stroke()
            };
            path.move_to(pt(base, t));
            path.cubic_bezier_to(
                pt(base + dir * rx, ym),
                pt(base + dir * rx * KAPPA, t),
                pt(base + dir * rx, ym - ry * KAPPA),
            );
            path.cubic_bezier_to(
                pt(base, b),
                pt(base + dir * rx, ym + ry * KAPPA),
                pt(base + dir * rx * KAPPA, b),
            );
            path
        }
        _ => return None,
    };
    if matches!(code, 0xE0B4 | 0xE0B6) {
        path.close();
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CELL: [i32; 4] = [0, 0, 8, 18];

    fn cover(c: char) -> Vec<Vec<bool>> {
        let mut grid = vec![vec![false; 8]; 18];
        for r in rects(c, CELL, 1).unwrap() {
            for row in grid.iter_mut().take(r.y1 as usize).skip(r.y0 as usize) {
                for cell in row.iter_mut().take(r.x1 as usize).skip(r.x0 as usize) {
                    *cell = true;
                }
            }
        }
        grid
    }

    #[test]
    fn lines_reach_every_edge_they_name() {
        for code in 0x2500..=0x257F_u32 {
            let arms = ARMS[(code - 0x2500) as usize];
            if arms == 0 {
                continue;
            }
            let c = char::from_u32(code).unwrap();
            let g = cover(c);
            let [up, right, down, left] =
                [arms >> 12, (arms >> 8) & 0xF, (arms >> 4) & 0xF, arms & 0xF];
            assert_eq!(g[0].iter().any(|&v| v), up != 0, "{c} top edge");
            assert_eq!(g[17].iter().any(|&v| v), down != 0, "{c} bottom edge");
            assert_eq!(g.iter().any(|row| row[0]), left != 0, "{c} left edge");
            assert_eq!(g.iter().any(|row| row[7]), right != 0, "{c} right edge");
        }
    }

    #[test]
    fn neighbours_join_without_gaps() {
        // ─ spans the whole width and │ the whole height, on the same line
        // as the arms of every other light character.
        let h = cover('─');
        let row = h.iter().position(|r| r[0]).unwrap();
        assert!(h[row].iter().all(|&v| v));
        let v = cover('│');
        let col = v[0].iter().position(|&v| v).unwrap();
        assert!(v.iter().all(|r| r[col]));
        let corner = cover('┌');
        assert!(corner[row][7] && corner[17][col] && corner[row][col]);
    }

    #[test]
    fn double_corners_close() {
        let g = cover('╔');
        // Outer corner: the top line and the left line meet.
        let top = g.iter().position(|r| r[7]).unwrap();
        let left = g[17].iter().position(|&v| v).unwrap();
        assert!(g[top][left]);
        // Inner corner leaves the inside of the pair open.
        assert!(!g[top + 1][left + 1]);
    }

    #[test]
    fn blocks_fill_the_cell() {
        assert!(cover('█').iter().all(|r| r.iter().all(|&v| v)));
        let upper = cover('▀');
        let lower = cover('▄');
        for y in 0..18 {
            assert!(upper[y][0] != lower[y][0], "halves meet at row {y}");
        }
        // ▛ is every quadrant but the lower right.
        let q = cover('▛');
        assert!(q[0][0] && q[0][7] && q[17][0] && !q[17][7]);
        assert!(cover('▐').iter().all(|r| r[7] && !r[0]));
    }

    #[test]
    fn paths_cover_arcs_diagonals_and_powerline() {
        for c in [
            '╭', '╮', '╯', '╰', '╱', '╲', '╳', '\u{E0B0}', '\u{E0B5}', '\u{E0B6}',
        ] {
            assert!(rects(c, CELL, 1).is_none(), "{c}");
            assert!(path(c, CELL, 1, 1.).unwrap().build().is_ok(), "{c}");
        }
    }
}
