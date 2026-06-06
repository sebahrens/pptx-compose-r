use std::collections::BTreeSet;

use crate::{
    error::{Error, Result},
    opc::{
        package::Package,
        part::PartStore,
        part_name::PartName,
        relationships::{Relationship, RelationshipSet, TargetMode, resolve_internal_target},
    },
    provenance::checksum::part_checksum,
};

const MEDIA_PREFIX: &str = "/ppt/media/image";
pub const IMAGE_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedMedia {
    pub part_name: PartName,
    pub content_type: String,
    pub byte_length: u64,
    pub shared_ref_count: u32,
}

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

pub fn resolve_embedded_media(
    rel_id: &str,
    slide_rels: &RelationshipSet,
    package: &Package,
) -> Result<ResolvedMedia> {
    let relationship = slide_rels.get(rel_id).ok_or_else(|| {
        Error::unsupported_package(format!("Picture relationship {rel_id} is missing."))
    })?;

    if relationship.target_mode != TargetMode::Internal {
        return Err(Error::unsupported_package(format!(
            "Picture relationship {rel_id} does not target an internal media part."
        )));
    }

    if relationship.rel_type != IMAGE_REL_TYPE {
        return Err(Error::unsupported_package(format!(
            "Picture relationship {rel_id} is not an image relationship."
        )));
    }

    let part_name = relationship_target_part(relationship)?;
    let part = package.parts().get(&part_name).ok_or_else(|| {
        Error::unsupported_package(format!(
            "Picture relationship {rel_id} targets missing media part {part_name}."
        ))
    })?;
    let content_type = package
        .content_types()
        .resolve(&part_name)
        .ok_or_else(|| {
            Error::unsupported_package(format!("Media part {part_name} has no content type."))
        })?
        .to_owned();
    let byte_length = u64::try_from(part.bytes().len()).map_err(|_| {
        Error::resource_limit_exceeded(format!("Media part {part_name} is too large."))
    })?;
    let shared_ref_count = shared_media_ref_count(package, &part_name);

    Ok(ResolvedMedia {
        part_name,
        content_type,
        byte_length,
        shared_ref_count,
    })
}

#[must_use]
pub fn shared_media_ref_count(package: &Package, media_part: &PartName) -> u32 {
    let count = package
        .relationships()
        .iter()
        .filter(|relationship| relationship.target_mode == TargetMode::Internal)
        .filter_map(|relationship| relationship_target_part(relationship).ok())
        .filter(|part_name| part_name == media_part)
        .count();

    u32::try_from(count).unwrap_or(u32::MAX)
}

fn relationship_target_part(relationship: &Relationship) -> Result<PartName> {
    if let Some(target) = &relationship.resolved_target {
        Ok(target.clone())
    } else {
        resolve_internal_target(&relationship.source, &relationship.target)
    }
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
