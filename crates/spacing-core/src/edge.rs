//! Screen-edge switching for platforms without portal barriers
//! (Windows, macOS — and any backend with only absolute positions).
//!
//! Technique (same as Barrier/Deskflow): the cursor is clamped at the
//! screen edge by the OS — it cannot move further. Every *subsequent*
//! pointer event delivered while clamped means the user is still
//! pushing outward. `needed` consecutive outward pushes inside
//! `window` → activation, reported as the configured edge name.

use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

/// Screen bounds + enabled edges ("left"|"right"|"top"|"bottom").
pub struct EdgeDetector {
    edges: BTreeSet<String>,
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
    needed: u32,
    window: Duration,
    streaks: HashMap<String, (u32, Instant)>,
}

impl EdgeDetector {
    /// `rect` = the local screen bounds in the same coordinate space the
    /// backend reports (e.g. virtual screen rect on Windows, main
    /// display points on macOS). Edge names not in
    /// `left|right|top|bottom` are ignored.
    pub fn new(
        rect: (i32, i32, i32, i32),
        edges: impl IntoIterator<Item = String>,
    ) -> Self {
        Self::with_params(rect, edges, 4, Duration::from_millis(1500))
    }

    pub fn with_params(
        rect: (i32, i32, i32, i32),
        edges: impl IntoIterator<Item = String>,
        needed: u32,
        window: Duration,
    ) -> Self {
        let (min_x, min_y, max_x, max_y) = rect;
        let edges = edges
            .into_iter()
            .filter(|e| matches!(e.as_str(), "left" | "right" | "top" | "bottom"))
            .collect();
        Self {
            edges,
            min_x,
            min_y,
            max_x,
            max_y,
            needed,
            window,
            streaks: HashMap::new(),
        }
    }

    /// Whether `(x, y)` sits inside the named edge's tolerance zone.
    /// A few px of inward wiggle keeps the cursor "at the edge" —
    /// hi-res mice and emulated devices jitter ±1px at the clamp, and
    /// that noise must not count as pulling away.
    fn in_zone(&self, edge: &str, x: i32, y: i32) -> bool {
        const TOL: i32 = 4;
        match edge {
            "right" => x >= self.max_x - TOL,
            "left" => x <= self.min_x + TOL,
            "bottom" => y >= self.max_y - TOL,
            "top" => y <= self.min_y + TOL,
            _ => false,
        }
    }

    /// Feed one pointer event. `x,y` = clamped cursor position as the OS
    /// reports it; `dx,dy` = motion delta for this event (may be 0 when
    /// the cursor is pinned at an edge). Returns the activated edge name
    /// once its push threshold is met.
    pub fn feed(&mut self, x: i32, y: i32, dx: i32, dy: i32) -> Option<String> {
        self.feed_at(x, y, dx, dy, Instant::now())
    }

    /// Deterministic seam for tests.
    pub fn feed_at(
        &mut self,
        x: i32,
        y: i32,
        dx: i32,
        dy: i32,
        now: Instant,
    ) -> Option<String> {
        for edge in self.edges.clone() {
            let dir = match edge.as_str() {
                "right" => dx >= 0,
                "left" => dx <= 0,
                "bottom" => dy >= 0,
                "top" => dy <= 0,
                _ => false,
            };
            let zone = self.in_zone(&edge, x, y);
            let pushing = dir && zone;
            if !pushing {
                // Neutral vs reset: an inward move that stays inside the
                // edge zone is jitter, not a pull-away — only leaving
                // the zone resets the streak.
                if !zone {
                    self.streaks.remove(&edge);
                }
                continue;
            }
            let entry = self.streaks.entry(edge.clone()).or_insert((0, now));
            let fresh = now.duration_since(entry.1) <= self.window;
            entry.0 = if fresh { entry.0 + 1 } else { 1 };
            entry.1 = now;
            if entry.0 >= self.needed {
                self.streaks.clear();
                return Some(edge);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RIGHT: (i32, i32, i32, i32) = (0, 0, 1919, 1079);

    fn det(edges: &[&str]) -> EdgeDetector {
        EdgeDetector::with_params(
            RIGHT,
            edges.iter().map(|s| s.to_string()),
            3,
            Duration::from_millis(1000),
        )
    }

    #[test]
    fn right_edge_activates_after_threshold() {
        let mut d = det(&["right"]);
        let t = Instant::now();
        assert_eq!(d.feed_at(1900, 500, 30, 0, t), None); // approach — not clamped
        assert_eq!(d.feed_at(1919, 500, 19, 0, t), None); // push 1 (arrives clamped)
        assert_eq!(d.feed_at(1919, 500, 0, 0, t), None); // push 2
        assert_eq!(d.feed_at(1919, 500, 0, 0, t), Some("right".into())); // push 3
    }

    #[test]
    fn streak_resets_when_moving_off_edge() {
        let mut d = det(&["right"]);
        let t = Instant::now();
        d.feed_at(1919, 500, 10, 0, t);
        d.feed_at(1919, 500, 0, 0, t);
        d.feed_at(1800, 500, -50, 0, t); // back inside → reset
        assert_eq!(d.feed_at(1919, 500, 60, 0, t), None);
        assert_eq!(d.feed_at(1919, 500, 0, 0, t), None);
        assert_eq!(d.feed_at(1919, 500, 0, 0, t), Some("right".into()));
    }

    #[test]
    fn stale_streak_expires() {
        let mut d = det(&["right"]);
        let t0 = Instant::now();
        d.feed_at(1919, 500, 10, 0, t0);
        d.feed_at(1919, 500, 0, 0, t0);
        let t1 = t0 + Duration::from_millis(1500); // past 1s window
        assert_eq!(d.feed_at(1919, 500, 0, 0, t1), None); // streak restarted at 1
        assert_eq!(d.feed_at(1919, 500, 0, 0, t1), None);
        assert_eq!(d.feed_at(1919, 500, 0, 0, t1), Some("right".into()));
    }

    #[test]
    fn unconfigured_edge_never_fires() {
        let mut d = det(&["left"]);
        let t = Instant::now();
        for _ in 0..10 {
            assert_eq!(d.feed_at(1919, 500, 0, 0, t), None); // right ignored
        }
    }

    #[test]
    fn jitter_at_edge_does_not_reset() {
        let mut d = det(&["right"]);
        let t = Instant::now();
        // Emulated/hi-res devices report ±1 corrections at the clamp —
        // an inward wiggle inside the tolerance zone is neutral, not a
        // pull-away; the push streak must survive it.
        assert_eq!(d.feed_at(1919, 500, 10, 0, t), None); // push 1
        assert_eq!(d.feed_at(1919, 500, 1, 0, t), None); // push 2
        assert_eq!(d.feed_at(1918, 500, -1, 0, t), None); // jitter — neutral
        assert_eq!(d.feed_at(1919, 500, 1, 0, t), Some("right".into())); // push 3
    }

    #[test]
    fn inward_motion_at_edge_does_not_count() {
        let mut d = det(&["right"]);
        let t = Instant::now();
        // Cursor still reported at max_x but moving inward — not a push.
        assert_eq!(d.feed_at(1919, 500, -5, 0, t), None);
        assert_eq!(d.feed_at(1919, 500, -5, 0, t), None);
        assert_eq!(d.feed_at(1919, 500, -5, 0, t), None);
    }

    #[test]
    fn top_and_bottom() {
        let mut d = det(&["top", "bottom"]);
        let t = Instant::now();
        for _ in 0..3 {
            d.feed_at(500, 0, 0, -10, t);
        }
        // top hit threshold first in the loop above? feed x3 → fires on 3rd
        // (the call above returns after feeding; verify bottom fires fresh)
        let mut d2 = det(&["top", "bottom"]);
        assert_eq!(d2.feed_at(500, 1079, 0, 10, t), None);
        assert_eq!(d2.feed_at(500, 1079, 0, 0, t), None);
        assert_eq!(d2.feed_at(500, 1079, 0, 0, t), Some("bottom".into()));
    }
}
