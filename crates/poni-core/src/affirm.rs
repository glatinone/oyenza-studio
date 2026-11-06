//! Affirmations on the resting island: a short line every few minutes
//! (`Settings::affirm_every_min`). The list is `affirmations.txt`, a few KB of
//! text built into the app; no network, no file read at run time.
//!
//! Which line shows is worked out from the clock, so nothing is stored: the
//! day is cut into slots of `every` minutes and slot `n` shows line
//! `(n * STEP + offset) % len`. STEP is a prime that does not divide the
//! list's length, so every line shows once before any repeats, in an order
//! that does not follow the file.

/// `affirmations.txt`: `title|second line` per line, `#` lines are comments.
const LIST: &str = include_str!("affirmations.txt");

const STEP: u64 = 7919;

/// Every affirmation as (title, second line), in file order.
pub fn all() -> impl Iterator<Item = (&'static str, &'static str)> {
    LIST.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(|l| match l.split_once('|') {
        Some((t, s)) => (t.trim(), s.trim()),
        None => (l, ""),
    })
}

/// The slot `unix` falls in for one line every `every_min` minutes
/// (`None` when they are off).
pub fn slot(unix: i64, every_min: u32) -> Option<i64> {
    (every_min > 0).then(|| unix.div_euclid(every_min as i64 * 60))
}

/// The affirmation for `slot`.
pub fn pick(slot: i64) -> (&'static str, &'static str) {
    let n = all().count() as u64;
    if n == 0 {
        return ("", "");
    }
    let step = if n % STEP == 0 { STEP + 2 } else { STEP };
    let i = (slot as u64).wrapping_mul(step).wrapping_add(n / 3) % n;
    all().nth(i as usize).unwrap_or(("", ""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_parses_and_fits_a_notice() {
        let v: Vec<_> = all().collect();
        assert!(v.len() > 100);
        for (t, s) in &v {
            assert!(!t.is_empty());
            assert!(t.chars().count() <= 42, "title too long: {t}");
            assert!(s.chars().count() <= 48, "second line too long: {s}");
        }
    }

    #[test]
    fn every_line_shows_before_one_repeats() {
        let n = all().count();
        let mut seen: Vec<_> = (0..n as i64).map(pick).collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), n);
    }

    #[test]
    fn slots() {
        assert_eq!(slot(0, 0), None);
        assert_eq!(slot(299, 5), Some(0));
        assert_eq!(slot(300, 5), Some(1));
    }
}
