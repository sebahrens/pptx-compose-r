use pptx_compose_core::{
    error::{Error, ErrorCode, Result},
    validation as core_validation,
};
use pptx_compose_json::{
    schema_versions::{VALIDATION_REPORT_SCHEMA, VALIDATION_REPORT_VERSION},
    schemas::{
        FindingCategory, FindingCode, FindingView, PatchValidationSummary, Severity, Summary,
        ValidationReport, ValidationStatus,
    },
};

pub fn validation_report(
    outcome: core_validation::ValidationOutcome,
    document_id: impl Into<String>,
    revision: u32,
) -> Result<ValidationReport> {
    Ok(ValidationReport {
        schema: VALIDATION_REPORT_SCHEMA.to_owned(),
        version: VALIDATION_REPORT_VERSION,
        document_id: document_id.into(),
        revision,
        status: validation_status(outcome.status),
        summary: Summary {
            fatal: count(outcome.summary.fatal)?,
            errors: count(outcome.summary.errors)?,
            warnings: count(outcome.summary.warnings)?,
            info: count(outcome.summary.info)?,
        },
        findings: outcome.findings.into_iter().map(finding_view).collect(),
    })
}

#[must_use]
pub const fn patch_validation_summary(report: &ValidationReport) -> PatchValidationSummary {
    PatchValidationSummary {
        status: report.status,
        errors: report.summary.errors + report.summary.fatal,
        warnings: report.summary.warnings,
    }
}

#[must_use]
pub fn has_blocking_findings(report: &ValidationReport) -> bool {
    report.findings.iter().any(|finding| finding.blocking)
}

fn finding_view(finding: core_validation::Finding) -> FindingView {
    FindingView {
        id: finding.id,
        severity: severity(finding.severity),
        category: category(finding.category),
        code: code(finding.code),
        message: finding.message,
        blocking: finding.blocking,
        location: finding.location,
        suggested_action: finding.suggested_action,
    }
}

const fn validation_status(status: core_validation::ValidationStatus) -> ValidationStatus {
    match status {
        core_validation::ValidationStatus::Valid => ValidationStatus::Valid,
        core_validation::ValidationStatus::Invalid => ValidationStatus::Invalid,
    }
}

const fn severity(severity: core_validation::Severity) -> Severity {
    match severity {
        core_validation::Severity::Info => Severity::Info,
        core_validation::Severity::Warning => Severity::Warning,
        core_validation::Severity::Error => Severity::Error,
        core_validation::Severity::Fatal => Severity::Fatal,
    }
}

const fn category(category: core_validation::FindingCategory) -> FindingCategory {
    match category {
        core_validation::FindingCategory::ContentType => FindingCategory::ContentType,
        core_validation::FindingCategory::Relationship => FindingCategory::Relationship,
        core_validation::FindingCategory::Presentation => FindingCategory::Presentation,
        core_validation::FindingCategory::Slide => FindingCategory::Slide,
        core_validation::FindingCategory::Comments => FindingCategory::Comments,
        core_validation::FindingCategory::Xml => FindingCategory::Xml,
        core_validation::FindingCategory::Package => FindingCategory::Package,
        core_validation::FindingCategory::Signature => FindingCategory::Signature,
    }
}

const fn code(code: core_validation::FindingCode) -> FindingCode {
    match code {
        core_validation::FindingCode::MissingContentType => FindingCode::MissingContentType,
        core_validation::FindingCode::MediaContentTypeMismatch => {
            FindingCode::MediaContentTypeMismatch
        }
        core_validation::FindingCode::DanglingInternalRelationship => {
            FindingCode::DanglingInternalRelationship
        }
        core_validation::FindingCode::UnreferencedMedia => FindingCode::UnreferencedMedia,
        core_validation::FindingCode::UnresolvedRelationshipReference => {
            FindingCode::UnresolvedRelationshipReference
        }
        core_validation::FindingCode::DuplicateRelationshipId => {
            FindingCode::DuplicateRelationshipId
        }
        core_validation::FindingCode::ExternalRelationshipNotChecked => {
            FindingCode::ExternalRelationshipNotChecked
        }
        core_validation::FindingCode::DanglingCommentAuthorRef => {
            FindingCode::DanglingCommentAuthorRef
        }
        core_validation::FindingCode::DuplicateSlideId => FindingCode::DuplicateSlideId,
        core_validation::FindingCode::SlideOrderMismatch => FindingCode::SlideOrderMismatch,
        core_validation::FindingCode::DuplicateDrawingId => FindingCode::DuplicateDrawingId,
        core_validation::FindingCode::InvalidBounds => FindingCode::InvalidBounds,
        core_validation::FindingCode::MalformedXml => FindingCode::MalformedXml,
        core_validation::FindingCode::MissingNamespaceDeclaration => {
            FindingCode::MissingNamespaceDeclaration
        }
        core_validation::FindingCode::PartDropped => FindingCode::PartDropped,
        core_validation::FindingCode::OrphanPart => FindingCode::OrphanPart,
        core_validation::FindingCode::SignatureInvalidatedByEdit => {
            FindingCode::SignatureInvalidatedByEdit
        }
    }
}

fn count(value: usize) -> Result<u32> {
    u32::try_from(value).map_err(|source| {
        Error::with_source(
            ErrorCode::InternalError,
            "Validation finding count exceeds report schema range.",
            source,
        )
    })
}
