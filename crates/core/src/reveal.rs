//! The reveal of a clip (F072): a straight edge that sweeps across the canvas and uncovers what
//! the clip draws, so a shot, a title or a group appears through a diagonal wipe over what lies
//! beneath it.
//!
//! It is a **property of any visual clip**, not a clip of its own: the inspector section
//! "Revelação" holds the settings, the plan carries the edge already evaluated for the frame,
//! and the compositor cuts every layer the clip draws with it. A light beam that rides the edge
//! is an ordinary group clip (two rotated rectangles with keyed positions), not part of this
//! module.
//!
//! **Geometry.** Everything is in canvas pixels, `y` growing downwards. The edge passes through
//! the point `(edge, height / 2)` and leans `angle` degrees from the vertical; with a positive
//! angle the top of the edge leads in the direction of travel (the shape of a slash when the edge
//! travels right). A clip shows on the side the edge came from: left of it when it travels
//! right, right of it when it travels left.
//!
//! **Time.** The edge starts at `from` and ends at `to` (percent of the canvas width), eased over
//! `duration` frames that begin `offset` frames after the clip starts. A negative offset starts
//! the sweep before the clip does, which is how a split keeps the second half exactly where the
//! whole clip would have been.
//!
//! Pure: a function of the settings and a frame, with no clock, window or file.

use crate::easing::KeyEasing;

/// `none` draws the clip whole (the default); `wipe` is the straight edge described above.
pub const REVEAL_MODES: &[&str] = &["none", "wipe"];

/// The inspector properties of the section, in the order it lists them.
pub const REVEAL_PROPERTIES: [&str; 7] = [
    "reveal_mode",
    "reveal_from",
    "reveal_to",
    "reveal_angle",
    "reveal_duration",
    "reveal_offset",
    "reveal_easing",
];

/// What the inspector stores for a clip whose `reveal_mode` is `wipe`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RevealSettings {
    /// Where the edge is at mid height when the sweep starts, percent of the canvas width.
    pub from: f64,
    /// Where it is when the sweep ends.
    pub to: f64,
    /// Degrees from the vertical; positive leads with the top of the edge.
    pub angle: f64,
    /// Frames the sweep lasts (at least 1).
    pub duration: f64,
    /// Frames between the clip's first frame and the start of the sweep (negative: before it).
    pub offset: f64,
    pub easing: KeyEasing,
}

/// The edge as the plan carries it for one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reveal {
    /// Percent of the canvas width where the edge crosses mid height, already eased.
    pub edge: f64,
    pub angle: f64,
    /// The edge travels right: the clip shows to its left. Otherwise it shows to the right.
    pub rightward: bool,
}

impl RevealSettings {
    /// The sweep's progress at `local` frames after the clip's start, before easing.
    fn progress(&self, local: i64) -> f64 {
        ((local as f64 - self.offset) / self.duration.max(1.0)).clamp(0.0, 1.0)
    }

    /// The edge at `local` frames after the clip's start on a `width × height` canvas, or `None`
    /// once it has uncovered the whole canvas (the clip is then drawn exactly as without a
    /// reveal, so the plan, the occlusion hint and the picture are those of an unrevealed clip).
    pub fn at(&self, local: i64, width: f64, height: f64) -> Option<Reveal> {
        let eased = self.easing.apply(self.progress(local));
        let reveal = Reveal {
            edge: self.from + (self.to - self.from) * eased,
            angle: self.angle,
            rightward: self.to >= self.from,
        };
        if reveal.uncovers_everything(width, height) {
            return None;
        }
        Some(reveal)
    }
}

impl Reveal {
    /// `+1` when the clip shows to the left of the edge, `-1` to the right.
    pub fn direction(&self) -> f64 {
        if self.rightward { 1.0 } else { -1.0 }
    }

    /// The `x` of the edge on pixel row `y` (rows from the top of a `width × height` canvas).
    pub fn edge_x(&self, y: f64, width: f64, height: f64) -> f64 {
        let lean = self.angle.to_radians().tan();
        self.edge / 100.0 * width + self.direction() * lean * (height / 2.0 - y)
    }

    /// The share of pixel `(x, y)` (its centre) the clip shows through: `1` behind the edge, `0`
    /// ahead of it, one pixel of anti-aliasing across it. The runtime shader computes the same
    /// number, so this is its reference.
    pub fn coverage(&self, x: f64, y: f64, width: f64, height: f64) -> f64 {
        let across = (self.edge_x(y, width, height) - x) * self.direction();
        (0.5 + across * self.angle.to_radians().cos()).clamp(0.0, 1.0)
    }

    /// Whether every pixel of the canvas is fully behind the edge. Conservative: one extra
    /// pixel of margin (measured across the edge) so anti-aliasing can never hide a difference.
    pub fn uncovers_everything(&self, width: f64, height: f64) -> bool {
        let margin = 1.0 / self.angle.to_radians().cos().max(1e-6);
        let (top, bottom) = (
            self.edge_x(0.0, width, height),
            self.edge_x(height, width, height),
        );
        if self.rightward {
            top.min(bottom) >= width + margin
        } else {
            top.max(bottom) <= -margin
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 23 s reference at 1920×1080: the edge leans atan(118 / 540), moves from x = −300 to
    /// x = 2229 over 9 frames with smoothStep.
    fn reference() -> RevealSettings {
        RevealSettings {
            from: -300.0 / 19.2,
            to: 2229.0 / 19.2,
            angle: (118.0_f64 / 540.0).atan().to_degrees(),
            duration: 9.0,
            offset: 0.0,
            easing: KeyEasing::SmoothStep,
        }
    }

    const W: f64 = 1920.0;
    const H: f64 = 1080.0;

    #[test]
    fn the_edge_of_the_reference_passes_through_its_polygon_corners() {
        // Frame 3 of 9: x = mix(−300, 2229, smooth(1/3)); the corners are x ± 118 at top/bottom.
        let reveal = reference().at(3, W, H).unwrap();
        let p = KeyEasing::SmoothStep.apply(3.0 / 9.0);
        let x = -300.0 + (2229.0 + 300.0) * p;
        assert!((reveal.edge_x(0.0, W, H) - (x + 118.0)).abs() < 1e-9);
        assert!((reveal.edge_x(H, W, H) - (x - 118.0)).abs() < 1e-9);
        assert!((reveal.edge_x(H / 2.0, W, H) - x).abs() < 1e-9);
    }

    #[test]
    fn before_the_sweep_nothing_shows_and_after_it_the_reveal_is_gone() {
        let settings = reference();
        let first = settings.at(0, W, H).unwrap();
        // Edge at −300 leaning ±118 px: even the top corner (−182) is off the left of the canvas.
        for (x, y) in [(0.5, 0.5), (0.5, H - 0.5), (W / 2.0, H / 2.0)] {
            assert_eq!(first.coverage(x, y, W, H), 0.0, "({x}, {y})");
        }
        assert!(settings.at(9, W, H).is_none(), "complete at the last step");
        assert!(settings.at(500, W, H).is_none());
        // Frame 7 is still short of the end: the left side is covered, and the bottom right
        // corner (the edge lags there) is not yet. At frame 8 the whole canvas is behind it.
        let late = settings.at(7, W, H).unwrap();
        assert_eq!(late.coverage(0.5, 540.0, W, H), 1.0);
        assert_eq!(late.coverage(W - 0.5, H - 0.5, W, H), 0.0);
        assert!(settings.at(8, W, H).is_none());
    }

    #[test]
    fn coverage_is_one_behind_the_edge_zero_ahead_and_a_ramp_across_it() {
        let reveal = Reveal {
            edge: 50.0,
            angle: 0.0,
            rightward: true,
        };
        assert_eq!(reveal.coverage(959.0, 100.0, W, H), 1.0);
        assert_eq!(reveal.coverage(961.0, 100.0, W, H), 0.0);
        assert_eq!(reveal.coverage(960.0, 100.0, W, H), 0.5);
        let leftward = Reveal {
            rightward: false,
            ..reveal
        };
        assert_eq!(leftward.coverage(961.0, 100.0, W, H), 1.0);
        assert_eq!(leftward.coverage(959.0, 100.0, W, H), 0.0);
    }

    #[test]
    fn a_positive_angle_leads_with_the_top_in_either_direction() {
        let right = Reveal {
            edge: 50.0,
            angle: 10.0,
            rightward: true,
        };
        assert!(right.edge_x(0.0, W, H) > right.edge_x(H, W, H));
        let left = Reveal {
            rightward: false,
            ..right
        };
        assert!(left.edge_x(0.0, W, H) < left.edge_x(H, W, H));
    }

    #[test]
    fn a_sweep_that_stops_short_keeps_its_edge_and_a_leftward_one_ends_off_the_left() {
        let half = RevealSettings {
            to: 50.0,
            ..reference()
        };
        let held = half.at(400, W, H).unwrap();
        assert_eq!(held.edge, 50.0, "the end value stays after the sweep");

        let leftward = RevealSettings {
            from: 120.0,
            to: -20.0,
            angle: 0.0,
            ..reference()
        };
        assert!(!leftward.at(0, W, H).unwrap().rightward);
        assert!(leftward.at(9, W, H).is_none(), "uncovers everything");
    }

    #[test]
    fn a_negative_offset_continues_a_sweep_that_started_before_the_clip() {
        let whole = RevealSettings {
            offset: 2.0,
            ..reference()
        };
        // The right half of a cut at local 5: its frame 0 is the whole clip's frame 5.
        let right = RevealSettings {
            offset: whole.offset - 5.0,
            ..whole
        };
        for local in 0..8 {
            assert_eq!(
                right.at(local, W, H),
                whole.at(local + 5, W, H),
                "frame {local}"
            );
        }
    }

    #[test]
    fn a_degenerate_duration_is_one_frame_and_a_steep_angle_never_claims_full_coverage_early() {
        let instant = RevealSettings {
            duration: 0.0,
            ..reference()
        };
        assert!(instant.at(0, W, H).is_some());
        assert!(instant.at(1, W, H).is_none());

        let steep = Reveal {
            edge: 100.0,
            angle: 80.0,
            rightward: true,
        };
        assert!(
            !steep.uncovers_everything(W, H),
            "the top of the edge is still on the canvas"
        );
    }
}
