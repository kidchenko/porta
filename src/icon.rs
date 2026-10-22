//! Draws the tray icon: a door.
//!
//! | face     | door                 | means                               |
//! |----------|----------------------|-------------------------------------|
//! | `Closed` | shut, with a knob    | nothing of yours is listening       |
//! | `Open`   | ajar                 | at least one of your servers is up  |
//!
//! Drawn from signed distance functions with 4x4 supersampling, like the
//! other apps in this family: crisp at any size, no image decoder. On macOS
//! both faces are template images that AppKit tints for the menu bar.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Face {
    Closed,
    Open,
}

/// What the tray asks for. Windows downsamples to the notification-area size
/// itself, and starting from 32 keeps 150% and 200% DPI sharp.
pub const EDGE: u32 = 32;

const GRID: f32 = 32.0;
const SS: u32 = 4;

type Rgb = [f32; 3];

/// RGBA8, `edge` x `edge`, straight alpha.
pub fn render(face: Face, dark_taskbar: bool, edge: u32) -> Vec<u8> {
    let (line, panel): (Rgb, Rgb) = if cfg!(target_os = "macos") {
        ([0.0; 3], [0.0; 3])
    } else if dark_taskbar {
        ([0.95, 0.95, 0.95], [0.40, 0.68, 1.0])
    } else {
        ([0.12, 0.12, 0.12], [0.12, 0.42, 0.86])
    };
    supersample(edge, |x, y| sample(face, x, y, line, panel))
}

fn supersample(edge: u32, f: impl Fn(f32, f32) -> Option<Rgb>) -> Vec<u8> {
    let scale = GRID / edge as f32;
    let n = (SS * SS) as f32;
    let mut out = Vec::with_capacity((edge * edge * 4) as usize);
    for py in 0..edge {
        for px in 0..edge {
            let mut acc = [0.0f32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) * scale;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) * scale;
                    if let Some(c) = f(x, y) {
                        acc[0] += c[0];
                        acc[1] += c[1];
                        acc[2] += c[2];
                        acc[3] += 1.0;
                    }
                }
            }
            if acc[3] == 0.0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                out.extend_from_slice(&[
                    (acc[0] / acc[3] * 255.0).round() as u8,
                    (acc[1] / acc[3] * 255.0).round() as u8,
                    (acc[2] / acc[3] * 255.0).round() as u8,
                    (acc[3] / n * 255.0).round() as u8,
                ]);
            }
        }
    }
    out
}

/// The door frame, as a filled shape; its outline is `frame().abs() - w`.
fn frame(x: f32, y: f32) -> f32 {
    rounded_box(x, y, 16.0, 16.5, 9.5, 13.0, 2.0)
}

/// The door leaf swung open on its left hinge, foreshortened: a quad that is
/// taller at the hinge than at its free edge.
fn leaf(x: f32, y: f32) -> f32 {
    let (a, b, c, d) = ((7.0, 3.0), (16.5, 6.5), (16.5, 26.5), (7.0, 30.0));
    triangle(x, y, a, b, c).min(triangle(x, y, a, c, d))
}

fn sample(face: Face, x: f32, y: f32, line: Rgb, panel: Rgb) -> Option<Rgb> {
    let outline = frame(x, y).abs() - 1.3 <= 0.0;
    match face {
        Face::Closed => {
            let knob = len(x - 21.5, y - 17.5) - 1.7 <= 0.0;
            (outline || knob).then_some(line)
        }
        Face::Open => {
            if leaf(x, y) <= 0.0 {
                // The knob is a hole in the leaf, so it reads in one colour too.
                return (len(x - 14.0, y - 17.0) - 1.4 > 0.0).then_some(panel);
            }
            outline.then_some(line)
        }
    }
}

fn len(x: f32, y: f32) -> f32 {
    (x * x + y * y).sqrt()
}

fn rounded_box(x: f32, y: f32, cx: f32, cy: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let qx = (x - cx).abs() - (hx - r);
    let qy = (y - cy).abs() - (hy - r);
    len(qx.max(0.0), qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

/// Exact signed distance to a triangle (Inigo Quilez's formulation).
fn triangle(x: f32, y: f32, p0: (f32, f32), p1: (f32, f32), p2: (f32, f32)) -> f32 {
    let e = [
        (p1.0 - p0.0, p1.1 - p0.1),
        (p2.0 - p1.0, p2.1 - p1.1),
        (p0.0 - p2.0, p0.1 - p2.1),
    ];
    let v = [
        (x - p0.0, y - p0.1),
        (x - p1.0, y - p1.1),
        (x - p2.0, y - p2.1),
    ];
    let s = (e[0].0 * e[2].1 - e[0].1 * e[2].0).signum();
    let mut dist = f32::MAX;
    let mut sign = f32::MAX;
    for i in 0..3 {
        let (ex, ey) = e[i];
        let (vx, vy) = v[i];
        let h = ((vx * ex + vy * ey) / (ex * ex + ey * ey)).clamp(0.0, 1.0);
        let (qx, qy) = (vx - ex * h, vy - ey * h);
        dist = dist.min(qx * qx + qy * qy);
        sign = sign.min(s * (vx * ey - vy * ex));
    }
    -dist.sqrt() * sign.signum()
}

/// The application icon: an open door on a blue tile. The room beyond is dark
/// and the leaf pale, so it reads as a door swung open.
pub fn render_app(edge: u32) -> Vec<u8> {
    const CREAM: Rgb = [0.99, 0.96, 0.89];
    // The leaf catches the light: paler than the tile, bluer than the frame.
    const LEAF: Rgb = [0.78, 0.88, 1.0];
    const NIGHT: Rgb = [0.05, 0.11, 0.27];
    const TOP: Rgb = [0.42, 0.68, 1.0];
    const BOTTOM: Rgb = [0.11, 0.30, 0.72];
    const GLYPH: f32 = 0.70;

    let tile = |y: f32| {
        let t = (y / GRID).clamp(0.0, 1.0);
        [0, 1, 2].map(|i| TOP[i] + (BOTTOM[i] - TOP[i]) * t)
    };
    supersample(edge, |x, y| {
        if rounded_box(x, y, 16.0, 16.0, 15.0, 15.0, 7.0) > 0.0 {
            return None;
        }
        let gx = 16.0 + (x - 16.0) / GLYPH;
        let gy = 16.5 + (y - 16.0) / GLYPH;
        Some(if leaf(gx, gy) <= 0.0 {
            if len(gx - 14.0, gy - 17.0) - 1.4 <= 0.0 {
                NIGHT
            } else {
                LEAF
            }
        } else if frame(gx, gy).abs() - 1.4 <= 0.0 {
            CREAM
        } else if frame(gx, gy) < 0.0 {
            NIGHT
        } else {
            tile(y)
        })
    })
}

/// What a menu row's dot says, where there is no program icon to show.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dot {
    /// A server you started.
    Yours,
    /// An app's or a service's.
    Other,
}

/// A small filled circle for a menu row, 32 px (menus draw it at 16).
pub fn dot(kind: Dot) -> Vec<u8> {
    let colour: Rgb = match kind {
        Dot::Yours => [0.30, 0.62, 1.0],
        Dot::Other => [0.55, 0.55, 0.58],
    };
    supersample(32, |x, y| {
        (len(x - 16.0, y - 16.0) - 6.5 <= 0.0).then_some(colour)
    })
}

/// The mark beside "Stop": a red square, the stop symbol. It is the one item
/// with a consequence, so the eye should find it without reading.
pub fn stop_mark() -> Vec<u8> {
    let red: Rgb = [0.90, 0.26, 0.24];
    supersample(32, |x, y| {
        (rounded_box(x, y, 16.0, 16.0, 7.5, 7.5, 2.0) <= 0.0).then_some(red)
    })
}

/// Sizes the app icon is written at: everything an .ico, an .icns iconset
/// and the docs want, each drawn rather than resampled.
pub const APP_SIZES: [u32; 10] = [16, 20, 24, 32, 40, 48, 64, 128, 256, 1024];

/// `porta --dump-icons <dir>`: every tray face and app icon size as raw RGBA.
/// `assets/make_icons.py` turns them into icon.ico and the PNGs.
pub fn dump(dir: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, face) in [("closed", Face::Closed), ("open", Face::Open)] {
        for (theme, dark) in [("dark", true), ("light", false)] {
            for edge in [EDGE, 128] {
                std::fs::write(
                    format!("{dir}/tray-{name}-{theme}-{edge}.rgba"),
                    render(face, dark, edge),
                )?;
            }
        }
    }
    for edge in APP_SIZES {
        std::fs::write(format!("{dir}/app-{edge}.rgba"), render_app(edge))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ink(face: Face) -> u32 {
        render(face, true, EDGE)
            .chunks(4)
            .map(|p| p[3] as u32)
            .sum()
    }

    #[test]
    fn faces_are_distinct() {
        assert!(ink(Face::Closed) > 0);
        assert!(
            ink(Face::Open) > ink(Face::Closed),
            "the open leaf adds ink"
        );
    }

    #[test]
    fn corners_are_transparent() {
        let px = render(Face::Open, true, EDGE);
        assert_eq!(px[3], 0);
        assert_eq!(px[px.len() - 1], 0);
    }

    #[test]
    fn app_icon_has_rounded_transparent_corners_and_opaque_centre() {
        let edge = 64;
        let px = render_app(edge);
        assert_eq!(px[3], 0);
        let centre = ((edge / 2 * edge + edge / 2) * 4 + 3) as usize;
        assert_eq!(px[centre], 255);
    }
}
