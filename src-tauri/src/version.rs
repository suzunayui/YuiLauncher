//! Versions look like 2026.1107.1, where the middle part has no leading zero
//! (2026.103.1 is January 3), so each part has to be compared as a number.

use std::cmp::Ordering;

fn parts(value: &str) -> Option<Vec<u64>> {
    if value.is_empty() {
        return None;
    }
    value
        .split('.')
        .map(|part| if part.chars().all(|c| c.is_ascii_digit()) { part.parse().ok() } else { None })
        .collect()
}

pub fn is_version(value: &str) -> bool {
    parts(value).is_some()
}

/// None when either side isn't a version.
pub fn compare_versions(a: &str, b: &str) -> Option<Ordering> {
    let (left, right) = (parts(a)?, parts(b)?);
    for index in 0..left.len().max(right.len()) {
        let difference = left.get(index).unwrap_or(&0).cmp(right.get(index).unwrap_or(&0));
        if difference != Ordering::Equal {
            return Some(difference);
        }
    }
    Some(Ordering::Equal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_each_part_as_a_number() {
        assert_eq!(compare_versions("2026.1021.3", "2026.103.1"), Some(Ordering::Greater));
        assert_eq!(compare_versions("2026.1107.10", "2026.1107.9"), Some(Ordering::Greater));
        assert_eq!(compare_versions("1.0", "1.0.0"), Some(Ordering::Equal));
        assert_eq!(compare_versions("1.0.0", "1.0.1"), Some(Ordering::Less));
    }

    #[test]
    fn rejects_what_is_not_a_version() {
        for value in ["", "1.0.0-beta", "v1.0", "1..0", "2026.1103.34-mac.1", "+1.0"] {
            assert!(!is_version(value), "{value}");
            assert_eq!(compare_versions(value, "1.0"), None, "{value}");
        }
        assert!(is_version("2026.1107.1"));
    }
}
