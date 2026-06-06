use std::collections::BTreeSet;

use crate::{
    error::{Error, Result},
    opc::{
        content_types::ContentTypes,
        part::{Part, PartStore},
        part_name::PartName,
        relationships::{Relationship, RelationshipGraph},
    },
};

pub const OFFICE_DOCUMENT_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlideIdEntry {
    pub slide_id: String,
    pub relationship_id: Option<String>,
    pub part: Option<PartName>,
}

impl SlideIdEntry {
    #[must_use]
    pub fn new(slide_id: impl Into<String>) -> Self {
        Self {
            slide_id: slide_id.into(),
            relationship_id: None,
            part: None,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Package {
    parts: PartStore,
    content_types: ContentTypes,
    relationships: RelationshipGraph,
    slide_ids: Vec<SlideIdEntry>,
    dirty_parts: BTreeSet<PartName>,
    original_parts: BTreeSet<PartName>,
}

impl Package {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn parts(&self) -> &PartStore {
        &self.parts
    }

    #[must_use]
    pub fn parts_mut(&mut self) -> &mut PartStore {
        &mut self.parts
    }

    #[must_use]
    pub fn content_types(&self) -> &ContentTypes {
        &self.content_types
    }

    #[must_use]
    pub fn content_types_mut(&mut self) -> &mut ContentTypes {
        &mut self.content_types
    }

    #[must_use]
    pub fn relationships(&self) -> &RelationshipGraph {
        &self.relationships
    }

    #[must_use]
    pub fn relationships_mut(&mut self) -> &mut RelationshipGraph {
        &mut self.relationships
    }

    #[must_use]
    pub fn slide_ids(&self) -> &[SlideIdEntry] {
        &self.slide_ids
    }

    #[must_use]
    pub fn dirty_parts(&self) -> &BTreeSet<PartName> {
        &self.dirty_parts
    }

    #[must_use]
    pub fn original_parts(&self) -> &BTreeSet<PartName> {
        &self.original_parts
    }

    pub fn office_document_part(&self) -> Result<PartName> {
        let root_rels_part = root_relationships_part()?;
        let root_rels = self.relationships.set_for(&root_rels_part).ok_or_else(|| {
            Error::malformed_package(
                "Package root relationships do not contain an Office document relationship.",
            )
        })?;

        let relationship = root_rels
            .rels
            .iter()
            .find(|relationship| relationship.rel_type == OFFICE_DOCUMENT_REL_TYPE)
            .ok_or_else(|| {
                Error::malformed_package(
                    "Package root relationships do not contain an Office document relationship.",
                )
            })?;

        relationship.resolved_target.clone().ok_or_else(|| {
            Error::malformed_package(format!(
                "Office document relationship {} does not resolve to an internal package part.",
                relationship.id
            ))
        })
    }

    pub fn insert_part(&mut self, part: Part) -> Result<&Part> {
        let name = part.name().clone();
        let inserted = self.parts.insert(part)?;
        self.original_parts.insert(name);
        Ok(inserted)
    }

    pub fn insert_zip_entry(
        &mut self,
        zip_entry_name: impl Into<String>,
        bytes: Vec<u8>,
    ) -> crate::error::Result<&Part> {
        let part = Part::from_zip_entry(zip_entry_name, bytes)?;
        self.insert_part(part)
    }

    pub fn remove_part(&mut self, part_name: &PartName) -> Option<Part> {
        let removed = self.parts.remove(part_name)?;
        self.original_parts.remove(part_name);
        self.dirty_parts.remove(part_name);
        Some(removed)
    }

    pub fn push_relationship(&mut self, relationship: Relationship) {
        self.relationships.push(relationship);
    }

    pub fn push_slide_id(&mut self, slide_id: SlideIdEntry) {
        self.slide_ids.push(slide_id);
    }

    pub fn replace_slide_ids(&mut self, slide_ids: Vec<SlideIdEntry>) {
        self.slide_ids = slide_ids;
    }

    pub fn mark_dirty(&mut self, part_name: PartName) {
        self.dirty_parts.insert(part_name);
    }
}

fn root_relationships_part() -> Result<PartName> {
    PartName::from_zip_entry("/_rels/.rels")
}

#[cfg(test)]
#[test]
fn discovers_office_document() {
    use crate::{
        error::ErrorCode,
        opc::relationships::{RelationshipSet, RelationshipSource, TargetMode},
    };

    let root_rels_part = root_relationships_part().expect("root relationships part is valid");
    let presentation_part =
        PartName::from_zip_entry("/ppt/presentation.xml").expect("presentation part is valid");
    let mut package = Package::new();
    package.relationships.insert_set(RelationshipSet {
        source: root_rels_part,
        rels: vec![
            Relationship {
                source: RelationshipSource::Package,
                id: "rUnknown".to_owned(),
                rel_type: "https://example.test/unknown".to_owned(),
                target: "custom.xml".to_owned(),
                mode: TargetMode::Internal,
                target_mode: TargetMode::Internal,
                resolved_target: Some(
                    PartName::from_zip_entry("/custom.xml").expect("unknown target is valid"),
                ),
            },
            Relationship {
                source: RelationshipSource::Package,
                id: "rOffice".to_owned(),
                rel_type: OFFICE_DOCUMENT_REL_TYPE.to_owned(),
                target: "ppt/presentation.xml".to_owned(),
                mode: TargetMode::Internal,
                target_mode: TargetMode::Internal,
                resolved_target: Some(presentation_part.clone()),
            },
        ],
    });

    assert_eq!(
        package
            .office_document_part()
            .expect("office document resolves"),
        presentation_part
    );

    let missing = Package::new()
        .office_document_part()
        .expect_err("missing office document is malformed");
    assert_eq!(missing.code(), ErrorCode::UnsupportedPackage);
}
