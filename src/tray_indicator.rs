use std::collections::VecDeque;
use std::time::{Duration, Instant};

const ICON_SIZE: i32 = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressureColor {
    Green,
    Yellow,
    Orange,
    Red,
}

impl PressureColor {
    pub fn from_cpu(avg_percent: f32) -> Self {
        if avg_percent >= 80.0 {
            Self::Red
        } else if avg_percent >= 65.0 {
            Self::Orange
        } else if avg_percent >= 50.0 {
            Self::Yellow
        } else {
            Self::Green
        }
    }

    fn rgb(self) -> (u8, u8, u8) {
        match self {
            Self::Green => (76, 175, 80),
            Self::Yellow => (255, 193, 7),
            Self::Orange => (255, 152, 0),
            Self::Red => (244, 67, 54),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Green => "low",
            Self::Yellow => "moderate",
            Self::Orange => "high",
            Self::Red => "critical",
        }
    }
}

struct Sample {
    at: Instant,
    cpu: f32,
}

pub struct PressureTracker {
    window: Duration,
    samples: VecDeque<Sample>,
}

impl PressureTracker {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            samples: VecDeque::new(),
        }
    }

    pub fn record(&mut self, cpu: f32) {
        let now = Instant::now();
        self.samples.push_back(Sample { at: now, cpu });
        self.prune(now);
    }

    pub fn average(&mut self) -> f32 {
        let now = Instant::now();
        self.prune(now);
        if self.samples.is_empty() {
            return 0.0;
        }
        let total: f32 = self.samples.iter().map(|sample| sample.cpu).sum();
        total / self.samples.len() as f32
    }

    fn prune(&mut self, now: Instant) {
        while let Some(sample) = self.samples.front() {
            if now.duration_since(sample.at) > self.window {
                self.samples.pop_front();
            } else {
                break;
            }
        }
    }
}

const EDGE_SOFTNESS: f32 = 1.0;
const W_STROKE_RADIUS: f32 = 0.58;
const W_STROKE_SOFTNESS: f32 = 0.65;
/// Letter width in local glyph coordinates (see `w_letter_coverage`).
const W_LOCAL_WIDTH: f32 = 5.6;
/// Fraction of the circle diameter occupied by the letter.
const W_DIAMETER_RATIO: f32 = 0.36;

fn circle_coverage(dx: f32, dy: f32, radius: f32) -> f32 {
    let dist = (dx * dx + dy * dy).sqrt() - radius;
    (0.5 - dist / EDGE_SOFTNESS).clamp(0.0, 1.0)
}

fn dist_to_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let abx = bx - ax;
    let aby = by - ay;
    let apx = px - ax;
    let apy = py - ay;
    let ab_len_sq = abx * abx + aby * aby;
    let t = if ab_len_sq > 0.0 {
        ((apx * abx + apy * aby) / ab_len_sq).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let closest_x = ax + t * abx;
    let closest_y = ay + t * aby;
    (px - closest_x).hypot(py - closest_y)
}

fn stroke_coverage(dist: f32) -> f32 {
    (W_STROKE_RADIUS + W_STROKE_SOFTNESS - dist).clamp(0.0, 1.0) / W_STROKE_SOFTNESS
}

fn w_segment_coverage(lx: f32, ly: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    stroke_coverage(dist_to_segment(lx, ly, ax, ay, bx, by))
}

/// Compact lowercase "w" in local coordinates (origin = glyph center).
fn w_letter_coverage(lx: f32, ly: f32) -> f32 {
    let mut coverage = 0.0_f32;
    let seg = |px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32| {
        w_segment_coverage(px, py, ax, ay, bx, by)
    };

    // Four stems + three bottom joins — kept narrow so strokes do not blanket the dot.
    coverage = coverage.max(seg(lx, ly, -2.8, -2.4, -2.8, 2.1));
    coverage = coverage.max(seg(lx, ly, -0.95, -2.4, -0.95, 0.7));
    coverage = coverage.max(seg(lx, ly, 0.95, -2.4, 0.95, 0.7));
    coverage = coverage.max(seg(lx, ly, 2.8, -2.4, 2.8, 2.1));

    coverage = coverage.max(seg(lx, ly, -2.8, 2.1, -0.95, 0.7));
    coverage = coverage.max(seg(lx, ly, -0.95, 0.7, 0.0, 2.1));
    coverage = coverage.max(seg(lx, ly, 0.0, 2.1, 0.95, 0.7));
    coverage = coverage.max(seg(lx, ly, 0.95, 0.7, 2.8, 2.1));

    coverage
}

fn render_dot_pixels(color: PressureColor) -> Vec<u8> {
    let (r, g, b) = color.rgb();
    let size = ICON_SIZE;
    let center = (size as f32 - 1.0) / 2.0;
    let radius = center - 1.5;
    let glyph_scale = (radius * 2.0 * W_DIAMETER_RATIO) / W_LOCAL_WIDTH;
    let mut data = Vec::with_capacity((size * size * 4) as usize);

    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let circle = circle_coverage(dx, dy, radius);

            let lx = dx / glyph_scale;
            let ly = dy / glyph_scale;
            let glyph = w_letter_coverage(lx, ly);

            let blend = glyph.clamp(0.0, 1.0);
            let alpha = circle;
            let inv = 1.0 - blend;
            let pixel_r = (r as f32 * inv + 255.0 * blend).round() as u8;
            let pixel_g = (g as f32 * inv + 255.0 * blend).round() as u8;
            let pixel_b = (b as f32 * inv + 255.0 * blend).round() as u8;
            let pixel_a = (alpha * 255.0).round() as u8;

            data.push(pixel_a);
            data.push(pixel_r);
            data.push(pixel_g);
            data.push(pixel_b);
        }
    }

    data
}

#[cfg(target_os = "linux")]
pub fn dot_icon(color: PressureColor) -> ksni::Icon {
    ksni::Icon {
        width: ICON_SIZE,
        height: ICON_SIZE,
        data: render_dot_pixels(color),
    }
}

#[cfg(not(target_os = "linux"))]
pub fn dot_icon(_color: PressureColor) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressure_color_thresholds() {
        assert_eq!(PressureColor::from_cpu(0.0), PressureColor::Green);
        assert_eq!(PressureColor::from_cpu(49.9), PressureColor::Green);
        assert_eq!(PressureColor::from_cpu(50.0), PressureColor::Yellow);
        assert_eq!(PressureColor::from_cpu(64.9), PressureColor::Yellow);
        assert_eq!(PressureColor::from_cpu(65.0), PressureColor::Orange);
        assert_eq!(PressureColor::from_cpu(79.9), PressureColor::Orange);
        assert_eq!(PressureColor::from_cpu(80.0), PressureColor::Red);
        assert_eq!(PressureColor::from_cpu(100.0), PressureColor::Red);
    }

    #[test]
    fn tracker_averages_samples_in_window() {
        let mut tracker = PressureTracker::new(Duration::from_secs(5));
        tracker.record(40.0);
        tracker.record(60.0);
        assert!((tracker.average() - 50.0).abs() < 0.01);
    }

    #[test]
    fn dot_icon_has_expected_size_and_white_glyph_stroke() {
        let pixels = render_dot_pixels(PressureColor::Green);
        assert_eq!(pixels.len(), (ICON_SIZE * ICON_SIZE * 4) as usize);

        let center = ICON_SIZE as usize / 2;
        let mut found_white_stroke = false;
        for y in (center.saturating_sub(4))..=(center + 4).min(ICON_SIZE as usize - 1) {
            for x in (center.saturating_sub(5))..=(center + 5).min(ICON_SIZE as usize - 1) {
                let idx = (y * ICON_SIZE as usize + x) * 4;
                if pixels[idx] > 200
                    && pixels[idx + 1] == 255
                    && pixels[idx + 2] == 255
                    && pixels[idx + 3] == 255
                {
                    found_white_stroke = true;
                    break;
                }
            }
        }
        assert!(found_white_stroke, "expected white w stroke inside icon");
    }

    #[test]
    fn dot_icon_glyph_does_not_blanket_circle() {
        let pixels = render_dot_pixels(PressureColor::Green);
        let mut circle_pixels = 0_usize;
        let mut colored_pixels = 0_usize;

        for y in 0..ICON_SIZE as usize {
            for x in 0..ICON_SIZE as usize {
                let idx = (y * ICON_SIZE as usize + x) * 4;
                if pixels[idx] < 128 {
                    continue;
                }
                circle_pixels += 1;
                if pixels[idx + 1] == 76 && pixels[idx + 2] == 175 && pixels[idx + 3] == 80 {
                    colored_pixels += 1;
                }
            }
        }

        assert!(circle_pixels > 0);
        assert!(
            colored_pixels * 100 / circle_pixels > 35,
            "expected visible green background between w strokes, got {colored_pixels}/{circle_pixels}"
        );
    }

    #[test]
    fn dot_icon_corner_is_transparent() {
        let pixels = render_dot_pixels(PressureColor::Red);
        assert_eq!(pixels[0], 0, "top-left alpha");
    }

    #[test]
    fn tracker_drops_samples_outside_window() {
        let mut tracker = PressureTracker::new(Duration::from_millis(100));
        tracker.record(100.0);
        std::thread::sleep(Duration::from_millis(150));
        tracker.record(0.0);
        assert!((tracker.average() - 0.0).abs() < 0.01);
    }
}
