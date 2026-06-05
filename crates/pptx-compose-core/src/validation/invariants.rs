use std::collections::{BTreeMap, BTreeSet};

use crate::{
    opc::{
        package::Package,
        part_name::PartName,
        relationships::{TargetMode, resolve_internal_target},
    },
    validation::{Finding, FindingCode, location},
};

pub fn check_invariants(pkg: &Package, findings: &mut Vec<Finding>) {
    check_duplicate_slide_id(pkg, findings);
    check_dangling_internal_relationship(pkg, findings);
    check_duplicate_relationship_id(pkg, findings);
    check_missing_content_type(pkg, findings);
    check_part_dropped(pkg, findings);
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

fn check_dangling_internal_relationship(pkg: &Package, findings: &mut Vec<Finding>) {
    for relationship in pkg.relationships().iter() {
        if relationship.target_mode != TargetMode::Internal {
            continue;
        }

        let resolved = resolve_internal_target(&relationship.source, &relationship.target);
        let missing = resolved
            .as_ref()
            .map_or(true, |part_name| pkg.parts().get(part_name).is_none());

        if missing {
            let mut entries = vec![("relationship_id", relationship.id.clone())];
            if let Some(part) = relationship.source.location_part() {
                entries.push(("part", part.zip_entry_name().to_owned()));
            }
            if let Ok(part_name) = resolved {
                entries.push(("target_part", part_name.zip_entry_name().to_owned()));
            } else {
                entries.push(("target", relationship.target.clone()));
            }

            findings.push(Finding::new(
                "",
                FindingCode::DanglingInternalRelationship,
                format!(
                    "Internal relationship {} points at a missing package part.",
                    relationship.id
                ),
                false,
                location(&entries),
                None,
            ));
        }
    }
}

fn check_duplicate_relationship_id(pkg: &Package, findings: &mut Vec<Finding>) {
    let mut ids_by_source: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for relationship in pkg.relationships().iter() {
        let source = source_key(relationship.source.location_part());
        let ids = ids_by_source.entry(source).or_default();
        if !ids.insert(relationship.id.clone()) {
            let mut entries = vec![("relationship_id", relationship.id.clone())];
            if let Some(part) = relationship.source.location_part() {
                entries.push(("part", part.zip_entry_name().to_owned()));
            }

            findings.push(Finding::new(
                "",
                FindingCode::DuplicateRelationshipId,
                format!(
                    "Relationship id {} appears more than once in one relationship part.",
                    relationship.id
                ),
                false,
                location(&entries),
                None,
            ));
        }
    }
}

fn check_missing_content_type(pkg: &Package, findings: &mut Vec<Finding>) {
    for part in pkg.parts().iter() {
        if is_opc_control_part(part.name()) {
            continue;
        }

        if pkg.content_types().resolve(part.name()).is_none() {
            findings.push(Finding::new(
                "",
                FindingCode::MissingContentType,
                format!("Part {} has no resolved content type.", part.name()),
                false,
                location(&[("part", part.name().zip_entry_name().to_owned())]),
                None,
            ));
        }
    }
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

fn source_key(part_name: Option<&PartName>) -> String {
    part_name.map_or_else(|| "/_rels/.rels".to_owned(), ToString::to_string)
}

fn is_opc_control_part(part_name: &PartName) -> bool {
    let path = part_name.as_str();
    path == "/[Content_Types].xml" || (path.contains("/_rels/") && path.ends_with(".rels"))
}

#[cfg(test)]
#[test]
fn detects_seeded_violations() {
    use crate::{
        opc::{
            package::{Package, SlideIdEntry},
            part_name::PartName,
            relationships::{Relationship, RelationshipSource},
        },
        validation::{FindingCode, ValidationMode, ValidationStatus, validate_package},
    };

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
    assert_eq!(outcome.findings[0].code, FindingCode::DuplicateSlideId);
    assert_eq!(
        outcome.findings[0].location,
        serde_json::json!({"slide_id": "256"})
    );
    assert_eq!(
        outcome.findings[1].code,
        FindingCode::DanglingInternalRelationship
    );
    assert_eq!(
        outcome.findings[1].location,
        serde_json::json!({
            "relationship_id": "rId1",
            "part": "ppt/presentation.xml",
            "target_part": "ppt/slides/slide1.xml"
        })
    );
    assert_eq!(outcome.summary.errors, 2);
}
