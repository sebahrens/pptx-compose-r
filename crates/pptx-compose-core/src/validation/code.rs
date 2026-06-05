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
    UnresolvedRelationshipReference,
    DuplicateRelationshipId,
    ExternalRelationshipNotChecked,
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

pub const FINDING_REGISTRY: &[(FindingCode, FindingCategory, Severity)] = &[
    (
        FindingCode::MissingContentType,
        FindingCategory::ContentType,
        Severity::Error,
    ),
    (
        FindingCode::MediaContentTypeMismatch,
        FindingCategory::ContentType,
        Severity::Error,
    ),
    (
        FindingCode::DanglingInternalRelationship,
        FindingCategory::Relationship,
        Severity::Error,
    ),
    (
        FindingCode::UnresolvedRelationshipReference,
        FindingCategory::Relationship,
        Severity::Error,
    ),
    (
        FindingCode::DuplicateRelationshipId,
        FindingCategory::Relationship,
        Severity::Error,
    ),
    (
        FindingCode::ExternalRelationshipNotChecked,
        FindingCategory::Relationship,
        Severity::Warning,
    ),
    (
        FindingCode::DuplicateSlideId,
        FindingCategory::Presentation,
        Severity::Error,
    ),
    (
        FindingCode::SlideOrderMismatch,
        FindingCategory::Presentation,
        Severity::Error,
    ),
    (
        FindingCode::DuplicateDrawingId,
        FindingCategory::Slide,
        Severity::Error,
    ),
    (
        FindingCode::InvalidBounds,
        FindingCategory::Slide,
        Severity::Error,
    ),
    (
        FindingCode::MalformedXml,
        FindingCategory::Xml,
        Severity::Fatal,
    ),
    (
        FindingCode::MissingNamespaceDeclaration,
        FindingCategory::Xml,
        Severity::Error,
    ),
    (
        FindingCode::PartDropped,
        FindingCategory::Package,
        Severity::Fatal,
    ),
    (
        FindingCode::OrphanPart,
        FindingCategory::Package,
        Severity::Info,
    ),
    (
        FindingCode::SignatureInvalidatedByEdit,
        FindingCategory::Signature,
        Severity::Warning,
    ),
];

impl FindingCode {
    #[must_use]
    pub const fn default_entry(self) -> (FindingCategory, Severity) {
        match self {
            Self::MissingContentType => (FindingCategory::ContentType, Severity::Error),
            Self::MediaContentTypeMismatch => (FindingCategory::ContentType, Severity::Error),
            Self::DanglingInternalRelationship => (FindingCategory::Relationship, Severity::Error),
            Self::UnresolvedRelationshipReference => {
                (FindingCategory::Relationship, Severity::Error)
            }
            Self::DuplicateRelationshipId => (FindingCategory::Relationship, Severity::Error),
            Self::ExternalRelationshipNotChecked => {
                (FindingCategory::Relationship, Severity::Warning)
            }
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

#[test]
fn registry_matches_044_table() {
    let expected = [
        (
            FindingCode::MissingContentType,
            FindingCategory::ContentType,
            Severity::Error,
        ),
        (
            FindingCode::MediaContentTypeMismatch,
            FindingCategory::ContentType,
            Severity::Error,
        ),
        (
            FindingCode::DanglingInternalRelationship,
            FindingCategory::Relationship,
            Severity::Error,
        ),
        (
            FindingCode::UnresolvedRelationshipReference,
            FindingCategory::Relationship,
            Severity::Error,
        ),
        (
            FindingCode::DuplicateRelationshipId,
            FindingCategory::Relationship,
            Severity::Error,
        ),
        (
            FindingCode::ExternalRelationshipNotChecked,
            FindingCategory::Relationship,
            Severity::Warning,
        ),
        (
            FindingCode::DuplicateSlideId,
            FindingCategory::Presentation,
            Severity::Error,
        ),
        (
            FindingCode::SlideOrderMismatch,
            FindingCategory::Presentation,
            Severity::Error,
        ),
        (
            FindingCode::DuplicateDrawingId,
            FindingCategory::Slide,
            Severity::Error,
        ),
        (
            FindingCode::InvalidBounds,
            FindingCategory::Slide,
            Severity::Error,
        ),
        (
            FindingCode::MalformedXml,
            FindingCategory::Xml,
            Severity::Fatal,
        ),
        (
            FindingCode::MissingNamespaceDeclaration,
            FindingCategory::Xml,
            Severity::Error,
        ),
        (
            FindingCode::PartDropped,
            FindingCategory::Package,
            Severity::Fatal,
        ),
        (
            FindingCode::OrphanPart,
            FindingCategory::Package,
            Severity::Info,
        ),
        (
            FindingCode::SignatureInvalidatedByEdit,
            FindingCategory::Signature,
            Severity::Warning,
        ),
    ];

    assert_eq!(FINDING_REGISTRY, expected);
    assert_eq!(FINDING_REGISTRY.len(), 15);

    for (code, category, severity) in expected {
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
