use std::collections::{BTreeMap, BTreeSet};

use crate::{
    opc::{
        package::Package,
        part_name::PartName,
        relationships::{TargetMode, resolve_internal_target},
    },
    validation::{Finding, FindingCode, location},
    xml::{
        document::{XmlElement, XmlNode},
        parser::parse_document,
    },
};

pub(crate) const PRODUCER_CHECK_DANGLING_INTERNAL_RELATIONSHIP: &str =
    "validation::package_graph::check_dangling_internal_relationship";
pub(crate) const PRODUCER_CHECK_DUPLICATE_RELATIONSHIP_ID: &str =
    "validation::package_graph::check_duplicate_relationship_id";
pub(crate) const PRODUCER_CHECK_MISSING_CONTENT_TYPE: &str =
    "validation::package_graph::check_missing_content_type";
pub(crate) const PRODUCER_CHECK_MEDIA_CONTENT_TYPE_MISMATCH: &str =
    "validation::package_graph::check_media_content_type_mismatch";
pub(crate) const PRODUCER_CHECK_UNRESOLVED_RELATIONSHIP_REFERENCE: &str =
    "validation::package_graph::check_unresolved_relationship_reference";

#[must_use]
pub fn check_package_graph(pkg: &Package) -> Vec<Finding> {
    let mut findings = Vec::new();
    check_dangling_internal_relationship(pkg, &mut findings);
    check_duplicate_relationship_id(pkg, &mut findings);
    check_missing_content_type(pkg, &mut findings);
    check_media_content_type_mismatch(pkg, &mut findings);
    check_unresolved_relationship_reference(pkg, &mut findings);
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

fn check_media_content_type_mismatch(pkg: &Package, findings: &mut Vec<Finding>) {
    for part in pkg.parts().iter() {
        if !is_media_part(part.name()) {
            continue;
        }

        let Some(sniffed) = sniff_media_content_type(part.bytes()) else {
            continue;
        };
        let Some(declared) = pkg.content_types().resolve(part.name()) else {
            continue;
        };

        if declared != sniffed {
            findings.push(Finding::new(
                "",
                FindingCode::MediaContentTypeMismatch,
                format!(
                    "Media part {} declares content type {declared} but bytes sniff as {sniffed}.",
                    part.name()
                ),
                false,
                location(&[
                    ("part", part.name().zip_entry_name().to_owned()),
                    ("declared_content_type", declared.to_owned()),
                    ("sniffed_content_type", sniffed.to_owned()),
                ]),
                Some("Fix the content type declaration or replace the media bytes.".to_owned()),
            ));
        }
    }
}

fn check_unresolved_relationship_reference(pkg: &Package, findings: &mut Vec<Finding>) {
    for part in pkg.parts().iter() {
        if !is_xml_part(part.name()) || is_opc_control_part(part.name()) {
            continue;
        }

        let Ok(document) = parse_document(part.bytes()) else {
            continue;
        };
        let Some(root) = document.root_element() else {
            continue;
        };
        let mut references = BTreeSet::new();
        collect_relationship_references(root, &mut references);

        for relationship_id in references {
            let resolved = pkg
                .relationships()
                .set_for(part.name())
                .is_some_and(|set| set.get(&relationship_id).is_some());
            if !resolved {
                findings.push(Finding::new(
                    "",
                    FindingCode::UnresolvedRelationshipReference,
                    format!(
                        "XML part {} references relationship id {relationship_id} that is missing from its relationship part.",
                        part.name()
                    ),
                    false,
                    location(&[
                        ("part", part.name().zip_entry_name().to_owned()),
                        ("relationship_id", relationship_id),
                    ]),
                    Some("Add the referenced relationship or remove the stale XML reference.".to_owned()),
                ));
            }
        }
    }
}

fn collect_relationship_references(element: &XmlElement, references: &mut BTreeSet<String>) {
    for attribute in &element.attributes {
        if attribute.namespace_declaration {
            continue;
        }
        if is_relationship_reference_attribute(element, &attribute.name.raw) {
            references.insert(attribute.value.clone());
        }
    }

    for child in element.children.iter().filter_map(XmlNode::as_element) {
        collect_relationship_references(child, references);
    }
}

fn is_relationship_reference_attribute(_element: &XmlElement, raw_name: &str) -> bool {
    matches!(raw_name, "r:id" | "r:embed" | "r:link" | "rId")
}

fn source_key(part_name: Option<&PartName>) -> String {
    part_name.map_or_else(|| "/_rels/.rels".to_owned(), ToString::to_string)
}

fn is_opc_control_part(part_name: &PartName) -> bool {
    let path = part_name.as_str();
    path == "/[Content_Types].xml" || (path.contains("/_rels/") && path.ends_with(".rels"))
}

fn is_xml_part(part_name: &PartName) -> bool {
    part_name
        .as_str()
        .rsplit_once('.')
        .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("xml"))
}

fn is_media_part(part_name: &PartName) -> bool {
    part_name.as_str().starts_with("/ppt/media/")
}

fn sniff_media_content_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF8") {
        Some("image/gif")
    } else {
        None
    }
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

#[test]
fn detects_media_content_type_mismatch() {
    use crate::{
        opc::{package::Package, part_name::PartName},
        validation::{FindingCode, package_graph::check_package_graph},
    };

    let mut package = Package::new();
    package
        .insert_zip_entry("ppt/media/image1.png", b"\xff\xd8\xff\xe0payload".to_vec())
        .expect("media part inserted");
    package
        .content_types_mut()
        .insert_default("png", "image/png");

    let findings = check_package_graph(&package);
    let finding = findings
        .iter()
        .find(|finding| finding.code == FindingCode::MediaContentTypeMismatch)
        .expect("mismatched media content type is reported");

    assert_eq!(
        finding.location["part"],
        PartName::from_zip_entry("ppt/media/image1.png")
            .expect("valid part")
            .zip_entry_name()
    );
    assert_eq!(finding.location["declared_content_type"], "image/png");
    assert_eq!(finding.location["sniffed_content_type"], "image/jpeg");
}

#[test]
fn detects_unresolved_relationship_reference() {
    use crate::{
        opc::{
            package::Package,
            part_name::PartName,
            relationships::{Relationship, RelationshipSource},
        },
        validation::{FindingCode, package_graph::check_package_graph},
    };

    let slide_part = PartName::from_zip_entry("ppt/slides/slide1.xml").expect("valid slide part");
    let mut package = Package::new();
    package
        .insert_zip_entry(
            "ppt/slides/slide1.xml",
            br#"<p:sld xmlns:p="p" xmlns:r="r"><p:pic r:embed="rIdMissing"/><p:pic r:embed="rId1"/></p:sld>"#.to_vec(),
        )
        .expect("slide part inserted");
    package.content_types_mut().insert_override(
        slide_part.clone(),
        "application/vnd.openxmlformats-officedocument.presentationml.slide+xml",
    );
    package.push_relationship(Relationship::internal(
        RelationshipSource::Part(slide_part),
        "rId1",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image",
        "../media/image1.png",
    ));

    let findings = check_package_graph(&package);

    assert_eq!(
        findings
            .iter()
            .filter(|finding| finding.code == FindingCode::UnresolvedRelationshipReference)
            .count(),
        1
    );
    let finding = findings
        .iter()
        .find(|finding| finding.code == FindingCode::UnresolvedRelationshipReference)
        .expect("missing relationship reference is reported");
    assert_eq!(finding.location["relationship_id"], "rIdMissing");
}
