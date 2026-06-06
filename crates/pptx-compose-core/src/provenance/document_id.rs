use std::collections::BTreeMap;

use crate::{
    opc::part_name::PartName,
    provenance::{
        checksum::part_checksum,
        cpj::{self, Cpj},
    },
};

const DOCUMENT_ID_SCHEMA: &str = "pptx-compose.document_id.v1";
const CONTENT_TYPES_PART_NAME: &str = "/[Content_Types].xml";

#[derive(Clone, Debug, Eq, PartialEq)]
struct ManifestEntry {
    name: String,
    checksum: String,
}

#[must_use]
pub fn document_id(parts: &[(PartName, &[u8])], content_types_bytes: &[u8]) -> String {
    let mut entries = Vec::with_capacity(parts.len() + 1);

    entries.push(manifest_entry(CONTENT_TYPES_PART_NAME, content_types_bytes));

    entries.extend(
        parts
            .iter()
            .map(|(name, bytes)| manifest_entry(name.as_str(), bytes)),
    );

    entries.sort_by(|left, right| left.name.cmp(&right.name));

    let mut preimage = BTreeMap::new();
    preimage.insert(
        "parts".to_owned(),
        Cpj::Array(entries.into_iter().map(ManifestEntry::into_cpj).collect()),
    );
    preimage.insert("schema".to_owned(), Cpj::Str(DOCUMENT_ID_SCHEMA.to_owned()));

    cpj::digest_cpj(&Cpj::Object(preimage))
}

fn manifest_entry(name: &str, bytes: &[u8]) -> ManifestEntry {
    ManifestEntry {
        name: name.to_owned(),
        checksum: part_checksum(bytes),
    }
}

impl ManifestEntry {
    fn into_cpj(self) -> Cpj {
        let mut entry = BTreeMap::new();
        entry.insert("checksum".to_owned(), Cpj::Str(self.checksum));
        entry.insert("name".to_owned(), Cpj::Str(self.name));
        Cpj::Object(entry)
    }
}

#[cfg(test)]
#[test]
fn manifest_stable() {
    let content_types = br#"<?xml version="1.0"?><Types/>"#;
    let presentation = part("/ppt/presentation.xml");
    let slide = part("/ppt/slides/slide1.xml");
    let slide_rels = part("/ppt/slides/_rels/slide1.xml.rels");

    let parts = vec![
        (slide.clone(), b"<p:sld>hello</p:sld>".as_slice()),
        (
            slide_rels.clone(),
            br#"<Relationships><Relationship Id="rId1"/></Relationships>"#.as_slice(),
        ),
        (presentation.clone(), b"<p:presentation/>".as_slice()),
    ];

    let base = document_id(&parts, content_types);
    assert_eq!(
        base,
        "sha256:2610efa71c965b45569609b83e454e27219cad81cee0dc39a6669bde50f07dc8"
    );

    let reordered = vec![
        (presentation, b"<p:presentation/>".as_slice()),
        (
            slide_rels,
            br#"<Relationships><Relationship Id="rId1"/></Relationships>"#.as_slice(),
        ),
        (slide, b"<p:sld>hello</p:sld>".as_slice()),
    ];
    assert_eq!(document_id(&reordered, content_types), base);

    let mutated = vec![
        (
            part("/ppt/slides/slide1.xml"),
            b"<p:sld>changed</p:sld>".as_slice(),
        ),
        (
            part("/ppt/slides/_rels/slide1.xml.rels"),
            br#"<Relationships><Relationship Id="rId1"/></Relationships>"#.as_slice(),
        ),
        (
            part("/ppt/presentation.xml"),
            b"<p:presentation/>".as_slice(),
        ),
    ];
    assert_ne!(document_id(&mutated, content_types), base);

    assert_ne!(
        document_id(&reordered, br#"<?xml version="1.0"?><Types changed="1"/>"#),
        base
    );
}

#[cfg(test)]
fn part(name: &str) -> PartName {
    PartName::from_zip_entry(name).expect("fixture part name is valid")
}
