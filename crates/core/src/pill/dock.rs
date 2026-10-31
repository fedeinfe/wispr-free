//! Where the pill window sits: edge docking, display following and drag snapping.
//!
//! Positions are in **desktop units**: one top-left-origin coordinate space across all screens,
//! whatever the OS natively uses for placing windows (points on macOS, pixels on Windows and
//! X11). Sizes of Slint content are in logical points; `Display::units_per_point` converts.

use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.x + self.width && p.y >= self.y && p.y < self.y + self.height
    }

    fn distance_to(&self, p: Point) -> f64 {
        let dx = (self.x - p.x).max(p.x - (self.x + self.width)).max(0.0);
        let dy = (self.y - p.y).max(p.y - (self.y + self.height)).max(0.0);
        dx.hypot(dy)
    }
}

/// One screen. `work_area` excludes the macOS Dock and menu bar, or the Windows taskbar.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Display {
    pub bounds: Rect,
    pub work_area: Rect,
    pub units_per_point: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Edge {
    #[default]
    Bottom,
    Left,
    Right,
}

impl Edge {
    pub fn as_str(self) -> &'static str {
        match self {
            Edge::Bottom => "bottom",
            Edge::Left => "left",
            Edge::Right => "right",
        }
    }
}

impl FromStr for Edge {
    type Err = UnknownEdge;

    fn from_str(s: &str) -> Result<Self, UnknownEdge> {
        match s {
            "bottom" => Ok(Edge::Bottom),
            "left" => Ok(Edge::Left),
            "right" => Ok(Edge::Right),
            _ => Err(UnknownEdge),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownEdge;

/// The screen under the cursor, or the closest one when the cursor is in a gap between screens
/// of different sizes.
pub fn display_at(displays: &[Display], cursor: Point) -> Option<&Display> {
    displays
        .iter()
        .find(|d| d.bounds.contains(cursor))
        .or_else(|| {
            displays.iter().min_by(|a, b| {
                let (a, b) = (a.bounds.distance_to(cursor), b.bounds.distance_to(cursor));
                a.total_cmp(&b)
            })
        })
}

/// Top-left of a `window` (in points) docked to `edge`, flush with the work area and centred
/// along it.
pub fn dock_origin(display: &Display, edge: Edge, window: Size) -> Point {
    let area = display.work_area;
    let width = window.width * display.units_per_point;
    let height = window.height * display.units_per_point;
    let centre_x = area.x + (area.width - width) / 2.0;
    let centre_y = area.y + (area.height - height) / 2.0;
    match edge {
        Edge::Bottom => Point {
            x: centre_x,
            y: area.y + area.height - height,
        },
        Edge::Left => Point {
            x: area.x,
            y: centre_y,
        },
        Edge::Right => Point {
            x: area.x + area.width - width,
            y: centre_y,
        },
    }
}

/// The dockable edge closest to `p`. There is no top dock.
pub fn nearest_edge(display: &Display, p: Point) -> Edge {
    let area = display.work_area;
    let distances = [
        (Edge::Bottom, area.y + area.height - p.y),
        (Edge::Left, p.x - area.x),
        (Edge::Right, area.x + area.width - p.x),
    ];
    distances
        .into_iter()
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map_or(Edge::Bottom, |(edge, _)| edge)
}
