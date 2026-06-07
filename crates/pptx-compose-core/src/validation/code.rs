use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
    Fatal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingCategory {
    ContentType,
    Relationship,
    Presentation,
    Slide,
    Comments,
    Xml,
    Package,
    Signature,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingCode {
    MissingContentType,
    MediaContentTypeMismatch,
    DanglingInternalRelationship,
    UnreferencedMedia,
    UnresolvedRelationshipReference,
    DuplicateRelationshipId,
    ExternalRelationshipNotChecked,
    DanglingCommentAuthorRef,
    DuplicateSlideId,
    SlideOrderMismatch,
    DuplicateDrawingId,
    InvalidBounds,
    MalformedXml,
    MissingNamespaceDeclaration,
    PartDropped,
    OrphanPart,
    SignatureInvalidatedByEdit,
}

impl FindingCode {
    pub const ALL: &[Self] = &[
        Self::MissingContentType,
        Self::MediaContentTypeMismatch,
        Self::DanglingInternalRelationship,
        Self::UnreferencedMedia,
        Self::UnresolvedRelationshipReference,
        Self::DuplicateRelationshipId,
        Self::ExternalRelationshipNotChecked,
        Self::DanglingCommentAuthorRef,
        Self::DuplicateSlideId,
        Self::SlideOrderMismatch,
        Self::DuplicateDrawingId,
        Self::InvalidBounds,
        Self::MalformedXml,
        Self::MissingNamespaceDeclaration,
        Self::PartDropped,
        Self::OrphanPart,
        Self::SignatureInvalidatedByEdit,
    ];

    #[must_use]
    pub const fn default_entry(self) -> (FindingCategory, Severity) {
        match self {
            Self::MissingContentType => (FindingCategory::ContentType, Severity::Error),
            Self::MediaContentTypeMismatch => (FindingCategory::ContentType, Severity::Error),
            Self::DanglingInternalRelationship => (FindingCategory::Relationship, Severity::Error),
            Self::UnreferencedMedia => (FindingCategory::Package, Severity::Info),
            Self::UnresolvedRelationshipReference => {
                (FindingCategory::Relationship, Severity::Error)
            }
            Self::DuplicateRelationshipId => (FindingCategory::Relationship, Severity::Error),
            Self::ExternalRelationshipNotChecked => {
                (FindingCategory::Relationship, Severity::Warning)
            }
            Self::DanglingCommentAuthorRef => (FindingCategory::Comments, Severity::Error),
            Self::DuplicateSlideId => (FindingCategory::Presentation, Severity::Error),
            Self::SlideOrderMismatch => (FindingCategory::Presentation, Severity::Error),
            Self::DuplicateDrawingId => (FindingCategory::Slide, Severity::Error),
            Self::InvalidBounds => (FindingCategory::Slide, Severity::Error),
            Self::MalformedXml => (FindingCategory::Xml, Severity::Fatal),
            Self::MissingNamespaceDeclaration => (FindingCategory::Xml, Severity::Error),
            Self::PartDropped => (FindingCategory::Package, Severity::Fatal),
            Self::OrphanPart => (FindingCategory::Package, Severity::Info),
            Self::SignatureInvalidatedByEdit => (FindingCategory::Signature, Severity::Warning),
        }
    }
}

const fn registry_entry(code: FindingCode) -> (FindingCode, FindingCategory, Severity) {
    let (category, severity) = code.default_entry();
    (code, category, severity)
}

const fn build_finding_registry()
-> [(FindingCode, FindingCategory, Severity); FindingCode::ALL.len()] {
    let mut entries = [registry_entry(FindingCode::MissingContentType); FindingCode::ALL.len()];
    let mut index = 0;

    while index < FindingCode::ALL.len() {
        entries[index] = registry_entry(FindingCode::ALL[index]);
        index += 1;
    }

    entries
}

const FINDING_REGISTRY_ENTRIES: [(FindingCode, FindingCategory, Severity); FindingCode::ALL.len()] =
    build_finding_registry();

pub const FINDING_REGISTRY: &[(FindingCode, FindingCategory, Severity)] = &FINDING_REGISTRY_ENTRIES;

#[test]
fn registry_matches_044_table() {
    assert_eq!(FindingCode::ALL.len(), 17);
    assert_eq!(FINDING_REGISTRY.len(), FindingCode::ALL.len());

    for (&(code, category, severity), &expected_code) in
        FINDING_REGISTRY.iter().zip(FindingCode::ALL)
    {
        assert_eq!(code, expected_code);
        assert_eq!(code.default_entry(), (category, severity));
    }

    assert_eq!(
        FindingCode::MalformedXml.default_entry(),
        (FindingCategory::Xml, Severity::Fatal)
    );
    assert_eq!(
        FindingCode::ExternalRelationshipNotChecked.default_entry(),
        (FindingCategory::Relationship, Severity::Warning)
    );
    assert_eq!(
        FindingCode::OrphanPart.default_entry(),
        (FindingCategory::Package, Severity::Info)
    );
}
