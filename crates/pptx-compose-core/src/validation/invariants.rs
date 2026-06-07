use std::collections::{BTreeMap, BTreeSet};

use crate::{
    opc::{package::Package, part_name::PartName},
    validation::{Finding, FindingCode, location},
    xml::{
        document::{XmlElement, XmlNode},
        parser::parse_document,
    },
};

pub(crate) const PRODUCER_CHECK_DUPLICATE_SLIDE_ID: &str =
    "validation::invariants::check_duplicate_slide_id";
pub(crate) const PRODUCER_CHECK_DUPLICATE_DRAWING_ID: &str =
    "validation::invariants::check_duplicate_drawing_id";
pub(crate) const PRODUCER_CHECK_MALFORMED_DIRTY_XML: &str =
    "validation::invariants::check_malformed_dirty_xml";
pub(crate) const PRODUCER_CHECK_MISSING_NAMESPACE_DECLARATION: &str =
    "validation::invariants::check_missing_namespace_declaration";
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
    check_xml_well_formed(pkg, findings);
    check_dirty_xml_parts(pkg, findings);
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

fn check_dirty_xml_parts(pkg: &Package, findings: &mut Vec<Finding>) {
    for part_name in pkg.dirty_parts() {
        if !is_xml_part(part_name) {
            continue;
        }
        let Some(part) = pkg.parts().get(part_name) else {
            continue;
        };

        match parse_document(part.bytes()) {
            Ok(document) => {
                if let Some(root) = document.root_element() {
                    check_missing_namespace_declaration(part_name, root, findings);
                    if is_slide_part(pkg, part_name) {
                        check_duplicate_drawing_id(part_name, root, findings);
                    }
                }
            }
            Err(error) => findings.push(Finding::new(
                "",
                FindingCode::MalformedXml,
                format!(
                    "Dirty XML part {part_name} is not well-formed: {}",
                    error.message()
                ),
                false,
                location(&[("part", part_name.zip_entry_name().to_owned())]),
                Some("Regenerate or repair the dirty XML before writing.".to_owned()),
            )),
        }
    }
}

fn check_xml_well_formed(pkg: &Package, findings: &mut Vec<Finding>) {
    for part in pkg.parts().iter() {
        let part_name = part.name();
        if !is_xml_part(part_name) || is_opc_control_part(part_name) {
            continue;
        }

        if let Err(error) = parse_document(part.bytes()) {
            findings.push(Finding::new(
                "",
                FindingCode::MalformedXml,
                format!(
                    "XML part {part_name} is not well-formed: {}",
                    error.message()
                ),
                false,
                location(&[("part", part_name.zip_entry_name().to_owned())]),
                Some("Repair the XML part before treating the package as valid.".to_owned()),
            ));
        }
    }
}

fn is_xml_part(part_name: &PartName) -> bool {
    part_name
        .as_str()
        .rsplit_once('.')
        .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("xml"))
}

fn is_opc_control_part(part_name: &PartName) -> bool {
    let path = part_name.as_str();
    path == "/[Content_Types].xml" || (path.contains("/_rels/") && path.ends_with(".rels"))
}

fn is_slide_part(pkg: &Package, part_name: &PartName) -> bool {
    pkg.slide_ids()
        .iter()
        .any(|slide| slide.part.as_ref() == Some(part_name))
        || {
            let path = part_name.as_str();
            path.starts_with("/ppt/slides/slide") && path.ends_with(".xml")
        }
}

fn check_missing_namespace_declaration(
    part_name: &PartName,
    root: &XmlElement,
    findings: &mut Vec<Finding>,
) {
    let mut reported = BTreeSet::new();
    check_element_namespaces(part_name, root, &BTreeMap::new(), &mut reported, findings);
}

fn check_element_namespaces(
    part_name: &PartName,
    element: &XmlElement,
    inherited: &BTreeMap<String, String>,
    reported: &mut BTreeSet<String>,
    findings: &mut Vec<Finding>,
) {
    let mut in_scope = inherited.clone();
    for binding in element.namespaces.bindings() {
        if let Some(prefix) = &binding.prefix {
            in_scope.insert(prefix.clone(), binding.uri.clone());
        }
    }

    if let Some(prefix) = &element.name.prefix {
        check_prefix(
            part_name,
            prefix,
            &element.name.raw,
            &in_scope,
            reported,
            findings,
        );
    }

    for attribute in &element.attributes {
        if attribute.namespace_declaration {
            continue;
        }
        if let Some(prefix) = &attribute.name.prefix {
            check_prefix(
                part_name,
                prefix,
                &attribute.name.raw,
                &in_scope,
                reported,
                findings,
            );
        }
    }

    for child in element.children.iter().filter_map(XmlNode::as_element) {
        check_element_namespaces(part_name, child, &in_scope, reported, findings);
    }
}

fn check_prefix(
    part_name: &PartName,
    prefix: &str,
    qualified_name: &str,
    in_scope: &BTreeMap<String, String>,
    reported: &mut BTreeSet<String>,
    findings: &mut Vec<Finding>,
) {
    if in_scope.contains_key(prefix) || !reported.insert(prefix.to_owned()) {
        return;
    }

    findings.push(Finding::new(
        "",
        FindingCode::MissingNamespaceDeclaration,
        format!("Dirty XML part {part_name} uses undeclared namespace prefix {prefix}."),
        false,
        location(&[
            ("part", part_name.zip_entry_name().to_owned()),
            ("prefix", prefix.to_owned()),
            ("qualified_name", qualified_name.to_owned()),
        ]),
        Some(
            "Declare the namespace prefix on the element or an ancestor before writing.".to_owned(),
        ),
    ));
}

fn check_duplicate_drawing_id(
    part_name: &PartName,
    root: &XmlElement,
    findings: &mut Vec<Finding>,
) {
    let mut seen = BTreeSet::new();
    let mut reported = BTreeSet::new();
    for sp_tree in descendants_named(root, "spTree") {
        collect_duplicate_cnvpr_ids(sp_tree, part_name, &mut seen, &mut reported, findings);
    }
}

fn collect_duplicate_cnvpr_ids(
    element: &XmlElement,
    part_name: &PartName,
    seen: &mut BTreeSet<String>,
    reported: &mut BTreeSet<String>,
    findings: &mut Vec<Finding>,
) {
    if element.name.local_name == "cNvPr"
        && let Some(id) = attr(element, "id")
        && !seen.insert(id.to_owned())
        && reported.insert(id.to_owned())
    {
        findings.push(Finding::new(
            "",
            FindingCode::DuplicateDrawingId,
            format!("Drawing non-visual id {id} appears more than once in slide {part_name}."),
            false,
            location(&[
                ("part", part_name.zip_entry_name().to_owned()),
                ("drawing_id", id.to_owned()),
            ]),
            Some("Allocate a unique p:cNvPr/@id within the slide shape tree.".to_owned()),
        ));
    }

    for child in element.children.iter().filter_map(XmlNode::as_element) {
        collect_duplicate_cnvpr_ids(child, part_name, seen, reported, findings);
    }
}

fn descendants_named<'a>(element: &'a XmlElement, local_name: &str) -> Vec<&'a XmlElement> {
    let mut descendants = Vec::new();
    collect_descendants_named(element, local_name, &mut descendants);
    descendants
}

fn collect_descendants_named<'a>(
    element: &'a XmlElement,
    local_name: &str,
    descendants: &mut Vec<&'a XmlElement>,
) {
    if element.name.local_name == local_name {
        descendants.push(element);
    }
    for child in element.children.iter().filter_map(XmlNode::as_element) {
        collect_descendants_named(child, local_name, descendants);
    }
}

fn attr<'a>(element: &'a XmlElement, local_name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.local_name == local_name)
        .map(|attribute| attribute.value.as_str())
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

    #[test]
    fn detects_dirty_xml_violations() {
        let slide_part =
            PartName::from_zip_entry("ppt/slides/slide1.xml").expect("valid slide part");
        let mut package = Package::new();
        package
            .insert_zip_entry(
                "ppt/slides/slide1.xml",
                br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:sp><p:nvSpPr><p:cNvPr id="4" name="Title"/></p:nvSpPr><p:spPr><a:xfrm/></p:spPr></p:sp><p:pic><p:nvPicPr><p:cNvPr id="4" name="Picture"/></p:nvPicPr></p:pic></p:spTree></p:cSld></p:sld>"#.to_vec(),
            )
            .expect("slide inserted");
        package
            .content_types_mut()
            .insert_default("xml", "application/xml");
        package.mark_dirty(slide_part.clone());
        package.push_slide_id(SlideIdEntry {
            slide_id: "256".to_owned(),
            relationship_id: Some("rId1".to_owned()),
            part: Some(slide_part),
        });

        let outcome = validate_package(&package, ValidationMode::Edited);

        assert_eq!(outcome.status, ValidationStatus::Invalid);
        let duplicate_drawing = outcome
            .findings
            .iter()
            .find(|finding| finding.code == FindingCode::DuplicateDrawingId)
            .expect("duplicate drawing id finding");
        assert_eq!(
            duplicate_drawing.location,
            serde_json::json!({
                "part": "ppt/slides/slide1.xml",
                "drawing_id": "4"
            })
        );
        let missing_namespace = outcome
            .findings
            .iter()
            .find(|finding| finding.code == FindingCode::MissingNamespaceDeclaration)
            .expect("missing namespace finding");
        assert_eq!(
            missing_namespace.location,
            serde_json::json!({
                "part": "ppt/slides/slide1.xml",
                "prefix": "a",
                "qualified_name": "a:xfrm"
            })
        );
    }

    #[test]
    fn detects_malformed_dirty_xml() {
        let slide_part =
            PartName::from_zip_entry("ppt/slides/slide1.xml").expect("valid slide part");
        let mut package = Package::new();
        package
            .insert_zip_entry(
                "ppt/slides/slide1.xml",
                br#"<p:sld xmlns:p="urn:p"><p:cSld></p:sld>"#.to_vec(),
            )
            .expect("slide inserted");
        package
            .content_types_mut()
            .insert_default("xml", "application/xml");
        package.mark_dirty(slide_part);

        let outcome = validate_package(&package, ValidationMode::Edited);

        assert_eq!(outcome.status, ValidationStatus::Invalid);
        let malformed_xml = outcome
            .findings
            .iter()
            .find(|finding| finding.code == FindingCode::MalformedXml)
            .expect("malformed dirty XML finding");
        assert_eq!(
            malformed_xml.location,
            serde_json::json!({"part": "ppt/slides/slide1.xml"})
        );
        assert_eq!(malformed_xml.severity, crate::validation::Severity::Fatal);
    }

    #[test]
    fn detects_malformed_undirtied_xml() {
        let mut package = Package::new();
        package
            .insert_zip_entry(
                "ppt/slides/slide1.xml",
                br#"<p:sld xmlns:p="urn:p"><p:cSld></p:sld>"#.to_vec(),
            )
            .expect("slide inserted");
        package
            .content_types_mut()
            .insert_default("xml", "application/xml");

        let outcome = validate_package(&package, ValidationMode::NoEdit);

        assert_eq!(outcome.status, ValidationStatus::Invalid);
        assert!(package.dirty_parts().is_empty());
        let malformed_xml = outcome
            .findings
            .iter()
            .find(|finding| finding.code == FindingCode::MalformedXml)
            .expect("malformed undirtied XML finding");
        assert_eq!(
            malformed_xml.location,
            serde_json::json!({"part": "ppt/slides/slide1.xml"})
        );
        assert_eq!(malformed_xml.severity, crate::validation::Severity::Fatal);
    }
}
