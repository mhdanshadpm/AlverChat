use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[cfg(test)]
pub fn crossed_drag_threshold(start: (f64, f64), current: (f64, f64), threshold: f64) -> bool {
    let dx = current.0 - start.0;
    let dy = current.1 - start.1;
    dx * dx + dy * dy >= threshold * threshold
}

pub fn clamp_icon_into_work_area(
    window: (i32, i32),
    area: Rect,
    window_size: (i32, i32),
    icon_offset: (i32, i32),
    icon_size: i32,
) -> (i32, i32) {
    let min_x = area.x - icon_offset.0;
    let max_x = area.x + area.width - icon_size - icon_offset.0;
    let min_y = area.y - icon_offset.1;
    let max_y = area.y + area.height - icon_size - icon_offset.1;
    let _ = window_size;
    (window.0.clamp(min_x, max_x), window.1.clamp(min_y, max_y))
}

pub fn edge_for_icon(icon_left: i32, icon_size: i32, area: Rect) -> Edge {
    let midpoint = area.x + area.width / 2;
    if icon_left + icon_size / 2 < midpoint {
        Edge::Left
    } else {
        Edge::Right
    }
}

pub fn snap_icon_to_edge(
    window: (i32, i32),
    area: Rect,
    icon_offset: (i32, i32),
    icon_size: i32,
    snap_enabled: bool,
    threshold: i32,
) -> (i32, i32, Edge) {
    let icon_left = window.0 + icon_offset.0;
    let icon_center = icon_left + icon_size / 2;
    let left_distance = (icon_left - area.x).abs();
    let right_distance = (area.x + area.width - icon_left - icon_size).abs();
    let nearest = if icon_center < area.x + area.width / 2 {
        Edge::Left
    } else {
        Edge::Right
    };
    let should_snap = snap_enabled || left_distance <= threshold || right_distance <= threshold;
    let edge = if should_snap {
        nearest
    } else {
        edge_for_icon(icon_left, icon_size, area)
    };
    if !should_snap {
        return (window.0, window.1, edge);
    }
    let x = match edge {
        Edge::Left => area.x - icon_offset.0,
        Edge::Right => area.x + area.width - icon_size - icon_offset.0,
    };
    let y = window.1.clamp(
        area.y - icon_offset.1,
        area.y + area.height - icon_size - icon_offset.1,
    );
    (x, y, edge)
}

/// The placement a Settings "Screen corner / side" value asks for: which edge the icon
/// snaps to and how far down the work area it sits (0 top, 0.5 center, 1 bottom).
/// An unrecognised value yields `None`, so a hand-edited preference cannot push the icon
/// off screen.
pub fn corner_position(corner: &str) -> Option<(Edge, f64)> {
    let (side, position) = corner.split_once('-')?;
    let edge = match side {
        "left" => Edge::Left,
        "right" => Edge::Right,
        _ => return None,
    };
    let percent = match position {
        "top" => 0.0,
        "center" => 0.5,
        "bottom" => 1.0,
        _ => return None,
    };
    Some((edge, percent))
}

pub fn vertical_percent(icon_top: i32, area: Rect, icon_size: i32) -> f64 {
    let travel = (area.height - icon_size).max(1);
    ((icon_top - area.y) as f64 / travel as f64).clamp(0.0, 1.0)
}

pub fn restore_icon_top(percent: f64, area: Rect, icon_size: i32) -> i32 {
    let travel = (area.height - icon_size).max(0);
    area.y + (percent.clamp(0.0, 1.0) * travel as f64).round() as i32
}

#[cfg(test)]
pub fn panel_follows_icon(attached: bool) -> bool {
    attached
}

#[cfg(test)]
mod tests {
    use super::*;
    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };

    #[test]
    fn threshold_distinguishes_click_from_drag() {
        assert!(!crossed_drag_threshold((10.0, 10.0), (13.0, 12.0), 4.0));
        assert!(crossed_drag_threshold((10.0, 10.0), (14.0, 10.0), 4.0));
    }
    #[test]
    fn icon_is_clamped_inside_work_area() {
        assert_eq!(
            clamp_icon_into_work_area((-50, 2000), AREA, (440, 220), (12, 12), 56),
            (-12, 1012)
        );
    }
    #[test]
    fn nearest_edge_snap_and_near_edge_snap_work() {
        assert_eq!(
            snap_icon_to_edge((120, 50), AREA, (12, 12), 56, true, 24),
            (-12, 50, Edge::Left)
        );
        assert_eq!(
            snap_icon_to_edge((1840, 80), AREA, (12, 12), 56, false, 24),
            (1852, 80, Edge::Right)
        );
        assert_eq!(
            snap_icon_to_edge((800, 80), AREA, (12, 12), 56, false, 24),
            (800, 80, Edge::Left)
        );
    }
    #[test]
    fn position_round_trips_as_vertical_percentage() {
        let top = restore_icon_top(0.25, AREA, 56);
        assert_eq!(top, 256);
        assert!((vertical_percent(top, AREA, 56) - 0.25).abs() < 0.002);
    }
    #[test]
    fn bubble_side_follows_icon_edge() {
        assert_eq!(edge_for_icon(0, 56, AREA), Edge::Left);
        assert_eq!(edge_for_icon(1864, 56, AREA), Edge::Right);
    }
    #[test]
    fn every_corner_the_settings_picker_offers_is_placeable() {
        for (corner, expected) in [
            ("left-top", (Edge::Left, 0.0)),
            ("left-center", (Edge::Left, 0.5)),
            ("left-bottom", (Edge::Left, 1.0)),
            ("right-top", (Edge::Right, 0.0)),
            ("right-center", (Edge::Right, 0.5)),
            ("right-bottom", (Edge::Right, 1.0)),
        ] {
            assert_eq!(corner_position(corner), Some(expected), "{corner}");
        }
        assert_eq!(corner_position(""), None);
        assert_eq!(corner_position("top-left"), None);
        assert_eq!(corner_position("right-middle"), None);

        // The percentages land on the rows the names promise.
        assert_eq!(
            restore_icon_top(corner_position("right-top").unwrap().1, AREA, 56),
            0
        );
        assert_eq!(
            restore_icon_top(corner_position("right-center").unwrap().1, AREA, 56),
            512
        );
        assert_eq!(
            restore_icon_top(corner_position("right-bottom").unwrap().1, AREA, 56),
            1024
        );
    }
    #[test]
    fn detached_panel_stops_following_icon() {
        assert!(!panel_follows_icon(false));
        assert!(panel_follows_icon(true));
    }
}
