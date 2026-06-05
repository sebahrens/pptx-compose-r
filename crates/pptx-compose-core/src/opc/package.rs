use std::collections::BTreeSet;

use crate::opc::{
    content_types::ContentTypes,
    part::{Part, PartStore},
    part_name::PartName,
    relationships::{Relationship, RelationshipGraph},
};

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

    pub fn insert_part(&mut self, part: Part) -> crate::error::Result<&Part> {
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

    pub fn push_relationship(&mut self, relationship: Relationship) {
        self.relationships.push(relationship);
    }

    pub fn push_slide_id(&mut self, slide_id: SlideIdEntry) {
        self.slide_ids.push(slide_id);
    }

    pub fn mark_dirty(&mut self, part_name: PartName) {
        self.dirty_parts.insert(part_name);
    }
}
