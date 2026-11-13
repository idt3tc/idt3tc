//! Parsers for the value syntaxes used on the command line.

use super::image::Color;
use super::sampling::{EdgeMode, Interp};
use crate::common::util::{parse_double, split};
use crate::common::{Error, Result};

/// A length that is either absolute (pixels) or relative to a reference
/// length ("50%").
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Length {
    pub value: f64,
    pub percent: bool,
}

impl Length {
    pub fn resolve(&self, reference: f64) -> f64 {
        if self.percent { self.value * reference / 100.0 } else { self.value }
    }
}

pub fn parse_length(s: &str) -> Option<Length> {
    if s.is_empty() {
        return None;
    }
    match s.strip_suffix('%') {
        Some(num) => Some(Length { value: parse_double(num)?, percent: true }),
        None => Some(Length { value: parse_double(s)?, percent: false }),
    }
}

/// "[W][xH][{+-}X{+-}Y]" with W, H, X, Y either pixels or percentages,
/// e.g. "512x512", "512x", "x256", "50%", "256x256+16+0", "+32-8".
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Geometry {
    pub has_w: bool,
    pub has_h: bool,
    pub has_offset: bool,
    pub w: Length,
    pub h: Length,
    pub x: Length,
    pub y: Length,
}

/// Fails with a usage error describing what is wrong with `s`.
pub fn parse_geometry(s: &str, what: &str) -> Result<Geometry> {
    let mut g = Geometry::default();
    // The offset starts at the first '+' or '-' that follows the size part
    // (a leading sign would be meaningless for a size).
    let (size, offset) = match s.find(['+', '-']) {
        Some(p) => (&s[..p], &s[p..]),
        None => (s, ""),
    };
    if !size.is_empty() {
        let xpos = size.find(['x', 'X']);
        let (ws, hs) = match xpos {
            Some(p) => (&size[..p], &size[p + 1..]),
            None => (size, ""),
        };
        if !ws.is_empty() {
            g.w = parse_length(ws)
                .filter(|l| l.value > 0.0)
                .ok_or_else(|| Error::usage(format!("{what}: invalid width '{ws}' in '{s}'")))?;
            g.has_w = true;
        }
        if !hs.is_empty() {
            g.h = parse_length(hs)
                .filter(|l| l.value > 0.0)
                .ok_or_else(|| Error::usage(format!("{what}: invalid height '{hs}' in '{s}'")))?;
            g.has_h = true;
        }
        // A bare "50%" scales both axes, like ImageMagick.
        if xpos.is_none() && g.w.percent {
            g.h = g.w;
            g.has_h = true;
        }
    }
    if !offset.is_empty() {
        // Expect exactly two signed components: {+-}X{+-}Y.
        let Some(second) = offset[1..].find(['+', '-']).map(|p| p + 1) else {
            return Err(Error::usage(format!("{what}: offset needs both X and Y (e.g. +16+0) in '{s}'")));
        };
        let xs = offset[..second].strip_prefix('+').unwrap_or(&offset[..second]);
        let ys = offset[second..].strip_prefix('+').unwrap_or(&offset[second..]);
        match (parse_length(xs), parse_length(ys)) {
            (Some(x), Some(y)) => {
                g.x = x;
                g.y = y;
            }
            _ => return Err(Error::usage(format!("{what}: invalid offset '{offset}' in '{s}'"))),
        }
        g.has_offset = true;
    }
    if !g.has_w && !g.has_h && !g.has_offset {
        return Err(Error::usage(format!("{what}: empty geometry")));
    }
    Ok(g)
}

/// "X,Y" pair of lengths (pixels or percent), e.g. "32,-16" or "50%,0".
pub fn parse_length_pair(s: &str, what: &str) -> Result<(Length, Length)> {
    let parts = split(s, ',');
    if parts.len() == 2
        && let (Some(x), Some(y)) = (parse_length(&parts[0]), parse_length(&parts[1]))
    {
        return Ok((x, y));
    }
    Err(Error::usage(format!("{what}: expected 'X,Y' (pixels or percent), got '{s}'")))
}

const NAMED_COLORS: &[(&str, [u8; 4])] = &[
    ("black", [0, 0, 0, 255]),
    ("white", [255, 255, 255, 255]),
    ("transparent", [0, 0, 0, 0]),
    ("none", [0, 0, 0, 0]),
    ("gray", [128, 128, 128, 255]),
    ("grey", [128, 128, 128, 255]),
    ("red", [255, 0, 0, 255]),
    ("green", [0, 255, 0, 255]),
    ("blue", [0, 0, 255, 255]),
    ("yellow", [255, 255, 0, 255]),
    ("cyan", [0, 255, 255, 255]),
    ("magenta", [255, 0, 255, 255]),
    // Flat tangent-space normal (0, 0, 1).
    ("normal", [128, 128, 255, 255]),
];

fn color_from_bytes(c: [i32; 4]) -> Color {
    Color::new(c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, c[3] as f32 / 255.0)
}

/// Colors: "#rgb", "#rgba", "#rrggbb", "#rrggbbaa" (the last two also
/// without '#'), "r,g,b[,a]" (0-255), or a name (black, white, transparent,
/// gray, red, green, blue, ...).
pub fn parse_color(s_in: &str) -> Option<Color> {
    let s = s_in.to_ascii_lowercase();
    if s.is_empty() {
        return None;
    }
    if let Some((_, c)) = NAMED_COLORS.iter().find(|(n, _)| *n == s) {
        return Some(color_from_bytes(c.map(i32::from)));
    }
    // '#' starts a comment in shells when it begins a word, so 6/8 digit
    // hex is also accepted without it ("ff8000").
    let bare_hex = (s.len() == 6 || s.len() == 8) && s.bytes().all(|b| b.is_ascii_hexdigit());
    if s.starts_with('#') || bare_hex {
        let h = if bare_hex { &s[..] } else { &s[1..] };
        let d: Vec<i32> = h.chars().map(|c| c.to_digit(16).map(|v| v as i32)).collect::<Option<_>>()?;
        let mut comps = [0, 0, 0, 255];
        match d.len() {
            3 | 4 => {
                for (i, v) in d.iter().enumerate() {
                    comps[i] = v * 17;
                }
            }
            6 | 8 => {
                for i in 0..d.len() / 2 {
                    comps[i] = d[2 * i] * 16 + d[2 * i + 1];
                }
            }
            _ => return None,
        }
        return Some(color_from_bytes(comps));
    }
    let parts = split(&s, ',');
    if parts.len() == 3 || parts.len() == 4 {
        let mut v = [0.0, 0.0, 0.0, 255.0];
        for (i, p) in parts.iter().enumerate() {
            v[i] = parse_double(p).filter(|x| (0.0..=255.0).contains(x))?;
        }
        return Some(Color::new(
            (v[0] / 255.0) as f32,
            (v[1] / 255.0) as f32,
            (v[2] / 255.0) as f32,
            (v[3] / 255.0) as f32,
        ));
    }
    None
}

pub fn parse_edge_mode(s: &str) -> Option<EdgeMode> {
    match s.to_ascii_lowercase().as_str() {
        "clamp" | "edge" => Some(EdgeMode::Clamp),
        "repeat" | "wrap" | "tile" => Some(EdgeMode::Repeat),
        "mirror" | "reflect" => Some(EdgeMode::Mirror),
        "fill" | "background" | "bg" => Some(EdgeMode::Fill),
        _ => None,
    }
}

pub fn edge_mode_name(m: EdgeMode) -> &'static str {
    match m {
        EdgeMode::Clamp => "clamp",
        EdgeMode::Repeat => "repeat",
        EdgeMode::Mirror => "mirror",
        EdgeMode::Fill => "fill",
    }
}

pub fn parse_interp(s: &str) -> Option<Interp> {
    match s.to_ascii_lowercase().as_str() {
        "nearest" | "point" => Some(Interp::Nearest),
        "bilinear" | "linear" | "triangle" => Some(Interp::Bilinear),
        "bicubic" | "cubic" | "catmullrom" => Some(Interp::Bicubic),
        _ => None,
    }
}

/// Placement of a region relative to a container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gravity {
    NorthWest,
    North,
    NorthEast,
    West,
    Center,
    East,
    SouthWest,
    South,
    SouthEast,
}

pub fn parse_gravity(s: &str) -> Option<Gravity> {
    use Gravity::*;
    Some(match s.to_ascii_lowercase().as_str() {
        "nw" | "northwest" | "topleft" | "top-left" => NorthWest,
        "n" | "north" | "top" => North,
        "ne" | "northeast" | "topright" | "top-right" => NorthEast,
        "w" | "west" | "left" => West,
        "c" | "center" | "centre" => Center,
        "e" | "east" | "right" => East,
        "sw" | "southwest" | "bottomleft" | "bottom-left" => SouthWest,
        "s" | "south" | "bottom" => South,
        "se" | "southeast" | "bottomright" | "bottom-right" => SouthEast,
        _ => return None,
    })
}

/// Origin of a `w x h` region inside a `cw x ch` container for a gravity,
/// before applying an offset.
pub fn gravity_origin(g: Gravity, cw: i64, ch: i64, w: i64, h: i64) -> (i64, i64) {
    use Gravity::*;
    // 0 = left/top, 1 = center, 2 = right/bottom
    let (col, row) = match g {
        NorthWest => (0, 0),
        North => (1, 0),
        NorthEast => (2, 0),
        West => (0, 1),
        Center => (1, 1),
        East => (2, 1),
        SouthWest => (0, 2),
        South => (1, 2),
        SouthEast => (2, 2),
    };
    // Floor division keeps odd differences consistent for negative values.
    let place = |mode: i32, container: i64, size: i64| {
        let diff = container - size;
        match mode {
            0 => 0,
            2 => diff,
            _ if diff >= 0 => diff / 2,
            _ => -((-diff + 1) / 2),
        }
    };
    (place(col, cw, w), place(row, ch, h))
}

/// Channel set such as "rgb", "a", "rgba" -> bitmask (r=1, g=2, b=4, a=8).
pub fn parse_channels(s: &str) -> Option<u32> {
    let s = s.to_ascii_lowercase();
    if s.is_empty() {
        return None;
    }
    if s == "all" {
        return Some(15);
    }
    let mut mask = 0;
    for c in s.chars() {
        mask |= match c {
            'r' => 1,
            'g' => 2,
            'b' => 4,
            'a' => 8,
            _ => return None,
        };
    }
    Some(mask)
}
