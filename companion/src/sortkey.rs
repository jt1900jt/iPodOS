//! Sort keys and identity folding. Ordering rules are specified in docs/ipdb-format.md.

/// Bucket 0..=25 is A..Z, 26 is '#'.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SortKey {
    pub bucket: u8,
    pub key: String,
}

pub fn sort_key(s: &str) -> SortKey {
    let folded = deunicode::deunicode(s).to_lowercase();
    let t = folded.trim();
    let mut rest = t;
    for art in ["the ", "a ", "an "] {
        if let Some(r) = t.strip_prefix(art) {
            if !r.trim().is_empty() {
                rest = r;
            }
            break;
        }
    }
    let key = rest.trim_start_matches(|c: char| !c.is_ascii_alphanumeric());
    let key = if key.is_empty() { t } else { key };
    let bucket = match key.as_bytes().first() {
        Some(&c) if c.is_ascii_lowercase() => c - b'a',
        _ => 26,
    };
    SortKey { bucket, key: key.to_string() }
}

/// Identity for merging names: case, accents and whitespace runs are ignored.
pub fn fold(s: &str) -> String {
    let lowered = deunicode::deunicode(s).to_lowercase();
    lowered.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn articles_and_buckets() {
        assert_eq!(sort_key("The Quiet Hours").key, "quiet hours");
        assert_eq!(sort_key("The").key, "the");
        assert_eq!(sort_key("A Tribe").bucket, b't' - b'a');
        assert_eq!(sort_key("Ångström").bucket, 0);
        assert_eq!(sort_key("10 Years").bucket, 26);
        assert_eq!(sort_key("...And Justice").key, "and justice");
        assert_eq!(sort_key("").bucket, 26);
        assert!(sort_key("Zed") < sort_key("1999"));
    }

    #[test]
    fn folding() {
        assert_eq!(fold("Björk "), fold("bjork"));
        assert_eq!(fold("Daft  Punk"), fold("daft punk"));
    }
}
