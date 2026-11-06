//! Affirmations on the resting island: a short line every few minutes
//! (`Settings::affirm_every_min`). The list is `%APPDATA%\Poni\affirmations.txt`,
//! read when a line is due (a few KB once per slot), so editing it takes effect
//! at the next slot with no rebuild. The copy built into the app
//! (`affirmations.txt` here) seeds that file when it is missing and is the
//! fallback when it cannot be read or holds no lines. No network.
//!
//! Which line shows is worked out from the clock, so nothing is stored: the
//! day is cut into slots of `every` minutes and slot `n` shows line
//! `(n * STEP + offset) % len`. STEP is a prime that does not divide the
//! list's length, so every line shows once before any repeats, in an order
//! that does not follow the file.

/// `affirmations.txt`: `title|second line` per line, `#` lines are comments.
const LIST: &str = include_str!("affirmations.txt");

const STEP: u64 = 7919;

/// Every affirmation in `text` as (title, second line), in file order.
fn parse(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(|l| match l.split_once('|') {
        Some((t, s)) => (t.trim(), s.trim()),
        None => (l, ""),
    })
}

/// The built-in list, in file order.
pub fn all() -> impl Iterator<Item = (&'static str, &'static str)> {
    parse(LIST)
}

/// The user's list file (Windows: `%APPDATA%\Poni\affirmations.txt`), seeded
/// from the built-in list the first time; the built-in list when it is
/// missing, unreadable or empty.
fn current() -> String {
    let Some(dir) = std::env::var_os("APPDATA").map(|d| std::path::PathBuf::from(d).join("Poni")) else {
        return LIST.to_string();
    };
    let path = dir.join("affirmations.txt");
    match std::fs::read_to_string(&path) {
        Ok(t) if parse(&t).next().is_some() => t,
        Ok(_) => LIST.to_string(),
        Err(_) => {
            let _ = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, LIST));
            LIST.to_string()
        }
    }
}

/// The line for `slot` out of `text`.
fn pick_in(text: &str, slot: i64) -> (String, String) {
    let n = parse(text).count() as u64;
    if n == 0 {
        return (String::new(), String::new());
    }
    let step = if n % STEP == 0 { STEP + 2 } else { STEP };
    let i = (slot as u64).wrapping_mul(step).wrapping_add(n / 3) % n;
    parse(text).nth(i as usize).map(|(t, s)| (t.to_string(), s.to_string())).unwrap_or_default()
}

/// The slot `unix` falls in for one line every `every_min` minutes
/// (`None` when they are off).
pub fn slot(unix: i64, every_min: u32) -> Option<i64> {
    (every_min > 0).then(|| unix.div_euclid(every_min as i64 * 60))
}

/// The affirmation for `slot`, from the user's list file.
pub fn pick(slot: i64) -> (String, String) {
    pick_in(&current(), slot)
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
        let mut seen: Vec<_> = (0..n as i64).map(|s| pick_in(LIST, s)).collect();
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
