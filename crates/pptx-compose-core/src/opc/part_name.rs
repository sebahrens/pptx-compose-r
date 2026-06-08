use std::{
    cmp::Ordering,
    fmt,
    hash::{Hash, Hasher},
};

use crate::error::{Error, Result};

#[derive(Clone, Debug, Eq)]
pub struct PartName {
    package_name: String,
    zip_entry_name: String,
}

impl PartName {
    pub fn from_zip_entry_name(raw: &str) -> Result<Self> {
        let slash_normalized = raw.replace('\\', "/");
        let decoded = percent_decode_once(&slash_normalized)?;

        if decoded.starts_with("//") {
            return Err(Error::unsafe_path("Part name must not be a UNC path."));
        }

        let canonical = if decoded.starts_with('/') {
            decoded
        } else {
            format!("/{decoded}")
        };

        validate_canonical(&canonical)?;

        let zip_entry_name = canonical
            .strip_prefix('/')
            .expect("canonical part name has one leading slash")
            .to_owned();

        Ok(Self {
            package_name: canonical,
            zip_entry_name,
        })
    }

    pub fn from_zip_entry(zip_entry_name: &str) -> Result<Self> {
        Self::from_zip_entry_name(zip_entry_name)
    }

    #[must_use]
    pub fn as_package_name(&self) -> &str {
        &self.package_name
    }

    #[must_use]
    pub fn as_zip_entry_name(&self) -> &str {
        &self.zip_entry_name
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        self.as_package_name()
    }

    #[must_use]
    pub fn zip_entry_name(&self) -> &str {
        self.as_zip_entry_name()
    }
}

impl fmt::Display for PartName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_package_name())
    }
}

impl Hash for PartName {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.package_name.hash(state);
    }
}

impl Ord for PartName {
    fn cmp(&self, other: &Self) -> Ordering {
        self.package_name.cmp(&other.package_name)
    }
}

impl PartialEq for PartName {
    fn eq(&self, other: &Self) -> bool {
        self.package_name == other.package_name
    }
}

impl PartialOrd for PartName {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub fn reject_unsafe_entry(raw_name: &str) -> Result<()> {
    if raw_name.starts_with('/') || raw_name.starts_with('\\') {
        return Err(Error::unsafe_path(
            "ZIP entry name must not be an absolute path.",
        ));
    }

    let slash_normalized = raw_name.replace('\\', "/");
    let decoded = percent_decode_once(&slash_normalized)?;

    if decoded.starts_with("//") {
        return Err(Error::unsafe_path("ZIP entry name must not be a UNC path."));
    }

    for (index, segment) in decoded.split('/').enumerate() {
        if segment == "." || segment == ".." {
            return Err(Error::unsafe_path(
                "ZIP entry name must not contain dot or dot-dot traversal segments.",
            ));
        }

        if index == 0 && is_drive_segment(segment) {
            return Err(Error::unsafe_path(
                "ZIP entry name must not be a drive path.",
            ));
        }
    }

    Ok(())
}

fn validate_canonical(canonical: &str) -> Result<()> {
    if !canonical.starts_with('/') || canonical.starts_with("//") || canonical.ends_with('/') {
        return Err(Error::unsafe_path(
            "Part name must have one leading slash and must not name a directory.",
        ));
    }

    let mut segments = canonical.split('/');
    if segments.next() != Some("") {
        return Err(Error::unsafe_path(
            "Part name must have exactly one leading slash.",
        ));
    }

    for (index, segment) in segments.enumerate() {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(Error::unsafe_path(
                "Part name must not contain empty, dot, or dot-dot segments.",
            ));
        }

        if index == 0 && is_drive_segment(segment) {
            return Err(Error::unsafe_path("Part name must not be a drive path."));
        }
    }

    Ok(())
}

fn is_drive_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic()
}

fn percent_decode_once(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = bytes.get(index + 1).copied();
            let low = bytes.get(index + 2).copied();
            let Some(high) = high.and_then(hex_value) else {
                return Err(Error::unsafe_path(
                    "Part name contains an invalid percent escape.",
                ));
            };
            let Some(low) = low.and_then(hex_value) else {
                return Err(Error::unsafe_path(
                    "Part name contains an invalid percent escape.",
                ));
            };
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }

    String::from_utf8(decoded)
        .map_err(|_| Error::unsafe_path("Part name percent decoding produced invalid UTF-8."))
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
#[test]
fn rejects_traversal_and_preserves_case_sensitive_segments() {
    use crate::{error::ErrorCode, opc::part::PartStore};

    for entry_name in [
        "../evil.xml",
        "/ppt/../evil.xml",
        "a//b",
        "ppt/slides/",
        r"C:\x",
        r"\\server\share\slide1.xml",
        "ppt/%2e%2e/evil.xml",
        "ppt/%zz.xml",
    ] {
        let error = PartName::from_zip_entry_name(entry_name).expect_err("unsafe path rejected");
        assert_eq!(error.code(), ErrorCode::UnsafePath);
    }

    let upper = PartName::from_zip_entry_name("/ppt/Slides/x.xml").expect("valid part");
    let lower = PartName::from_zip_entry_name("/ppt/slides/x.xml").expect("valid part");

    assert_ne!(upper, lower);
    assert_eq!(upper.as_package_name(), "/ppt/Slides/x.xml");
    assert_eq!(upper.as_zip_entry_name(), "ppt/Slides/x.xml");
    assert_eq!(lower.as_package_name(), "/ppt/slides/x.xml");
    assert_eq!(lower.as_zip_entry_name(), "ppt/slides/x.xml");

    let mut store = PartStore::new();
    store
        .insert_zip_entry("/ppt/Slides/x.xml", Vec::new())
        .expect("first insert");
    store
        .insert_zip_entry("/ppt/slides/x.xml", Vec::new())
        .expect("second insert");

    assert!(store.get(&upper).is_some());
    assert!(store.get(&lower).is_some());
}

#[cfg(test)]
mod tests {
    use crate::{
        error::ErrorCode,
        opc::{part::PartStore, part_name::PartName},
    };

    #[test]
    fn accepts_absolute_part_name() {
        let part_name =
            PartName::from_zip_entry_name("/ppt/slides/slide1.xml").expect("valid part");

        assert_eq!(part_name.as_package_name(), "/ppt/slides/slide1.xml");
        assert_eq!(part_name.as_zip_entry_name(), "ppt/slides/slide1.xml");
    }

    #[test]
    fn rejects_duplicate_normalized_part_names() {
        let mut store = PartStore::new();
        store
            .insert_zip_entry("ppt/slides/slide1.xml", Vec::new())
            .expect("first insert");

        let error = store
            .insert_zip_entry("/ppt/slides/slide1.xml", Vec::new())
            .expect_err("duplicate rejected");

        assert_eq!(error.code(), ErrorCode::UnsupportedPackage);
    }

    #[test]
    fn percent_decodes_once_before_validation() {
        let part_name =
            PartName::from_zip_entry_name("ppt%2Fslides%2Fslide1.xml").expect("valid part");
        assert_eq!(part_name.as_package_name(), "/ppt/slides/slide1.xml");
        assert_eq!(part_name.as_zip_entry_name(), "ppt/slides/slide1.xml");

        let error = PartName::from_zip_entry_name("ppt/%zz.xml").expect_err("bad escape rejected");
        assert_eq!(error.code(), ErrorCode::UnsafePath);
    }
}
