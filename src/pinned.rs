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
    /// Split a session surface into (main_area, pin_area) for this pin, or
    /// None when the surface is too small to fit the pin AND a usable tab
    /// area (hide the pin instead of squeezing both).
    ///
    /// Right-side pins are content-fit to [MIN_PIN_WIDTH, MAX_PIN_WIDTH] —
    /// brn dash renders 30–90 cols (its own clamp), so the pin never
    /// squeezes below the content floor or wastes space beyond its cap.
    pub fn split_area(&self, area: Rect) -> Option<(Rect, Rect)> {
        match self.side {
            PinnedSide::Right => {
                let width = (area.width as f32 * self.ratio)
                    .round()
                    .clamp(MIN_PIN_WIDTH as f32, MAX_PIN_WIDTH as f32) as u16;
                if width + MIN_TAB_WIDTH > area.width {
                    return None; // not enough room — hide
                }
                Some((
                    Rect::new(area.x, area.y, area.width - width, area.height),
                    Rect::new(area.x + area.width - width, area.y, width, area.height),
                ))
            }
            PinnedSide::Down => {
                let height = (area.height as f32 * self.ratio)
                    .round()
                    .clamp(MIN_PIN_HEIGHT as f32, MAX_PIN_HEIGHT as f32) as u16;
                if height + MIN_TAB_HEIGHT > area.height {
                    return None; // not enough room — hide
                }
                Some((
                    Rect::new(area.x, area.y, area.width, area.height - height),
                    Rect::new(area.x, area.y + area.height - height, area.width, height),
                ))
            }
        }
    }
}

/// Minimum width a right-side pin may claim — brn dash's own render floor
/// (Math.max(30, …) in brn-dash.ts); below this it overflows the pane.
const MIN_PIN_WIDTH: u16 = 30;
/// Maximum width a right-side pin claims — brn dash caps at 90 cols.
const MAX_PIN_WIDTH: u16 = 90;
/// Keep at least this much of the surface for the tab layout.
const MIN_TAB_WIDTH: u16 = 40;
/// Minimum height for a bottom pin (content floor).
const MIN_PIN_HEIGHT: u16 = 5;
/// Maximum height for a bottom pin.
const MAX_PIN_HEIGHT: u16 = 30;
/// Keep at least this much height for the tab layout.
const MIN_TAB_HEIGHT: u16 = 10;

/// Compute (tab_area, [(pin, pin_rect)]) for the session's pinned panes.
/// Pins that cannot fit (surface too small) are omitted — they stay alive
/// but are not carved or rendered until there is room again.
pub fn split_pinned_area(pinned: &[PinnedPane], area: Rect) -> (Rect, Vec<(PinnedPane, Rect)>) {
    let mut main = area;
    let mut rects = Vec::with_capacity(pinned.len());
    for pin in pinned {
        if let Some((m, p)) = pin.split_area(main) {
            main = m;
            rects.push((*pin, p));
        }
    }
    (main, rects)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    fn pin(side: PinnedSide, ratio: f32) -> PinnedPane {
        PinnedPane { pane_id: crate::layout::PaneId::from_raw(1), side, ratio }
    }

    #[test]
    fn right_pin_respects_ratio_within_content_bounds() {
        // 200-col surface, ratio 0.25 -> 50 cols, tab keeps 150.
        let (tab, p) = pin(PinnedSide::Right, 0.25).split_area(Rect::new(0, 0, 200, 40)).unwrap();
        assert_eq!(p.width, 50);
        assert_eq!(tab.width, 150);
    }

    #[test]
    fn right_pin_caps_at_content_max() {
        // 400-col surface, ratio 0.25 would be 100 -> clamped to MAX_PIN_WIDTH 90.
        let (tab, p) = pin(PinnedSide::Right, 0.25).split_area(Rect::new(0, 0, 400, 40)).unwrap();
        assert_eq!(p.width, 90);
        assert_eq!(tab.width, 310);
    }

    #[test]
    fn right_pin_raises_to_content_min() {
        // 120-col surface, ratio 0.25 would be 30 -> at MIN_PIN_WIDTH, tab 90.
        let (tab, p) = pin(PinnedSide::Right, 0.25).split_area(Rect::new(0, 0, 120, 40)).unwrap();
        assert_eq!(p.width, 30);
        assert_eq!(tab.width, 90);
    }

    #[test]
    fn right_pin_hides_when_surface_too_small() {
        // 60-col surface: min pin 30 + min tab 40 = 70 > 60 -> hide (None).
        assert!(pin(PinnedSide::Right, 0.25).split_area(Rect::new(0, 0, 60, 40)).is_none());
        // 69-col surface also hides; 70-col fits exactly.
        assert!(pin(PinnedSide::Right, 0.25).split_area(Rect::new(0, 0, 69, 40)).is_none());
        assert!(pin(PinnedSide::Right, 0.25).split_area(Rect::new(0, 0, 70, 40)).is_some());
    }

    #[test]
    fn split_pinned_area_omits_hidden_pins() {
        let (main, rects) = split_pinned_area(&[pin(PinnedSide::Right, 0.25)], Rect::new(0, 0, 60, 40));
        assert!(rects.is_empty());
        assert_eq!(main.width, 60); // tab keeps the full surface
    }
}
