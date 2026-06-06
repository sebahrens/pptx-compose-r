use std::collections::{BTreeMap, BTreeSet};

use crate::{
    opc::{
        package::Package,
        part_name::PartName,
        relationships::{TargetMode, resolve_internal_target},
    },
    validation::{Finding, FindingCode, location},
};

pub(crate) const PRODUCER_CHECK_DANGLING_INTERNAL_RELATIONSHIP: &str =
    "validation::package_graph::check_dangling_internal_relationship";
pub(crate) const PRODUCER_CHECK_DUPLICATE_RELATIONSHIP_ID: &str =
    "validation::package_graph::check_duplicate_relationship_id";
pub(crate) const PRODUCER_CHECK_MISSING_CONTENT_TYPE: &str =
    "validation::package_graph::check_missing_content_type";

#[must_use]
pub fn check_package_graph(pkg: &Package) -> Vec<Finding> {
    let mut findings = Vec::new();
    check_dangling_internal_relationship(pkg, &mut findings);
    check_duplicate_relationship_id(pkg, &mut findings);
    check_missing_content_type(pkg, &mut findings);
    findings
}

fn check_dangling_internal_relationship(pkg: &Package, findings: &mut Vec<Finding>) {
    for relationship in pkg.relationships().iter() {
        if relationship.target_mode != TargetMode::Internal {
            continue;
        }

        let resolved = relationship
            .resolved_target
            .clone()
            .map(Ok)
            .unwrap_or_else(|| resolve_internal_target(&relationship.source, &relationship.target));
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

fn source_key(part_name: Option<&PartName>) -> String {
    part_name.map_or_else(|| "/_rels/.rels".to_owned(), ToString::to_string)
}

fn is_opc_control_part(part_name: &PartName) -> bool {
    let path = part_name.as_str();
    path == "/[Content_Types].xml" || (path.contains("/_rels/") && path.ends_with(".rels"))
}

#[test]
fn detects_violations() {
    use crate::{
        opc::{
            package::Package,
            part_name::PartName,
            relationships::{Relationship, RelationshipSource},
        },
        validation::{FindingCode, package_graph::check_package_graph},
    };

    let mut package = Package::new();
    package
        .insert_zip_entry("ppt/presentation.xml", Vec::new())
        .expect("presentation part inserted");
    package
        .insert_zip_entry("ppt/media/image1.png", Vec::new())
        .expect("image part inserted");
    package
        .insert_zip_entry("ppt/embeddings/object1.bin", Vec::new())
        .expect("untyped part inserted");
    package.content_types_mut().insert_override(
        PartName::from_zip_entry("ppt/presentation.xml").expect("valid part name"),
        "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml",
    );
    package
        .content_types_mut()
        .insert_default("png", "image/png");

    let source = RelationshipSource::Part(
        PartName::from_zip_entry("ppt/presentation.xml").expect("valid part name"),
    );
    package.push_relationship(Relationship::internal(
        source.clone(),
        "rId1",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image",
        "media/image1.png",
    ));
    package.push_relationship(Relationship::internal(
        source.clone(),
        "rId2",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide",
        "slides/slide99.xml",
    ));
    package.push_relationship(Relationship::internal(
        source,
        "rId2",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image",
        "media/image1.png",
    ));

    let findings = check_package_graph(&package);
    assert_eq!(findings.len(), 3);
    assert_eq!(
        findings
            .iter()
            .filter(|finding| finding.code == FindingCode::DanglingInternalRelationship)
            .count(),
        1
    );
    assert_eq!(
        findings
            .iter()
            .filter(|finding| finding.code == FindingCode::DuplicateRelationshipId)
            .count(),
        1
    );
    assert_eq!(
        findings
            .iter()
            .filter(|finding| finding.code == FindingCode::MissingContentType)
            .count(),
        1
    );

    let mut clean = Package::new();
    clean
        .insert_zip_entry("ppt/presentation.xml", Vec::new())
        .expect("presentation part inserted");
    clean
        .insert_zip_entry("ppt/media/image1.png", Vec::new())
        .expect("image part inserted");
    clean.content_types_mut().insert_override(
        PartName::from_zip_entry("ppt/presentation.xml").expect("valid part name"),
        "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml",
    );
    clean.content_types_mut().insert_default("png", "image/png");
    clean.push_relationship(Relationship::internal(
        RelationshipSource::Part(
            PartName::from_zip_entry("ppt/presentation.xml").expect("valid part name"),
        ),
        "rId1",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image",
        "media/image1.png",
    ));

    assert!(check_package_graph(&clean).is_empty());
}
