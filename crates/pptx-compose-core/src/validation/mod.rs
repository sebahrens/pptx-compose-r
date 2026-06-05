pub mod code;
pub mod invariants;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use code::{FINDING_REGISTRY, FindingCategory, FindingCode, Severity};
use invariants::check_invariants;

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
