//! Session-level pinned panes.
//!
//! A pinned pane persists across every workspace and tab of the session —
//! the brn dash sidebar (Right) or bottom bar (Down). Pinned panes own
//! their pane state at session level; each tab's tile layout renders into
//! the area the pins leave free.

use ratatui::layout::Rect;

use crate::layout::PaneId;

/// Which edge a pinned pane claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinnedSide {
    /// Right column (sidebar).
    #[default]
    Right,
    /// Bottom row (horizontal bar).
    Down,
}

/// A pane pinned at session level: visible in every workspace × tab.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PinnedPane {
    pub pane_id: PaneId,
    pub side: PinnedSide,
    /// Share of the remaining session surface this pane claims (0.0..1.0).
    pub ratio: f32,
}

impl PinnedPane {
    /// Split a session surface into (main_area, pin_area) for this pin.
    /// Applied in pin order — later pins split what earlier pins left.
    pub fn split_area(&self, area: Rect) -> (Rect, Rect) {
        match self.side {
            PinnedSide::Right => {
                let width = (area.width as f32 * self.ratio)
                    .round()
                    .clamp(12.0, area.width as f32) as u16;
                (
                    Rect::new(area.x, area.y, area.width - width, area.height),
                    Rect::new(area.x + area.width - width, area.y, width, area.height),
                )
            }
            PinnedSide::Down => {
                let height = (area.height as f32 * self.ratio)
                    .round()
                    .clamp(5.0, area.height as f32) as u16;
                (
                    Rect::new(area.x, area.y, area.width, area.height - height),
                    Rect::new(area.x, area.y + area.height - height, area.width, height),
                )
            }
        }
    }
}

/// Compute (tab_area, [(pin, pin_rect)]) for the session's pinned panes.
pub fn split_pinned_area(pinned: &[PinnedPane], area: Rect) -> (Rect, Vec<(PinnedPane, Rect)>) {
    let mut main = area;
    let mut rects = Vec::with_capacity(pinned.len());
    for pin in pinned {
        let (m, p) = pin.split_area(main);
        main = m;
        rects.push((*pin, p));
    }
    (main, rects)
}
