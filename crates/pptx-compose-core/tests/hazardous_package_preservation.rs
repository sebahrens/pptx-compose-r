use pptx_compose_core::{
    opc::{
        package::Package,
        part_name::PartName,
        relationships::{Relationship, RelationshipSource},
    },
    validation::{FindingCode, Severity, ValidationMode, ValidationStatus, validate_package},
};

const PRESENTATION_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
const SLIDE_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide";
const HYPERLINK_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
const VBA_REL_TYPE: &str = "http://schemas.microsoft.com/office/2006/relationships/vbaProject";
const SIGNATURE_ORIGIN_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/package/2006/relationships/digital-signature/origin";

#[test]
fn preserves_hazardous_parts_and_warns_without_fetching() {
    let mut package = hazardous_fixture_package();

    let external_relationships = package
        .relationships()
        .external_relationships()
        .collect::<Vec<_>>();
    assert_eq!(external_relationships.len(), 1);
    assert_eq!(external_relationships[0].id, "rIdExternal");
    assert_eq!(
        external_relationships[0].target,
        "https://127.0.0.1:9/not-fetched"
    );

    let no_edit = validate_package(&package, ValidationMode::NoEdit);
    assert_eq!(no_edit.status, ValidationStatus::Valid);
    assert_eq!(no_edit.summary.warnings, 1);
    assert_finding(
        &no_edit.findings,
        FindingCode::ExternalRelationshipNotChecked,
    );
    assert_no_finding(&no_edit.findings, FindingCode::SignatureInvalidatedByEdit);
    assert_eq!(
        no_edit.findings[0].location,
        serde_json::json!({
            "relationship_id": "rIdExternal",
            "target": "https://127.0.0.1:9/not-fetched",
            "part": "ppt/slides/slide1.xml"
        })
    );

    assert_original_part_bytes(&package, "ppt/vbaProject.bin", b"opaque-vba-bytes");
    assert_original_part_bytes(&package, "_xmlsignatures/origin.sigs", b"signature-origin");
    assert_original_part_bytes(&package, "_xmlsignatures/sig1.xml", b"<Signature/>");

    package.mark_dirty(part("ppt/presentation.xml"));
    let edited = validate_package(&package, ValidationMode::Edited);
    assert_eq!(edited.status, ValidationStatus::Valid);
    assert_eq!(edited.summary.warnings, 2);
    assert_finding(
        &edited.findings,
        FindingCode::ExternalRelationshipNotChecked,
    );
    let signature_warning =
        assert_finding(&edited.findings, FindingCode::SignatureInvalidatedByEdit);
    assert_eq!(signature_warning.severity, Severity::Warning);
    assert_eq!(
        signature_warning.location,
        serde_json::json!({"dirty_parts": "ppt/presentation.xml"})
    );

    assert_original_part_bytes(&package, "ppt/vbaProject.bin", b"opaque-vba-bytes");
    assert!(!package.dirty_parts().contains(&part("ppt/vbaProject.bin")));
    assert!(
        !package
            .dirty_parts()
            .contains(&part("_xmlsignatures/origin.sigs"))
    );
    assert!(
        !package
            .dirty_parts()
            .contains(&part("_xmlsignatures/sig1.xml"))
    );
}

fn hazardous_fixture_package() -> Package {
    let mut package = Package::new();
    insert(&mut package, "[Content_Types].xml", b"<Types/>");
    insert(&mut package, "_rels/.rels", b"<Relationships/>");
    insert(&mut package, "ppt/presentation.xml", b"<p:presentation/>");
    insert(
        &mut package,
        "ppt/_rels/presentation.xml.rels",
        b"<Relationships/>",
    );
    insert(&mut package, "ppt/slides/slide1.xml", b"<p:sld/>");
    insert(
        &mut package,
        "ppt/slides/_rels/slide1.xml.rels",
        b"<Relationships/>",
    );
    insert(&mut package, "ppt/vbaProject.bin", b"opaque-vba-bytes");
    insert(
        &mut package,
        "_xmlsignatures/origin.sigs",
        b"signature-origin",
    );
    insert(&mut package, "_xmlsignatures/sig1.xml", b"<Signature/>");

    package
        .content_types_mut()
        .insert_default("xml", "application/xml");
    package
        .content_types_mut()
        .insert_default("bin", "application/vnd.ms-office.vbaProject");
    package.content_types_mut().insert_override(
        part("ppt/presentation.xml"),
        "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml",
    );
    package.content_types_mut().insert_override(
        part("_xmlsignatures/origin.sigs"),
        "application/vnd.openxmlformats-package.digital-signature-origin",
    );
    package.content_types_mut().insert_override(
        part("_xmlsignatures/sig1.xml"),
        "application/vnd.openxmlformats-package.digital-signature-xmlsignature+xml",
    );

    package.push_relationship(Relationship::internal(
        RelationshipSource::Package,
        "rIdOfficeDoc",
        PRESENTATION_REL_TYPE,
        "ppt/presentation.xml",
    ));
    package.push_relationship(Relationship::internal(
        RelationshipSource::Package,
        "rIdSignatureOrigin",
        SIGNATURE_ORIGIN_REL_TYPE,
        "_xmlsignatures/origin.sigs",
    ));
    package.push_relationship(Relationship::internal(
        RelationshipSource::Part(part("ppt/presentation.xml")),
        "rIdSlide1",
        SLIDE_REL_TYPE,
        "slides/slide1.xml",
    ));
    package.push_relationship(Relationship::internal(
        RelationshipSource::Part(part("ppt/presentation.xml")),
        "rIdVba",
        VBA_REL_TYPE,
        "vbaProject.bin",
    ));
    package.push_relationship(Relationship::external(
        RelationshipSource::Part(part("ppt/slides/slide1.xml")),
        "rIdExternal",
        HYPERLINK_REL_TYPE,
        "https://127.0.0.1:9/not-fetched",
    ));

    package
}

fn insert(package: &mut Package, name: &str, bytes: &[u8]) {
    package
        .insert_zip_entry(name, bytes.to_vec())
        .expect("fixture part inserts");
}

fn part(name: &str) -> PartName {
    PartName::from_zip_entry(name).expect("valid fixture part name")
}

fn assert_original_part_bytes(package: &Package, name: &str, expected: &[u8]) {
    let part_name = part(name);
    assert!(
        package.original_parts().contains(&part_name),
        "{name} must remain an original package part"
    );
    let actual = package
        .parts()
        .get(&part_name)
        .unwrap_or_else(|| panic!("{name} must be preserved"))
        .bytes();
    assert_eq!(actual, expected, "{name} bytes changed");
}

fn assert_finding(
    findings: &[pptx_compose_core::validation::Finding],
    code: FindingCode,
) -> &pptx_compose_core::validation::Finding {
    findings
        .iter()
        .find(|finding| finding.code == code)
        .unwrap_or_else(|| panic!("{code:?} finding must be present"))
}

fn assert_no_finding(findings: &[pptx_compose_core::validation::Finding], code: FindingCode) {
    assert!(
        findings.iter().all(|finding| finding.code != code),
        "{code:?} finding must be absent"
    );
}
