//! Small text helpers for diagnostics.

/// Levenshtein edit distance (integer DP).
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// The candidate closest to `word` within `max(2, len/3)` edits, ties broken
/// by candidate order.
pub fn nearest<'a>(word: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let limit = (word.chars().count() / 3).max(2);
    let mut best: Option<(usize, &str)> = None;
    for c in candidates {
        let d = levenshtein(word, c);
        if d <= limit && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, c));
        }
    }
    best.map(|(_, c)| c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances_and_suggestions() {
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("armour", "amour"), 1);
        assert_eq!(
            nearest("amour", ["attack", "armour", "damage"]),
            Some("armour")
        );
        assert_eq!(
            nearest("war_cri", ["mymod:war_cry"]),
            None,
            "namespace makes it too far"
        );
        assert_eq!(
            nearest("mymod:war_cri", ["mymod:war_cry", "mymod:war_cry2"]),
            Some("mymod:war_cry")
        );
        assert_eq!(nearest("zzzzzz", ["attack"]), None);
    }
}
