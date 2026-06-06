use std::collections::{BTreeMap, BTreeSet};

use crate::{
    error::{Error, ErrorCode, Result},
    opc::part_name::PartName,
    xml::{document::XmlElement, parser::parse_document},
};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RelationshipSource {
    Package,
    Part(PartName),
}

impl RelationshipSource {
    #[must_use]
    pub fn location_part(&self) -> Option<&PartName> {
        match self {
            Self::Package => None,
            Self::Part(part_name) => Some(part_name),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetMode {
    Internal,
    External,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Relationship {
    pub source: RelationshipSource,
    pub id: String,
    pub rel_type: String,
    pub target: String,
    pub mode: TargetMode,
    pub target_mode: TargetMode,
    pub resolved_target: Option<PartName>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExternalRelationship<'a> {
    pub source: &'a RelationshipSource,
    pub id: &'a str,
    pub rel_type: &'a str,
    pub target: &'a str,
}

impl Relationship {
    #[must_use]
    pub fn internal(
        source: RelationshipSource,
        id: impl Into<String>,
        rel_type: impl Into<String>,
        target: impl Into<String>,
    ) -> Self {
        Self {
            source,
            id: id.into(),
            rel_type: rel_type.into(),
            target: target.into(),
            mode: TargetMode::Internal,
            target_mode: TargetMode::Internal,
            resolved_target: None,
        }
    }

    #[must_use]
    pub fn external(
        source: RelationshipSource,
        id: impl Into<String>,
        rel_type: impl Into<String>,
        target: impl Into<String>,
    ) -> Self {
        Self {
            source,
            id: id.into(),
            rel_type: rel_type.into(),
            target: target.into(),
            mode: TargetMode::External,
            target_mode: TargetMode::External,
            resolved_target: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationshipSet {
    pub source: PartName,
    pub rels: Vec<Relationship>,
}

impl RelationshipSet {
    pub fn parse(source_part: &PartName, raw: &[u8]) -> Result<Self> {
        let document = parse_document(raw).map_err(|source| {
            Error::with_source(
                ErrorCode::UnsupportedPackage,
                format!("Could not parse relationship part for {source_part}."),
                source,
            )
        })?;
        let root = document.root_element().ok_or_else(|| {
            Error::unsupported_package(format!(
                "Relationship part for {source_part} has no root element."
            ))
        })?;

        if root.name.local_name != "Relationships" {
            return Err(Error::unsupported_package(format!(
                "Relationship part for {source_part} root element is not Relationships."
            )));
        }

        let mut rels = Vec::new();
        for child in root.children.iter().filter_map(|node| node.as_element()) {
            if child.name.local_name != "Relationship" {
                continue;
            }

            rels.push(parse_relationship(source_part, child)?);
        }

        Ok(Self {
            source: source_part.clone(),
            rels,
        })
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Relationship> {
        self.rels.iter().find(|relationship| relationship.id == id)
    }

    #[must_use]
    pub fn allocate_id(&self) -> String {
        let existing = self
            .rels
            .iter()
            .map(|relationship| relationship.id.as_str())
            .collect::<BTreeSet<_>>();
        let max_suffix = existing
            .iter()
            .filter_map(|id| relationship_id_suffix(id))
            .max_by(|left, right| compare_decimal(left, right))
            .unwrap_or("0");

        let mut candidate_suffix = increment_decimal(max_suffix);
        loop {
            let candidate = format!("rId{candidate_suffix}");
            if !existing.contains(candidate.as_str()) {
                return candidate;
            }
            candidate_suffix = increment_decimal(&candidate_suffix);
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RelationshipGraph {
    relationships: Vec<Relationship>,
    sets: BTreeMap<PartName, RelationshipSet>,
}

impl RelationshipGraph {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, relationship: Relationship) {
        if let RelationshipSource::Part(source) = &relationship.source {
            let set = self
                .sets
                .entry(source.clone())
                .or_insert_with(|| RelationshipSet {
                    source: source.clone(),
                    rels: Vec::new(),
                });
            set.rels.push(relationship.clone());
        }
        self.relationships.push(relationship);
    }

    pub fn insert_set(&mut self, set: RelationshipSet) {
        self.relationships.extend(set.rels.iter().cloned());
        self.sets.insert(set.source.clone(), set);
    }

    #[must_use]
    pub fn set_for(&self, source: &PartName) -> Option<&RelationshipSet> {
        self.sets.get(source)
    }

    #[must_use]
    pub fn resolve(&self, source: &PartName, r_id: &str) -> Option<&PartName> {
        self.set_for(source)?.get(r_id)?.resolved_target.as_ref()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &Relationship> {
        self.relationships.iter()
    }

    pub fn external_relationships(&self) -> impl Iterator<Item = ExternalRelationship<'_>> {
        self.relationships
            .iter()
            .filter(|relationship| relationship.target_mode == TargetMode::External)
            .map(|relationship| ExternalRelationship {
                source: &relationship.source,
                id: relationship.id.as_str(),
                rel_type: relationship.rel_type.as_str(),
                target: relationship.target.as_str(),
            })
    }
}

pub fn resolve_internal_target(source: &RelationshipSource, target: &str) -> Result<PartName> {
    if target.starts_with('/') {
        return PartName::from_zip_entry(target);
    }

    let mut segments = match source {
        RelationshipSource::Package => Vec::new(),
        RelationshipSource::Part(part_name) => {
            let mut segments = split_part_segments(part_name);
            let _ = segments.pop();
            segments
        }
    };

    for segment in target.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if segments.pop().is_none() {
                    return Err(Error::unsafe_path(
                        "Relationship target resolves above the package root.",
                    ));
                }
            }
            segment => segments.push(segment.to_owned()),
        }
    }

    PartName::from_zip_entry(segments.join("/").as_str())
}

fn parse_relationship(source_part: &PartName, element: &XmlElement) -> Result<Relationship> {
    let id = required_attr(element, "Id")?;
    let rel_type = required_attr(element, "Type")?;
    let target = required_attr(element, "Target")?;
    let mode = match optional_attr(element, "TargetMode") {
        None | Some("Internal") => TargetMode::Internal,
        Some("External") => TargetMode::External,
        Some(other) => {
            return Err(Error::unsupported_package(format!(
                "Relationship {id} has unsupported TargetMode {other}."
            )));
        }
    };
    let source = RelationshipSource::Part(source_part.clone());
    let resolved_target = match mode {
        TargetMode::Internal => Some(resolve_internal_target(&source, target)?),
        TargetMode::External => None,
    };

    Ok(Relationship {
        source,
        id: id.to_owned(),
        rel_type: rel_type.to_owned(),
        target: target.to_owned(),
        mode,
        target_mode: mode,
        resolved_target,
    })
}

fn required_attr<'a>(element: &'a XmlElement, name: &str) -> Result<&'a str> {
    optional_attr(element, name).ok_or_else(|| {
        Error::unsupported_package(format!(
            "Relationship element is missing required attribute {name}."
        ))
    })
}

fn optional_attr<'a>(element: &'a XmlElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.local_name == name)
        .map(|attribute| attribute.value.as_str())
}

fn split_part_segments(part_name: &PartName) -> Vec<String> {
    part_name
        .as_str()
        .trim_start_matches('/')
        .split('/')
        .map(str::to_owned)
        .collect()
}

fn relationship_id_suffix(id: &str) -> Option<&str> {
    let suffix = id.strip_prefix("rId")?;
    if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(suffix)
}

fn compare_decimal(left: &str, right: &str) -> std::cmp::Ordering {
    let left = normalize_decimal(left);
    let right = normalize_decimal(right);
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}

fn normalize_decimal(value: &str) -> &str {
    let trimmed = value.trim_start_matches('0');
    if trimmed.is_empty() { "0" } else { trimmed }
}

fn increment_decimal(value: &str) -> String {
    let mut digits = value.as_bytes().to_vec();
    let mut index = digits.len();

    while index > 0 {
        index -= 1;
        if digits[index] != b'9' {
            digits[index] += 1;
            return digits.into_iter().map(char::from).collect();
        }
        digits[index] = b'0';
    }

    let mut incremented = String::with_capacity(digits.len() + 1);
    incremented.push('1');
    incremented.extend(digits.into_iter().map(char::from));
    incremented
}

#[cfg(test)]
#[test]
fn parse_and_resolve() {
    let source = PartName::from_zip_entry("/ppt/presentation.xml").expect("valid source");
    let raw = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.test/image.png?a=1&amp;b=2" TargetMode="External"/></Relationships>"#;

    let set = RelationshipSet::parse(&source, raw).expect("relationships parse");

    assert_eq!(set.source, source);
    assert_eq!(set.rels.len(), 2);
    assert_eq!(set.rels[0].id, "rId1");
    assert_eq!(set.rels[0].mode, TargetMode::Internal);
    assert_eq!(set.rels[0].target, "slides/slide1.xml");
    assert_eq!(
        set.rels[0]
            .resolved_target
            .as_ref()
            .expect("internal target resolved")
            .as_str(),
        "/ppt/slides/slide1.xml"
    );
    assert_eq!(set.rels[1].mode, TargetMode::External);
    assert_eq!(
        set.rels[1].target,
        "https://example.test/image.png?a=1&amp;b=2"
    );
    assert!(set.rels[1].resolved_target.is_none());

    let mut graph = RelationshipGraph::new();
    graph.insert_set(set);
    assert_eq!(
        graph
            .resolve(&source, "rId1")
            .expect("graph resolves relationship")
            .as_str(),
        "/ppt/slides/slide1.xml"
    );
    assert!(graph.resolve(&source, "rId2").is_none());
}

#[cfg(test)]
mod allocate_id {
    use super::*;

    fn relationship_set(ids: &[&str]) -> RelationshipSet {
        let source = PartName::from_zip_entry("/ppt/slides/slide1.xml").expect("valid source");
        let rels = ids
            .iter()
            .map(|id| {
                Relationship::internal(
                    RelationshipSource::Part(source.clone()),
                    *id,
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image",
                    "../media/image1.png",
                )
            })
            .collect();

        RelationshipSet { source, rels }
    }

    #[test]
    fn allocates_after_max_numeric_suffix() {
        let set = relationship_set(&["rId1", "rId2", "rId5"]);

        assert_eq!(set.allocate_id(), "rId6");
    }

    #[test]
    fn empty_set_starts_at_one() {
        let set = relationship_set(&[]);

        assert_eq!(set.allocate_id(), "rId1");
    }

    #[test]
    fn ignores_non_conforming_ids_for_max_but_avoids_collisions() {
        let set = relationship_set(&["rId1", "rId2x", "custom", "rId2"]);

        assert_eq!(set.allocate_id(), "rId3");
    }

    #[test]
    fn increments_until_candidate_is_free() {
        let set = relationship_set(&["rId1", "rId03", "rId4"]);

        assert_eq!(set.allocate_id(), "rId5");
    }
}
