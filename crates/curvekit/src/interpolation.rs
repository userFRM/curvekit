//! Yield curve interpolation methods.
//!
//! All methods operate on `BTreeMap<u32, f64>` (days → continuously-compounded
//! rate) and return `Option<f64>` — `None` only when the map is empty.

use std::collections::BTreeMap;

/// Linear interpolation (or flat extrapolation at curve boundaries).
///
/// - Exact match → return the stored value.
/// - Within bounds → linearly interpolate between bracketing points.
/// - Below minimum → return the lowest available rate (flat extrapolation).
/// - Above maximum → return the highest available rate (flat extrapolation).
/// - Empty map → `None`.
pub fn linear(points: &BTreeMap<u32, f64>, target: u32) -> Option<f64> {
    if points.is_empty() {
        return None;
    }

    // Exact match fast path.
    if let Some(&r) = points.get(&target) {
        return Some(r);
    }

    // Find bracketing points using BTreeMap range capabilities.
    let lower = points.range(..target).next_back();
    let upper = points.range(target..).next();

    match (lower, upper) {
        (Some((&dl, &rl)), Some((&du, &ru))) => {
            // Linear interpolation.
            let t = (target - dl) as f64 / (du - dl) as f64;
            Some(rl + t * (ru - rl))
        }
        // Flat extrapolation at boundaries.
        (Some((_, &rl)), None) => Some(rl),
        (None, Some((_, &ru))) => Some(ru),
        (None, None) => None,
    }
}

/// Monotone cubic spline (Fritsch-Carlson) interpolation.
///
/// Produces a smooth curve that honours all data points and avoids overshooting.
/// Falls back to `linear` when fewer than 3 points are available.
pub fn cubic_spline(points: &BTreeMap<u32, f64>, target: u32) -> Option<f64> {
    if points.len() < 3 {
        return linear(points, target);
    }

    let xs: Vec<f64> = points.keys().map(|&d| d as f64).collect();
    let ys: Vec<f64> = points.values().copied().collect();
    let n = xs.len();

    // Exact match fast path.
    if let Some(&r) = points.get(&target) {
        return Some(r);
    }

    let t = target as f64;

    // Clamp to boundary (flat extrapolation beyond the curve ends).
    if t < xs[0] {
        return Some(ys[0]);
    }
    if t > xs[n - 1] {
        return Some(ys[n - 1]);
    }

    // Compute secant slopes.
    let mut delta = vec![0.0_f64; n - 1];
    let mut h = vec![0.0_f64; n - 1];
    for i in 0..n - 1 {
        h[i] = xs[i + 1] - xs[i];
        delta[i] = (ys[i + 1] - ys[i]) / h[i];
    }

    // Fritsch-Carlson tangents.
    let mut m = vec![0.0_f64; n];
    m[0] = delta[0];
    m[n - 1] = delta[n - 2];
    for i in 1..n - 1 {
        if delta[i - 1] * delta[i] <= 0.0 {
            m[i] = 0.0;
        } else {
            let w1 = 2.0 * h[i] + h[i - 1];
            let w2 = h[i] + 2.0 * h[i - 1];
            m[i] = (w1 + w2) / (w1 / delta[i - 1] + w2 / delta[i]);
        }
    }

    // Find the segment containing target.
    let seg = xs
        .windows(2)
        .position(|w| t >= w[0] && t <= w[1])
        .unwrap_or(n - 2);

    // Evaluate Hermite cubic on the segment.
    let dx = t - xs[seg];
    let h_seg = h[seg];
    let t1 = dx / h_seg;
    let t2 = t1 * t1;
    let t3 = t2 * t1;

    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + t1;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h11 = t3 - t2;

    Some(h00 * ys[seg] + h10 * h_seg * m[seg] + h01 * ys[seg + 1] + h11 * h_seg * m[seg + 1])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_points(pairs: &[(u32, f64)]) -> BTreeMap<u32, f64> {
        pairs.iter().copied().collect()
    }

    #[test]
    fn linear_exact_match() {
        let pts = make_points(&[(30, 0.04), (365, 0.05)]);
        assert!((linear(&pts, 30).unwrap() - 0.04).abs() < 1e-12);
        assert!((linear(&pts, 365).unwrap() - 0.05).abs() < 1e-12);
    }

    #[test]
    fn linear_midpoint() {
        let pts = make_points(&[(0, 0.0), (100, 1.0)]);
        assert!((linear(&pts, 50).unwrap() - 0.5).abs() < 1e-12);
    }

    #[test]
    fn linear_extrapolate_below() {
        let pts = make_points(&[(30, 0.04), (365, 0.05)]);
        // Below minimum → flat extrapolation at 0.04
        assert!((linear(&pts, 1).unwrap() - 0.04).abs() < 1e-12);
    }

    #[test]
    fn linear_extrapolate_above() {
        let pts = make_points(&[(30, 0.04), (365, 0.05)]);
        // Above maximum → flat extrapolation at 0.05
        assert!((linear(&pts, 10000).unwrap() - 0.05).abs() < 1e-12);
    }

    #[test]
    fn linear_empty_returns_none() {
        let pts: BTreeMap<u32, f64> = BTreeMap::new();
        assert!(linear(&pts, 30).is_none());
    }

    #[test]
    fn cubic_spline_exact_match() {
        let pts = make_points(&[(30, 0.04), (180, 0.045), (365, 0.05)]);
        assert!((cubic_spline(&pts, 30).unwrap() - 0.04).abs() < 1e-10);
    }

    #[test]
    fn cubic_spline_monotone_on_increasing_curve() {
        let pts = make_points(&[(30, 0.04), (90, 0.042), (180, 0.045), (365, 0.05)]);
        let v60 = cubic_spline(&pts, 60).unwrap();
        let v90 = cubic_spline(&pts, 90).unwrap();
        // Monotone: v60 should be between 0.04 and 0.042
        assert!((0.04..=0.042 + 1e-10).contains(&v60), "v60={v60}");
        // Exact match
        assert!((v90 - 0.042).abs() < 1e-10, "v90={v90}");
    }

    #[test]
    fn cubic_falls_back_to_linear_for_two_points() {
        let pts = make_points(&[(30, 0.04), (60, 0.05)]);
        // With only 2 points, cubic_spline falls back to linear
        let cubic = cubic_spline(&pts, 45).unwrap();
        let lin = linear(&pts, 45).unwrap();
        assert!((cubic - lin).abs() < 1e-12);
    }
}
