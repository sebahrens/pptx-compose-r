use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use pptx_compose_core::{
    error::{Error, Result},
    opc::{
        package::Package,
        part_name::PartName,
        relationships::{Relationship, RelationshipSource},
    },
    pptx::{
        ids::{
            ElementKind, SpTreePath, agent_element_id, index_sp_tree, paragraph_agent_id,
            run_agent_id,
        },
        presentation::PresentationDocument,
        shape::read_shape,
        text::read_text_body,
    },
    provenance::{
        checksum::part_checksum,
        document_id::document_id,
        fingerprint::{FingerprintInput, fingerprint},
        text_hash::text_hash,
    },
    xml::{
        document::{XmlElement, XmlNode},
        parser::parse_document,
    },
    zip::{
        reader::{RawEntry, from_bytes},
        writer::{DirtyEntry, WriteEntry, write_vec},
    },
};

const CONTENT_TYPES_PART: &str = "/[Content_Types].xml";
const SLIDE_PART: &str = "/ppt/slides/slide1.xml";

#[test]
fn no_edit_stability() -> Result<()> {
    let input = read_minimal_fixture()?;
    let original_entries = from_bytes(&input)?;
    let original = provenance_snapshot(&original_entries)?;

    let write_entries: Vec<_> = original_entries.iter().map(WriteEntry::Clean).collect();
    let output = write_vec(&input, &write_entries)?;
    let written_entries = from_bytes(&output)?;
    let written = provenance_snapshot(&written_entries)?;

    assert_eq!(written.document_id, original.document_id);
    assert_eq!(written.part_checksums, original.part_checksums);
    assert_eq!(written.agent_ids, original.agent_ids);
    assert_eq!(written.fingerprints, original.fingerprints);
    assert_eq!(written.text_hashes, original.text_hashes);

    let reopened = provenance_snapshot(&from_bytes(&output)?)?;
    assert_eq!(reopened.document_id, original.document_id);

    Ok(())
}

#[test]
fn cross_run_determinism() -> Result<()> {
    let input = read_minimal_fixture()?;

    let first = provenance_snapshot(&from_bytes(&input)?)?;
    let second = provenance_snapshot(&from_bytes(&input)?)?;

    assert_eq!(second.document_id, first.document_id);
    assert_eq!(second.part_checksums, first.part_checksums);
    assert_eq!(second.agent_ids, first.agent_ids);
    assert_eq!(second.fingerprints, first.fingerprints);
    assert_eq!(second.text_hashes, first.text_hashes);

    Ok(())
}

#[test]
fn edit_locality() -> Result<()> {
    let input = read_minimal_fixture()?;
    let original_entries = from_bytes(&input)?;
    let original = provenance_snapshot(&original_entries)?;

    let slide_entry = original_entries
        .iter()
        .find(|entry| entry.name.as_str() == SLIDE_PART)
        .ok_or_else(|| Error::unsupported_package("Minimal fixture is missing slide1.xml."))?;
    let dirty_slide = dirty_slide_bytes(&slide_entry.bytes)?;

    let write_entries: Vec<_> = original_entries
        .iter()
        .map(|entry| {
            if entry.name.as_str() == SLIDE_PART {
                WriteEntry::Dirty(DirtyEntry {
                    name: entry.meta.original_name.as_str(),
                    bytes: dirty_slide.as_slice(),
                    meta: &entry.meta,
                })
            } else {
                WriteEntry::Clean(entry)
            }
        })
        .collect();
    let output = write_vec(&input, &write_entries)?;
    let edited = provenance_snapshot(&from_bytes(&output)?)?;

    assert_ne!(edited.document_id, original.document_id);

    let changed_parts: Vec<_> = original
        .part_checksums
        .iter()
        .filter_map(|(name, checksum)| {
            (edited.part_checksums.get(name) != Some(checksum)).then_some(name.as_str())
        })
        .collect();
    assert_eq!(changed_parts, vec![SLIDE_PART]);

    assert_eq!(
        edited.agent_ids, original.agent_ids,
        "direct dirty-part substitution must not change structural agent IDs"
    );

    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProvenanceSnapshot {
    document_id: String,
    part_checksums: BTreeMap<String, String>,
    agent_ids: Vec<String>,
    fingerprints: Vec<String>,
    text_hashes: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ElementSnapshot {
    agent_id: String,
    fingerprint: String,
    text_hash: Option<String>,
    paragraph_ids: Vec<String>,
    run_ids: Vec<String>,
}

fn provenance_snapshot(entries: &[RawEntry]) -> Result<ProvenanceSnapshot> {
    let part_checksums = entries
        .iter()
        .map(|entry| (entry.name.as_str().to_owned(), part_checksum(&entry.bytes)))
        .collect::<BTreeMap<_, _>>();
    let document_id = snapshot_document_id(entries)?;
    let package = package_from_entries(entries)?;
    let presentation = PresentationDocument::open(package)?;
    let element_snapshots = element_snapshots(&presentation)?;

    Ok(ProvenanceSnapshot {
        document_id,
        part_checksums,
        agent_ids: agent_ids(&presentation, &element_snapshots),
        fingerprints: element_snapshots
            .iter()
            .map(|element| element.fingerprint.clone())
            .collect(),
        text_hashes: element_snapshots
            .iter()
            .filter_map(|element| element.text_hash.clone())
            .collect(),
    })
}

fn snapshot_document_id(entries: &[RawEntry]) -> Result<String> {
    let content_types = entries
        .iter()
        .find(|entry| entry.name.as_str() == CONTENT_TYPES_PART)
        .ok_or_else(|| Error::unsupported_package("Package is missing [Content_Types].xml."))?;
    let parts = entries
        .iter()
        .filter(|entry| entry.name.as_str() != CONTENT_TYPES_PART)
        .map(|entry| (entry.name.clone(), entry.bytes.as_slice()))
        .collect::<Vec<_>>();

    Ok(document_id(&parts, &content_types.bytes))
}

fn agent_ids(
    presentation: &PresentationDocument,
    element_snapshots: &[ElementSnapshot],
) -> Vec<String> {
    let mut ids = presentation
        .slides()
        .iter()
        .map(|slide| slide.agent_id())
        .collect::<Vec<_>>();
    for element in element_snapshots {
        ids.push(element.agent_id.clone());
        ids.extend(element.paragraph_ids.iter().cloned());
        ids.extend(element.run_ids.iter().cloned());
    }
    ids
}

fn element_snapshots(presentation: &PresentationDocument) -> Result<Vec<ElementSnapshot>> {
    let mut snapshots = Vec::new();
    for slide in presentation.slides() {
        let part = presentation
            .package()
            .parts()
            .get(&slide.part_name)
            .ok_or_else(|| {
                Error::unsupported_package(format!("Slide part {} is missing.", slide.part_name))
            })?;
        let document = parse_document(part.bytes())?;
        let root = document.root_element().ok_or_else(|| {
            Error::unsupported_package(format!(
                "Slide part {} has no root element.",
                slide.part_name
            ))
        })?;
        let sp_tree = first_descendant(root, "spTree").ok_or_else(|| {
            Error::unsupported_package(format!(
                "Slide part {} is missing p:spTree.",
                slide.part_name
            ))
        })?;
        let slide_id = slide.agent_id();

        for (path, kind) in index_sp_tree(sp_tree) {
            let Some(element) = element_at_path(sp_tree, &path.sp_tree_path) else {
                continue;
            };
            snapshots.push(element_snapshot(
                &slide_id,
                &slide.part_name,
                element,
                path,
                kind,
            ));
        }
    }

    Ok(snapshots)
}

fn element_snapshot(
    slide_id: &str,
    part_name: &PartName,
    element: &XmlElement,
    path: SpTreePath,
    kind: ElementKind,
) -> ElementSnapshot {
    let shape = read_shape(element, path.clone());
    let text_body = first_descendant(element, "txBody").map(read_text_body);
    let element_id = agent_element_id(slide_id, kind, shape.cnvpr_id, &path);
    let text_hash = text_body
        .as_ref()
        .map(|text_body| text_hash(&text_body.normalized));
    let fingerprint = fingerprint(&FingerprintInput {
        kind,
        part: part_name.clone(),
        sp_tree_path: path.sp_tree_path,
        group_path: path.group_path,
        cnvpr_id: shape.cnvpr_id,
        text_hash: text_hash.clone(),
    });
    let mut paragraph_ids = Vec::new();
    let mut run_ids = Vec::new();
    if let Some(text_body) = text_body {
        for paragraph in text_body.paragraphs {
            let paragraph_id = paragraph_agent_id(&element_id, paragraph.index as usize);
            for run in paragraph.runs {
                run_ids.push(run_agent_id(&paragraph_id, run.index as usize));
            }
            paragraph_ids.push(paragraph_id);
        }
    }

    ElementSnapshot {
        agent_id: element_id,
        fingerprint,
        text_hash,
        paragraph_ids,
        run_ids,
    }
}

fn package_from_entries(entries: &[RawEntry]) -> Result<Package> {
    let mut package = Package::new();
    for entry in entries {
        package.insert_part(pptx_compose_core::opc::part::Part::from_zip_entry(
            entry.meta.original_name.clone(),
            entry.bytes.clone(),
        )?)?;
    }

    hydrate_relationships(&mut package)?;
    Ok(package)
}

fn hydrate_relationships(package: &mut Package) -> Result<()> {
    let rels_entries = package
        .parts()
        .iter()
        .filter(|part| part.name().as_str().ends_with(".rels"))
        .map(|part| (part.name().clone(), part.bytes().to_vec()))
        .collect::<Vec<_>>();

    for (rels_part_name, raw) in rels_entries {
        let source = relationship_source_for(&rels_part_name)?;
        let document = parse_document(&raw)?;
        let root = document
            .root_element()
            .ok_or_else(|| Error::unsupported_package(".rels part has no root element."))?;

        for child in root.children.iter().filter_map(XmlNode::as_element) {
            if child.name.local_name != "Relationship" {
                continue;
            }

            let id = required_attr(child, "Id")?;
            let rel_type = required_attr(child, "Type")?;
            let target = required_attr(child, "Target")?;
            let relationship = if optional_attr(child, "TargetMode") == Some("External") {
                Relationship::external(source.clone(), id, rel_type, target)
            } else {
                Relationship::internal(source.clone(), id, rel_type, target)
            };
            package.push_relationship(relationship);
        }
    }

    Ok(())
}

fn relationship_source_for(rels_part_name: &PartName) -> Result<RelationshipSource> {
    let rels_path = rels_part_name.as_str();
    if rels_path == "/_rels/.rels" {
        return Ok(RelationshipSource::Package);
    }

    let Some((directory, file_name)) = rels_path.rsplit_once("/_rels/") else {
        return Err(Error::unsupported_package(format!(
            "Relationship part {rels_part_name} is not in an _rels directory."
        )));
    };
    let Some(source_file_name) = file_name.strip_suffix(".rels") else {
        return Err(Error::unsupported_package(format!(
            "Relationship part {rels_part_name} does not end with .rels."
        )));
    };

    PartName::from_zip_entry(format!("{directory}/{source_file_name}").as_str())
        .map(RelationshipSource::Part)
}

fn first_descendant<'a>(element: &'a XmlElement, local_name: &str) -> Option<&'a XmlElement> {
    for child in element.children.iter().filter_map(XmlNode::as_element) {
        if child.name.local_name == local_name {
            return Some(child);
        }
        if let Some(descendant) = first_descendant(child, local_name) {
            return Some(descendant);
        }
    }
    None
}

fn element_at_path<'a>(sp_tree: &'a XmlElement, path: &[u32]) -> Option<&'a XmlElement> {
    let (first, rest) = path.split_first()?;
    let zero_based = usize::try_from(first.checked_sub(1)?).ok()?;
    let child = sp_tree
        .children
        .iter()
        .filter_map(XmlNode::as_element)
        .nth(zero_based)?;
    if rest.is_empty() {
        Some(child)
    } else {
        element_at_path(child, rest)
    }
}

fn required_attr<'a>(element: &'a XmlElement, name: &str) -> Result<&'a str> {
    optional_attr(element, name).ok_or_else(|| {
        Error::unsupported_package(format!(
            "Element {} is missing required attribute {name}.",
            element.name.raw
        ))
    })
}

fn optional_attr<'a>(element: &'a XmlElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.local_name == name)
        .map(|attribute| attribute.value.as_str())
}

fn dirty_slide_bytes(raw: &[u8]) -> Result<Vec<u8>> {
    let needle = br#"name="""#;
    let Some(position) = raw
        .windows(needle.len())
        .position(|window| window == needle)
    else {
        return Err(Error::unsupported_package(
            "Minimal fixture slide does not contain the expected cNvPr name attribute.",
        ));
    };

    let mut dirty = Vec::with_capacity(raw.len() + "edited".len());
    let insert_at = position + needle.len() - 1;
    dirty.extend_from_slice(&raw[..insert_at]);
    dirty.extend_from_slice(b"edited");
    dirty.extend_from_slice(&raw[insert_at..]);
    parse_document(&dirty)?;
    Ok(dirty)
}

fn read_minimal_fixture() -> Result<Vec<u8>> {
    std::fs::read(minimal_fixture()).map_err(|source| {
        Error::parse_error(
            "Could not read fixtures/minimal.pptx for provenance tests.",
            source,
        )
    })
}

fn minimal_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/minimal.pptx")
}
