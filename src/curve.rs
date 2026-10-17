//! Duty-curve math, ported from liquidctl's `normalize_profile` and
//! `interpolate_profile`, without allocation.
//!
//! The Kraken firmware does not take a list of points. It takes a table with
//! one duty value per whole degree of liquid temperature, from 20 °C to 59 °C.
//! A user curve is a short list of `(temperature, duty)` points, so it is first
//! normalized, then sampled at each of those 40 temperatures.

/// First temperature of the firmware table, in °C.
pub const FIRST_TEMP: u8 = 20;
/// Last temperature of the firmware table, in °C. The curve is forced to 100%
/// here as a failsafe, as liquidctl does.
pub const CRITICAL_TEMP: u8 = 59;
/// Number of entries in the firmware table.
pub const TABLE_LEN: usize = (CRITICAL_TEMP - FIRST_TEMP + 1) as usize;
/// Most points a user curve may have.
pub const MAX_POINTS: usize = 32;

/// One point of a curve: liquid temperature in °C, duty in %.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Point {
    pub temp: u8,
    pub duty: u8,
}

impl Point {
    pub const fn new(temp: u8, duty: u8) -> Self {
        Point { temp, duty }
    }
}

/// A curve sorted by temperature, with strictly increasing temperatures and
/// non-decreasing duties, that ends at the first point reaching the maximum.
#[derive(Clone, Copy, Debug)]
pub struct Normalized {
    pts: [Point; MAX_POINTS + 1],
    len: usize,
}

impl Normalized {
    pub fn points(&self) -> &[Point] {
        &self.pts[..self.len]
    }

    fn last(&self) -> Option<Point> {
        self.len.checked_sub(1).map(|i| self.pts[i])
    }

    fn push(&mut self, p: Point) {
        self.pts[self.len] = p;
        self.len += 1;
    }
}

/// Normalize a curve the way liquidctl does, with `(crit_temp, 100)` added as
/// a failsafe.
///
/// One deliberate difference: liquidctl compares each point against the
/// previous *input* point when enforcing monotonic duty, which can leave a
/// decreasing curve for some inputs. Here each point is compared against the
/// last *kept* point, so the result is always monotonic. For any curve that is
/// already non-decreasing, both give the same result.
pub fn normalize(input: &[Point], crit_temp: u8) -> Normalized {
    assert!(input.len() <= MAX_POINTS, "too many curve points");

    let n = input.len() + 1;
    let mut sorted = [Point::default(); MAX_POINTS + 1];
    sorted[..input.len()].copy_from_slice(input);
    sorted[input.len()] = Point::new(crit_temp, 100);

    // Insertion sort by temperature ascending, then duty descending, so the
    // highest duty wins when a temperature appears twice.
    for i in 1..n {
        let mut j = i;
        while j > 0 && sorts_before(sorted[j], sorted[j - 1]) {
            sorted.swap(j, j - 1);
            j -= 1;
        }
    }

    let mut out = Normalized { pts: [Point::default(); MAX_POINTS + 1], len: 0 };
    for &p in &sorted[..n] {
        let kept = match out.last() {
            None => p,
            Some(last) if p.temp == last.temp => continue,
            Some(last) => Point::new(p.temp, p.duty.max(last.duty)),
        };
        out.push(kept);
        if kept.duty >= 100 {
            break;
        }
    }
    out
}

fn sorts_before(a: Point, b: Point) -> bool {
    a.temp < b.temp || (a.temp == b.temp && a.duty > b.duty)
}

/// Duty at temperature `x` on a normalized curve, linearly interpolated and
/// rounded half to even like Python's `round()`. Below the first point the
/// first duty applies, above the last point the last duty applies.
pub fn interpolate(curve: &[Point], x: u8) -> u8 {
    let mut lower = curve[0];
    let mut upper = curve[curve.len() - 1];
    for &p in curve {
        if p.temp <= x {
            lower = p;
        }
        if p.temp >= x {
            upper = p;
            break;
        }
    }
    if lower.temp == upper.temp {
        return lower.duty;
    }
    // Normalized curves never decrease, so every term here is non-negative.
    let num = (x - lower.temp) as u32 * (upper.duty - lower.duty) as u32;
    let den = (upper.temp - lower.temp) as u32;
    let (q, r) = (num / den, num % den);
    let base = lower.duty as u32 + q;
    let rounded = match (2 * r).cmp(&den) {
        core::cmp::Ordering::Less => base,
        core::cmp::Ordering::Greater => base + 1,
        core::cmp::Ordering::Equal => base + (base & 1),
    };
    rounded as u8
}

/// The 40-entry firmware table for a curve, with every duty clamped to
/// `[min, max]`.
pub fn table(input: &[Point], min: u8, max: u8) -> [u8; TABLE_LEN] {
    let norm = normalize(input, CRITICAL_TEMP);
    let mut out = [0; TABLE_LEN];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = interpolate(norm.points(), FIRST_TEMP + i as u8).clamp(min, max);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pts(v: &[(u8, u8)]) -> Vec<Point> {
        v.iter().map(|&(t, d)| Point::new(t, d)).collect()
    }

    fn norm(v: &[(u8, u8)], crit: u8) -> Vec<(u8, u8)> {
        normalize(&pts(v), crit).points().iter().map(|p| (p.temp, p.duty)).collect()
    }

    // The doctests of liquidctl's normalize_profile.
    #[test]
    fn normalize_matches_liquidctl() {
        assert_eq!(
            norm(&[(30, 40), (25, 25), (35, 30), (40, 35), (40, 80)], 60),
            [(25, 25), (30, 40), (35, 40), (40, 80), (60, 100)]
        );
        assert_eq!(
            norm(&[(30, 40), (25, 25), (35, 30), (40, 100)], 60),
            [(25, 25), (30, 40), (35, 40), (40, 100)]
        );
        assert_eq!(
            norm(&[(30, 40), (25, 25), (35, 100), (40, 100)], 60),
            [(25, 25), (30, 40), (35, 100)]
        );
        assert_eq!(norm(&[], 60), [(60, 100)]);
    }

    #[test]
    fn normalize_is_always_monotonic() {
        assert_eq!(norm(&[(25, 50), (30, 40), (35, 30)], 59), [(25, 50), (30, 50), (35, 50), (59, 100)]);
    }

    // The doctests of liquidctl's interpolate_profile.
    #[test]
    fn interpolate_matches_liquidctl() {
        assert_eq!(interpolate(&pts(&[(20, 50), (50, 70), (60, 100)]), 33), 59);
        assert_eq!(interpolate(&pts(&[(20, 50), (50, 70)]), 19), 50);
        assert_eq!(interpolate(&pts(&[(20, 50), (50, 70)]), 51), 70);
        assert_eq!(interpolate(&pts(&[(20, 50)]), 20), 50);
    }

    #[test]
    fn interpolate_rounds_half_to_even() {
        // 21 + 0.5 rounds to 22, 22 + 0.5 rounds to 22, as Python's round().
        assert_eq!(interpolate(&pts(&[(20, 21), (22, 22)]), 21), 22);
        assert_eq!(interpolate(&pts(&[(20, 22), (22, 23)]), 21), 22);
    }

    #[test]
    fn table_clamps_and_fills_forty_degrees() {
        let t = table(&pts(&[(27, 10), (28, 50), (29, 75), (30, 100)]), 20, 100);
        assert_eq!(t.len(), 40);
        assert!(t[..8].iter().all(|&d| d == 20)); // 20..=27 °C, clamped up from 10
        assert_eq!(&t[8..10], &[50, 75]);
        assert!(t[10..].iter().all(|&d| d == 100));
    }
}
