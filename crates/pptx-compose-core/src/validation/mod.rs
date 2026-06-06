pub mod code;
pub mod invariants;
pub mod package_graph;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use code::{FINDING_REGISTRY, FindingCategory, FindingCode, Severity};
use invariants::check_invariants;
use package_graph::check_package_graph;

use crate::opc::package::Package;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidationMode {
    NoEdit,
    Edited,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    Valid,
    Invalid,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValidationSummary {
    pub fatal: usize,
    pub errors: usize,
    pub warnings: usize,
    pub info: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValidationOutcome {
    pub status: ValidationStatus,
    pub summary: ValidationSummary,
    pub findings: Vec<Finding>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub severity: Severity,
    pub category: FindingCategory,
    pub code: FindingCode,
    pub message: String,
    pub blocking: bool,
    pub location: Value,
    pub suggested_action: Option<String>,
}

impl Finding {
    /// Finding severity starts at the registry default and may only be escalated.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        code: FindingCode,
        message: impl Into<String>,
        blocking: bool,
        location: Value,
        suggested_action: Option<String>,
    ) -> Self {
        let (category, severity) = code.default_entry();

        Self {
            id: id.into(),
            severity,
            category,
            code,
            message: message.into(),
            blocking,
            location,
            suggested_action,
        }
    }
}

#[must_use]
pub fn validate_package(pkg: &Package, mode: ValidationMode) -> ValidationOutcome {
    let mut findings = Vec::new();
    findings.extend(check_package_graph(pkg));
    check_invariants(pkg, &mut findings);

    for (index, finding) in findings.iter_mut().enumerate() {
        finding.id = format!("finding-{}", index + 1);
        finding.blocking = match mode {
            ValidationMode::NoEdit => finding.severity == Severity::Fatal,
            ValidationMode::Edited => {
                finding.severity == Severity::Fatal || finding.severity == Severity::Error
            }
        };
    }

    let summary = summarize(&findings);
    let status = if findings.iter().any(|finding| finding.blocking) {
        ValidationStatus::Invalid
    } else {
        ValidationStatus::Valid
    };

    ValidationOutcome {
        status,
        summary,
        findings,
    }
}

fn summarize(findings: &[Finding]) -> ValidationSummary {
    let mut summary = ValidationSummary::default();

    for finding in findings {
        match finding.severity {
            Severity::Fatal => summary.fatal += 1,
            Severity::Error => summary.errors += 1,
            Severity::Warning => summary.warnings += 1,
            Severity::Info => summary.info += 1,
        }
    }

    summary
}

#[must_use]
pub(crate) fn location(entries: &[(&str, String)]) -> Value {
    let mut object = serde_json::Map::new();
    for (key, value) in entries {
        object.insert((*key).to_owned(), json!(value));
    }
    Value::Object(object)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FindingCoverage {
    pub code: FindingCode,
    pub producer: Option<&'static str>,
    pub producer_test: Option<&'static str>,
    pub deferral: Option<Deferral>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Deferral {
    pub owner: &'static str,
    pub spec: &'static str,
    pub reason: &'static str,
}

#[doc(hidden)]
pub const FINDING_COVERAGE: &[FindingCoverage] = &[
    FindingCoverage {
        code: FindingCode::MissingContentType,
        producer: Some(package_graph::PRODUCER_CHECK_MISSING_CONTENT_TYPE),
        producer_test: Some("validation::package_graph::detects_violations"),
        deferral: None,
    },
    FindingCoverage {
        code: FindingCode::MediaContentTypeMismatch,
        producer: None,
        producer_test: None,
        deferral: Some(Deferral {
            owner: "media sniff validation task",
            spec: "specs/012-content-types-and-relationships.md; specs/032; specs/044-results-validation-errors.md",
            reason: "Media magic-byte sniffing and content-type comparison are owned by the media validation work; this task only audits registry coverage.",
        }),
    },
    FindingCoverage {
        code: FindingCode::DanglingInternalRelationship,
        producer: Some(package_graph::PRODUCER_CHECK_DANGLING_INTERNAL_RELATIONSHIP),
        producer_test: Some("validation::package_graph::detects_violations"),
        deferral: None,
    },
    FindingCoverage {
        code: FindingCode::UnresolvedRelationshipReference,
        producer: None,
        producer_test: None,
        deferral: Some(Deferral {
            owner: "slide XML relationship reference validation task",
            spec: "specs/012-content-types-and-relationships.md; specs/050-roundtrip-invariants.md",
            reason: "The core package validator does not yet parse every XML relationship reference that can carry an r:id/rId.",
        }),
    },
    FindingCoverage {
        code: FindingCode::DuplicateRelationshipId,
        producer: Some(package_graph::PRODUCER_CHECK_DUPLICATE_RELATIONSHIP_ID),
        producer_test: Some("validation::package_graph::detects_violations"),
        deferral: None,
    },
    FindingCoverage {
        code: FindingCode::ExternalRelationshipNotChecked,
        producer: Some(invariants::PRODUCER_CHECK_EXTERNAL_RELATIONSHIP_NOT_CHECKED),
        producer_test: Some(
            "hazardous_package_preservation::preserves_hazardous_parts_and_warns_without_fetching",
        ),
        deferral: None,
    },
    FindingCoverage {
        code: FindingCode::DuplicateSlideId,
        producer: Some(invariants::PRODUCER_CHECK_DUPLICATE_SLIDE_ID),
        producer_test: Some("validation::invariants::tests::detects_seeded_violations"),
        deferral: None,
    },
    FindingCoverage {
        code: FindingCode::SlideOrderMismatch,
        producer: None,
        producer_test: None,
        deferral: Some(Deferral {
            owner: "presentation slide-order validation task",
            spec: "specs/050-roundtrip-invariants.md",
            reason: "Slide order comparison needs the presentation model and explicit reorder-edit state.",
        }),
    },
    FindingCoverage {
        code: FindingCode::DuplicateDrawingId,
        producer: None,
        producer_test: None,
        deferral: Some(Deferral {
            owner: "slide shape-tree validation task",
            spec: "specs/050-roundtrip-invariants.md",
            reason: "Drawing non-visual id validation belongs with slide shape-tree parsing.",
        }),
    },
    FindingCoverage {
        code: FindingCode::InvalidBounds,
        producer: None,
        producer_test: None,
        deferral: Some(Deferral {
            owner: "edit/DrawingML validation task",
            spec: "specs/047-drawingml-construction.md; specs/044-results-validation-errors.md",
            reason: "Bounds are validated when constructing or moving DrawingML elements, not by the package-level invariant pass.",
        }),
    },
    FindingCoverage {
        code: FindingCode::MalformedXml,
        producer: None,
        producer_test: None,
        deferral: Some(Deferral {
            owner: "dirty XML writer validation task",
            spec: "specs/050-roundtrip-invariants.md",
            reason: "Only dirty XML parts are reserialized; malformed written XML detection belongs with dirty-part XML writing.",
        }),
    },
    FindingCoverage {
        code: FindingCode::MissingNamespaceDeclaration,
        producer: None,
        producer_test: None,
        deferral: Some(Deferral {
            owner: "dirty XML namespace validation task",
            spec: "specs/047-drawingml-construction.md; specs/050-roundtrip-invariants.md",
            reason: "Namespace declaration checks require dirty XML construction/writer context.",
        }),
    },
    FindingCoverage {
        code: FindingCode::PartDropped,
        producer: Some(invariants::PRODUCER_CHECK_PART_DROPPED),
        producer_test: Some(
            "validation::registry_coverage::every_044_finding_has_producer_or_deferral",
        ),
        deferral: None,
    },
    FindingCoverage {
        code: FindingCode::OrphanPart,
        producer: None,
        producer_test: None,
        deferral: Some(Deferral {
            owner: "relationship graph reachability validation task",
            spec: "specs/010; specs/044-results-validation-errors.md",
            reason: "Reachability analysis is informational and awaits full package relationship graph roots.",
        }),
    },
    FindingCoverage {
        code: FindingCode::SignatureInvalidatedByEdit,
        producer: Some(invariants::PRODUCER_CHECK_SIGNATURE_INVALIDATED_BY_EDIT),
        producer_test: Some(
            "hazardous_package_preservation::preserves_hazardous_parts_and_warns_without_fetching",
        ),
        deferral: None,
    },
];
