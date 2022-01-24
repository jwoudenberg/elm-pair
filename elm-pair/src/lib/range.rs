use std::ops::Range;

pub fn contains_range<U: std::cmp::PartialOrd>(
    outer: &Range<U>,
    inner: &Range<U>,
) -> bool {
    outer.start <= inner.start && outer.end >= inner.end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_range_includes_smaller_range() {
        assert!(contains_range(&(0..10), &(2..3)));
    }

    #[test]
    fn range_includes_itself() {
        assert!(contains_range(&(0..10), &(0..10)));
    }

    #[test]
    fn range_does_not_include_overlapping_range() {
        assert!(!contains_range(&(0..10), &(5..15)));
    }

    #[test]
    fn range_does_not_include_larger_range() {
        assert!(!contains_range(&(5..10), &(0..15)));
    }

    #[test]
    fn range_does_not_include_non_overlapping_range() {
        assert!(!contains_range(&(0..10), &(20..30)));
    }
}
