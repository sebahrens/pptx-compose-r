use crate::{
    error::{Error, Result},
    opc::part_name::PartName,
};

#[derive(Clone, Debug, Eq, PartialEq)]
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
    pub target_mode: TargetMode,
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
            target_mode: TargetMode::Internal,
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
            target_mode: TargetMode::External,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RelationshipGraph {
    relationships: Vec<Relationship>,
}

impl RelationshipGraph {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, relationship: Relationship) {
        self.relationships.push(relationship);
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &Relationship> {
        self.relationships.iter()
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

fn split_part_segments(part_name: &PartName) -> Vec<String> {
    part_name
        .as_str()
        .trim_start_matches('/')
        .split('/')
        .map(str::to_owned)
        .collect()
}
