use std::collections::BTreeSet;

use crate::{
    opc::{part::PartStore, part_name::PartName},
    provenance::checksum::part_checksum,
};

const MEDIA_PREFIX: &str = "/ppt/media/image";

#[derive(Clone, Debug, Default)]
pub struct MediaPartNameAllocator {
    allocated: BTreeSet<PartName>,
}

impl MediaPartNameAllocator {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn next_media_part_name(&mut self, parts: &PartStore, ext: &str) -> PartName {
        let extension = ext.to_ascii_lowercase();
        let next = next_media_index(parts, self.allocated.iter(), extension.as_str()) + 1;
        let part_name = media_part_name(next, extension.as_str());
        self.allocated.insert(part_name.clone());
        part_name
    }
}

#[must_use]
pub fn next_media_part_name(parts: &PartStore, ext: &str) -> PartName {
    MediaPartNameAllocator::new().next_media_part_name(parts, ext)
}

#[must_use]
pub fn dedup_lookup(parts: &PartStore, sha256: &str, opt_in: bool) -> Option<PartName> {
    if !opt_in {
        return None;
    }

    parts
        .iter()
        .filter(|part| is_media_part(part.name()))
        .find(|part| part_checksum(part.bytes()) == sha256)
        .map(|part| part.name().clone())
}

fn next_media_index<'a>(
    parts: &PartStore,
    allocated: impl Iterator<Item = &'a PartName>,
    ext: &str,
) -> u32 {
    let existing_max = parts
        .iter()
        .map(|part| part.name())
        .filter_map(|part_name| media_index(part_name, ext))
        .max()
        .unwrap_or(0);
    let allocated_max = allocated
        .filter_map(|part_name| media_index(part_name, ext))
        .max()
        .unwrap_or(0);

    existing_max.max(allocated_max)
}

fn media_index(part_name: &PartName, ext: &str) -> Option<u32> {
    let path = part_name.as_str();
    let suffix = format!(".{ext}");
    let index = path.strip_prefix(MEDIA_PREFIX)?.strip_suffix(&suffix)?;

    if index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    index.parse().ok()
}

fn media_part_name(index: u32, ext: &str) -> PartName {
    PartName::from_zip_entry(format!("ppt/media/image{index}.{ext}").as_str())
        .expect("deterministic media part names are valid OPC part names")
}

fn is_media_part(part_name: &PartName) -> bool {
    let path = part_name.as_str();
    path.starts_with("/ppt/media/") && path.rsplit_once('.').is_some()
}

#[cfg(test)]
#[test]
fn names_deterministically_and_dedups() {
    let matching_bytes = b"same image bytes".to_vec();
    let digest = part_checksum(&matching_bytes);
    let mut parts = PartStore::new();
    parts
        .insert_zip_entry("ppt/media/image1.png", matching_bytes)
        .expect("valid fixture part");
    parts
        .insert_zip_entry("ppt/media/image2.png", b"different image bytes".to_vec())
        .expect("valid fixture part");

    let first = next_media_part_name(&parts, "PNG");
    let second = next_media_part_name(&parts, "png");
    assert_eq!(first.as_str(), "/ppt/media/image3.png");
    assert_eq!(first, second);

    let mut allocator = MediaPartNameAllocator::new();
    assert_eq!(
        allocator.next_media_part_name(&parts, "png").as_str(),
        "/ppt/media/image3.png"
    );
    assert_eq!(
        allocator.next_media_part_name(&parts, "png").as_str(),
        "/ppt/media/image4.png"
    );

    assert_eq!(dedup_lookup(&parts, &digest, false), None);
    let deduped = dedup_lookup(&parts, &digest, true).expect("matching part found");
    assert_eq!(deduped.as_str(), "/ppt/media/image1.png");
}
