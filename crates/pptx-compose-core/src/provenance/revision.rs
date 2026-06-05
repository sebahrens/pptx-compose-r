/// Monotonic in-memory guard for an open document session.
///
/// A revision is meaningful only when paired with the current `document_id`.
/// It is never persisted into package bytes and is not derived from durable
/// package identity. The edit crate owns deciding whether an apply actually
/// wrote a part and passing that outcome to [`Revision::record_apply`].
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Revision(u64);

impl Revision {
    /// Returns the numeric session revision value.
    #[must_use]
    pub fn value(self) -> u64 {
        self.0
    }

    /// Records the outcome of a non-dry-run apply attempt.
    ///
    /// The revision advances by exactly one only when the apply succeeded and
    /// wrote at least one part. Dry-runs, failed applies, and no-op applies keep
    /// the current value.
    pub fn record_apply(&mut self, wrote_part: bool) -> u64 {
        if wrote_part {
            self.0 += 1;
        }

        self.0
    }
}

/// Creates the revision state for a fresh open/parse.
#[must_use]
pub fn on_open() -> Revision {
    Revision(1)
}

#[cfg(test)]
#[test]
fn lifecycle() {
    let mut revision = on_open();

    assert_eq!(revision.value(), 1);
    assert_eq!(revision.record_apply(true), 2);
    assert_eq!(revision.value(), 2);
    assert_eq!(revision.record_apply(false), 2);
    assert_eq!(revision.value(), 2);
}
