use std::{error, fmt};

use pptx_compose_core::{
    error::{Error, ErrorCode, Result},
    opc::package::Package,
    validation::{ValidationMode, validate_package},
};
use pptx_compose_json::{
    schema_versions::{PATCH_REPORT_SCHEMA, PATCH_REPORT_VERSION},
    schemas::{PatchReport, PatchStatus, ValidationReport},
};

use crate::reports::{has_blocking_findings, patch_validation_summary, validation_report};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PatchContext {
    pub document_id: String,
    pub base_revision: u32,
    pub new_document_id: String,
    pub new_revision: u32,
    pub dry_run: bool,
}

impl PatchContext {
    #[must_use]
    pub fn new(
        document_id: impl Into<String>,
        base_revision: u32,
        new_document_id: impl Into<String>,
        new_revision: u32,
    ) -> Self {
        Self {
            document_id: document_id.into(),
            base_revision,
            new_document_id: new_document_id.into(),
            new_revision,
            dry_run: false,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PatchEffects {
    pub changed_parts: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ApplyPatchResult {
    pub package: WritablePackage,
    pub report: PatchReport,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WritablePackage {
    package: Package,
}

impl WritablePackage {
    #[must_use]
    pub const fn package(&self) -> &Package {
        &self.package
    }

    #[must_use]
    pub fn into_inner(self) -> Package {
        self.package
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ValidationFailedReport {
    pub report: ValidationReport,
}

impl fmt::Display for ValidationFailedReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "validation failed with {} error(s) and {} fatal finding(s)",
            self.report.summary.errors, self.report.summary.fatal
        )
    }
}

impl error::Error for ValidationFailedReport {}

pub fn apply_patch<F>(package: Package, context: PatchContext, apply: F) -> Result<ApplyPatchResult>
where
    F: FnOnce(&mut Package) -> Result<PatchEffects>,
{
    let mut edited = package;
    let effects = apply(&mut edited)?;
    let validation = validate_for_write(&edited, &context.new_document_id, context.new_revision)?;

    if has_blocking_findings(&validation) {
        return Err(validation_failed(validation));
    }

    Ok(ApplyPatchResult {
        package: WritablePackage { package: edited },
        report: PatchReport {
            schema: PATCH_REPORT_SCHEMA.to_owned(),
            version: PATCH_REPORT_VERSION,
            status: if context.dry_run {
                PatchStatus::DryRunSuccess
            } else {
                PatchStatus::Applied
            },
            dry_run: context.dry_run,
            document_id: context.document_id,
            base_revision: context.base_revision,
            new_document_id: context.new_document_id,
            new_revision: context.new_revision,
            operation_reports: Vec::new(),
            changed_parts: effects.changed_parts,
            validation: patch_validation_summary(&validation),
        },
    })
}

pub fn validate_for_write(
    package: &Package,
    document_id: impl Into<String>,
    revision: u32,
) -> Result<ValidationReport> {
    validation_report(
        validate_package(package, ValidationMode::Edited),
        document_id,
        revision,
    )
}

fn validation_failed(report: ValidationReport) -> Error {
    Error::with_source(
        ErrorCode::ValidationFailed,
        "Edited package failed validation and was not made writable.",
        ValidationFailedReport { report },
    )
    .with_suggestion("Inspect the validation report, fix the invalid edit, and retry the patch.")
}

#[cfg(test)]
#[test]
fn blocks_write_on_invalid() {
    test_support::blocks_write_on_invalid();
}

#[cfg(test)]
mod test_support {
    use pptx_compose_core::{
        error::ErrorCode,
        opc::{
            package::Package,
            part_name::PartName,
            relationships::{Relationship, RelationshipSource},
        },
    };
    use pptx_compose_json::schemas::{FindingCode, ValidationStatus};

    use super::{PatchContext, PatchEffects, apply_patch};

    const IMAGE_REL_TYPE: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";

    pub fn blocks_write_on_invalid() {
        let package = base_package();
        let source = part("ppt/slides/slide1.xml");
        let error = apply_patch(package.clone(), context(), |package| {
            package.push_relationship(Relationship::internal(
                RelationshipSource::Part(source),
                "rId1",
                IMAGE_REL_TYPE,
                "../media/missing.png",
            ));
            Ok(PatchEffects {
                changed_parts: vec!["ppt/slides/_rels/slide1.xml.rels".to_owned()],
            })
        })
        .expect_err("dangling relationship blocks writable package");

        assert_eq!(error.code(), ErrorCode::ValidationFailed);

        let clean = apply_patch(package, context(), |_| Ok(PatchEffects::default()))
            .expect("valid patch returns writable package");

        assert_eq!(clean.report.validation.status, ValidationStatus::Valid);
        assert_eq!(clean.report.validation.errors, 0);
        assert!(!clean.package.package().parts().is_empty());
    }

    #[test]
    fn warning_only_validation_passes_through_report() {
        let mut package = base_package();
        package.push_relationship(Relationship::external(
            RelationshipSource::Part(part("ppt/slides/slide1.xml")),
            "rId2",
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink",
            "https://example.test/",
        ));

        let result = apply_patch(package, context(), |_| Ok(PatchEffects::default()))
            .expect("warning-only validation is writable");

        assert_eq!(result.report.validation.status, ValidationStatus::Valid);
        assert_eq!(result.report.validation.errors, 0);
        assert_eq!(result.report.validation.warnings, 1);
    }

    #[test]
    fn validate_for_write_reports_blocking_findings() {
        let mut package = base_package();
        package.push_relationship(Relationship::internal(
            RelationshipSource::Part(part("ppt/slides/slide1.xml")),
            "rId1",
            IMAGE_REL_TYPE,
            "../media/missing.png",
        ));

        let report = super::validate_for_write(&package, "sha256:test", 1)
            .expect("validation report builds");

        assert_eq!(report.status, ValidationStatus::Invalid);
        assert!(report.findings.iter().any(|finding| {
            finding.code == FindingCode::DanglingInternalRelationship && finding.blocking
        }));
    }

    fn base_package() -> Package {
        let mut package = Package::new();
        package
            .insert_zip_entry("ppt/slides/slide1.xml", b"<p:sld/>".to_vec())
            .expect("slide part inserted");
        package.content_types_mut().insert_override(
            part("ppt/slides/slide1.xml"),
            "application/vnd.openxmlformats-officedocument.presentationml.slide+xml",
        );
        package
    }

    fn context() -> PatchContext {
        PatchContext::new("sha256:old", 1, "sha256:new", 2)
    }

    fn part(name: &str) -> PartName {
        PartName::from_zip_entry(name).expect("valid fixture part name")
    }
}
