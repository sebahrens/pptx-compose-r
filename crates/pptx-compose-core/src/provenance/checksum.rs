use super::cpj;

/// Returns the spec 046 checksum for a single OPC part's raw stored payload.
///
/// Callers must pass the exact uncompressed payload bytes from the OPC part
/// store, or for dirty parts the exact bytes the writer will emit. Do not pass
/// re-serialized XML, compressed ZIP bytes, ZIP metadata, or normalized content.
#[must_use]
pub fn part_checksum(raw_bytes: &[u8]) -> String {
    cpj::digest_prefixed(raw_bytes)
}

#[cfg(test)]
#[test]
fn raw_bytes() {
    let raw = b"raw part bytes\n";
    let with_trailing_space = b"raw part bytes\n ";

    assert_eq!(
        part_checksum(raw),
        "sha256:a132365b83fa7fa4baa339520bc75db1656be2c9b593bd14c6bfba18f7f6a534"
    );
    assert_eq!(
        part_checksum(with_trailing_space),
        "sha256:48f32ee51eba2ac6682673ef1dae42b4ea60e6241629386026ac403a1bf3dfe1"
    );
    assert_ne!(part_checksum(raw), part_checksum(with_trailing_space));
}
