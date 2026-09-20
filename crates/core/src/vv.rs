//! Version vectors: one counter per device that has changed an entry.

use std::collections::BTreeMap;

pub type VersionVector = BTreeMap<String, u64>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ordering {
    Equal,
    /// `a` has seen everything `b` has, and more.
    Dominates,
    /// `b` has seen everything `a` has, and more.
    Dominated,
    Concurrent,
}

/// A key missing on one side counts as zero.
pub fn compare(a: &VersionVector, b: &VersionVector) -> Ordering {
    let mut a_ahead = false;
    let mut b_ahead = false;
    for key in a.keys().chain(b.keys()) {
        let x = a.get(key).copied().unwrap_or(0);
        let y = b.get(key).copied().unwrap_or(0);
        if x > y {
            a_ahead = true;
        } else if y > x {
            b_ahead = true;
        }
    }
    match (a_ahead, b_ahead) {
        (false, false) => Ordering::Equal,
        (true, false) => Ordering::Dominates,
        (false, true) => Ordering::Dominated,
        (true, true) => Ordering::Concurrent,
    }
}

/// Pointwise maximum.
pub fn merge(a: &VersionVector, b: &VersionVector) -> VersionVector {
    let mut out = a.clone();
    for (k, v) in b {
        let slot = out.entry(k.clone()).or_insert(0);
        if *v > *slot {
            *slot = *v;
        }
    }
    out
}

/// Sets `vv[device]` to one more than the largest counter anywhere in the
/// vector, so a device that was behind jumps ahead of everything it has seen.
pub fn bump(vv: &mut VersionVector, device: &str) {
    let max = vv.values().copied().max().unwrap_or(0);
    vv.insert(device.to_string(), max + 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vv(pairs: &[(&str, u64)]) -> VersionVector {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn compare_covers_all_four_outcomes() {
        assert_eq!(compare(&vv(&[("a", 1)]), &vv(&[("a", 1)])), Ordering::Equal);
        assert_eq!(compare(&vv(&[]), &vv(&[])), Ordering::Equal);
        assert_eq!(
            compare(&vv(&[("a", 2)]), &vv(&[("a", 1)])),
            Ordering::Dominates
        );
        assert_eq!(
            compare(&vv(&[("a", 1)]), &vv(&[("a", 1), ("b", 1)])),
            Ordering::Dominated
        );
        assert_eq!(
            compare(&vv(&[("a", 2)]), &vv(&[("b", 1)])),
            Ordering::Concurrent
        );
    }

    #[test]
    fn missing_key_counts_as_zero() {
        assert_eq!(compare(&vv(&[("a", 0)]), &vv(&[])), Ordering::Equal);
        assert_eq!(compare(&vv(&[("a", 1)]), &vv(&[])), Ordering::Dominates);
    }

    #[test]
    fn merge_is_pointwise_max() {
        let m = merge(&vv(&[("a", 2), ("b", 1)]), &vv(&[("b", 3), ("c", 1)]));
        assert_eq!(m, vv(&[("a", 2), ("b", 3), ("c", 1)]));
    }

    #[test]
    fn bump_uses_max_plus_one() {
        let mut v = vv(&[("a", 1), ("b", 5)]);
        bump(&mut v, "a");
        assert_eq!(v, vv(&[("a", 6), ("b", 5)]));
        let mut empty = vv(&[]);
        bump(&mut empty, "a");
        assert_eq!(empty, vv(&[("a", 1)]));
    }
}
