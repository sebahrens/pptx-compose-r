use std::collections::BTreeSet;

use crate::{
    opc::{package::Package, part_name::PartName},
    validation::{Finding, FindingCode, location},
};

pub(crate) const PRODUCER_CHECK_DUPLICATE_SLIDE_ID: &str =
    "validation::invariants::check_duplicate_slide_id";
pub(crate) const PRODUCER_CHECK_PART_DROPPED: &str = "validation::invariants::check_part_dropped";
pub(crate) const PRODUCER_CHECK_EXTERNAL_RELATIONSHIP_NOT_CHECKED: &str =
    "validation::invariants::check_external_relationship_not_checked";
pub(crate) const PRODUCER_CHECK_SIGNATURE_INVALIDATED_BY_EDIT: &str =
    "validation::invariants::check_signature_invalidated_by_edit";

const DIGITAL_SIGNATURE_REL_PREFIX: &str =
    "http://schemas.openxmlformats.org/package/2006/relationships/digital-signature/";
const DIGITAL_SIGNATURE_ORIGIN_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-package.digital-signature-origin";
const DIGITAL_SIGNATURE_XML_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-package.digital-signature-xmlsignature+xml";

pub fn check_invariants(pkg: &Package, findings: &mut Vec<Finding>) {
    check_duplicate_slide_id(pkg, findings);
    check_external_relationship_not_checked(pkg, findings);
    check_part_dropped(pkg, findings);
    check_signature_invalidated_by_edit(pkg, findings);
}

fn check_duplicate_slide_id(pkg: &Package, findings: &mut Vec<Finding>) {
    let mut seen = BTreeSet::new();

    for slide in pkg.slide_ids() {
        if !seen.insert(slide.slide_id.clone()) {
            let mut entries = vec![("slide_id", slide.slide_id.clone())];
            if let Some(part) = &slide.part {
                entries.push(("part", part.zip_entry_name().to_owned()));
            }
            if let Some(relationship_id) = &slide.relationship_id {
                entries.push(("relationship_id", relationship_id.clone()));
            }

            findings.push(Finding::new(
                "",
                FindingCode::DuplicateSlideId,
                format!("Slide id {} appears more than once.", slide.slide_id),
                false,
                location(&entries),
                None,
            ));
        }
    }
}

fn check_external_relationship_not_checked(pkg: &Package, findings: &mut Vec<Finding>) {
    for relationship in pkg.relationships().external_relationships() {
        let mut entries = vec![
            ("relationship_id", relationship.id.to_owned()),
            ("target", relationship.target.to_owned()),
        ];
        if let Some(part) = relationship.source.location_part() {
            entries.push(("part", part.zip_entry_name().to_owned()));
        }

        findings.push(Finding::new(
            "",
            FindingCode::ExternalRelationshipNotChecked,
            format!(
                "External relationship {} was preserved but not fetched.",
                relationship.id
            ),
            false,
            location(&entries),
            None,
        ));
    }
}

fn check_signature_invalidated_by_edit(pkg: &Package, findings: &mut Vec<Finding>) {
    if pkg.dirty_parts().is_empty() || !has_digital_signature(pkg) {
        return;
    }

    let dirty_parts = pkg
        .dirty_parts()
        .iter()
        .map(PartName::zip_entry_name)
        .collect::<Vec<_>>()
        .join(",");

    findings.push(Finding::new(
        "",
        FindingCode::SignatureInvalidatedByEdit,
        "A mutating edit to this signed package invalidates its digital signature.",
        false,
        location(&[("dirty_parts", dirty_parts)]),
        Some("Warn the caller that existing digital signatures no longer validate.".to_owned()),
    ));
}

fn has_digital_signature(pkg: &Package) -> bool {
    pkg.relationships().iter().any(|relationship| {
        relationship
            .rel_type
            .starts_with(DIGITAL_SIGNATURE_REL_PREFIX)
    }) || pkg.parts().iter().any(|part| {
        matches!(
            pkg.content_types().resolve(part.name()),
            Some(DIGITAL_SIGNATURE_ORIGIN_CONTENT_TYPE | DIGITAL_SIGNATURE_XML_CONTENT_TYPE)
        )
    })
}

fn check_part_dropped(pkg: &Package, findings: &mut Vec<Finding>) {
    for original_part in pkg.original_parts() {
        if pkg.parts().get(original_part).is_some() {
            continue;
        }

        findings.push(Finding::new(
            "",
            FindingCode::PartDropped,
            format!("Original part {original_part} is missing from the package."),
            false,
            location(&[("part", original_part.zip_entry_name().to_owned())]),
            None,
        ));
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        opc::{
            package::{Package, SlideIdEntry},
            part_name::PartName,
            relationships::{Relationship, RelationshipSource},
        },
        validation::{FindingCode, ValidationMode, ValidationStatus, validate_package},
    };

    #[test]
    fn detects_seeded_violations() {
        let mut package = Package::new();
        package
            .insert_zip_entry("ppt/presentation.xml", Vec::new())
            .expect("presentation part inserted");
        package
            .content_types_mut()
            .insert_default("xml", "application/xml");
        package.push_slide_id(SlideIdEntry::new("256"));
        package.push_slide_id(SlideIdEntry::new("256"));
        package.push_relationship(Relationship::internal(
            RelationshipSource::Part(
                PartName::from_zip_entry("ppt/presentation.xml").expect("valid part name"),
            ),
            "rId1",
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide",
            "slides/slide1.xml",
        ));

        let outcome = validate_package(&package, ValidationMode::Edited);

        assert_eq!(outcome.status, ValidationStatus::Invalid);
        assert_eq!(outcome.findings.len(), 2);
        let duplicate_slide = outcome
            .findings
            .iter()
            .find(|finding| finding.code == FindingCode::DuplicateSlideId)
            .expect("duplicate slide finding");
        assert_eq!(
            duplicate_slide.location,
            serde_json::json!({"slide_id": "256"})
        );
        let dangling_relationship = outcome
            .findings
            .iter()
            .find(|finding| finding.code == FindingCode::DanglingInternalRelationship)
            .expect("dangling relationship finding");
        assert_eq!(
            dangling_relationship.location,
            serde_json::json!({
                "relationship_id": "rId1",
                "part": "ppt/presentation.xml",
                "target_part": "ppt/slides/slide1.xml"
            })
        );
        assert_eq!(outcome.summary.errors, 2);
    }
}
