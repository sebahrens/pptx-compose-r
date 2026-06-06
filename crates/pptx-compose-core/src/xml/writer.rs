use quick_xml::{
    Writer,
    events::{BytesCData, BytesDecl, BytesEnd, BytesPI, BytesRef, BytesStart, BytesText, Event},
};

use crate::error::{Error, ErrorCode, Result};

use super::document::{XmlAttribute, XmlDocument, XmlElement, XmlNode};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WriteOptions {
    pub mode: WriteMode,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WriteMode {
    #[default]
    Preserve,
    Deterministic,
}

pub fn write_document(doc: &XmlDocument, opts: &WriteOptions) -> Result<Vec<u8>> {
    let mut writer = Writer::new(Vec::new());

    if let Some(declaration) = &doc.declaration {
        let declaration =
            BytesDecl::from_start(BytesStart::from_content(declaration.as_str(), "xml".len()));
        write_event(&mut writer, Event::Decl(declaration))?;
    }

    for node in &doc.nodes {
        write_node(&mut writer, node, opts)?;
    }

    Ok(writer.into_inner())
}

fn write_node(writer: &mut Writer<Vec<u8>>, node: &XmlNode, opts: &WriteOptions) -> Result<()> {
    match node {
        XmlNode::Element(element) => write_element(writer, element, opts),
        XmlNode::Text(text) => write_event(writer, Event::Text(BytesText::new(text))),
        XmlNode::CData(cdata) => {
            for event in BytesCData::escaped(cdata) {
                write_event(writer, Event::CData(event))?;
            }
            Ok(())
        }
        XmlNode::Comment(comment) => {
            write_event(writer, Event::Comment(BytesText::from_escaped(comment)))
        }
        XmlNode::ProcessingInstruction(instruction) => {
            write_event(writer, Event::PI(BytesPI::new(instruction)))
        }
        XmlNode::DocType(doc_type) => {
            write_event(writer, Event::DocType(BytesText::from_escaped(doc_type)))
        }
        XmlNode::GeneralRef(reference) => {
            write_event(writer, Event::GeneralRef(BytesRef::new(reference)))
        }
    }
}

fn write_element(
    writer: &mut Writer<Vec<u8>>,
    element: &XmlElement,
    opts: &WriteOptions,
) -> Result<()> {
    let start = start_event(element, opts.mode == WriteMode::Deterministic);
    if element.children.is_empty() {
        return write_event(writer, Event::Empty(start));
    }

    write_event(writer, Event::Start(start.borrow()))?;
    for child in &element.children {
        write_node(writer, child, opts)?;
    }
    write_event(writer, Event::End(BytesEnd::new(element.name.raw.as_str())))
}

fn start_event(element: &XmlElement, deterministic: bool) -> BytesStart<'static> {
    let mut start = BytesStart::new(element.name.raw.clone());

    for (name, value) in element.namespaces.declaration_attributes(deterministic) {
        start.push_attribute((name.as_str(), value));
    }

    let mut attributes = element
        .attributes
        .iter()
        .filter(|attribute| !attribute.namespace_declaration)
        .collect::<Vec<_>>();

    if deterministic {
        attributes.sort_by(attribute_order);
    }

    for attribute in attributes {
        start.push_attribute((attribute.name.raw.as_str(), attribute.value.as_str()));
    }

    start
}

fn attribute_order(left: &&XmlAttribute, right: &&XmlAttribute) -> std::cmp::Ordering {
    left.name
        .raw
        .cmp(&right.name.raw)
        .then(left.value.cmp(&right.value))
}

fn write_event<'a>(writer: &mut Writer<Vec<u8>>, event: Event<'a>) -> Result<()> {
    writer.write_event(event).map_err(|source| {
        Error::with_source(ErrorCode::WriteFailed, "Could not write XML.", source)
    })
}

#[cfg(test)]
use crate::xml::{
    document::{QualifiedName, XmlPart},
    namespaces::{NamespaceBinding, NamespaceTable},
    parser,
};

#[cfg(test)]
#[test]
fn wellformed_and_deterministic() {
    let raw = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:sld xmlns:cust="urn:custom" xmlns:p="urn:p" cust:flag="yes" data-z="9"><p:cSld/><cust:unknown data-id="42"/></p:sld>"#;
    let parsed = parser::parse_document(raw).expect("input parses");
    let deterministic = WriteOptions {
        mode: WriteMode::Deterministic,
    };

    let first = write_document(&parsed, &deterministic).expect("xml writes");
    let second = write_document(&parsed, &deterministic).expect("xml writes again");
    assert_eq!(first, second);

    let reparsed = parser::parse_document(&first).expect("written XML reparses");
    assert_eq!(reparsed, parsed);

    let serialized = String::from_utf8(first).expect("writer emits UTF-8");
    assert!(serialized.contains(r#"xmlns:cust="urn:custom""#));
    assert!(serialized.contains(r#"xmlns:p="urn:p""#));
    assert!(serialized.contains(r#"cust:flag="yes""#));
    assert!(serialized.contains(r#"<cust:unknown data-id="42"/>"#));

    let escaped_doc = document_with_escaped_text();
    let escaped = write_document(&escaped_doc, &WriteOptions::default()).expect("xml writes");
    assert_eq!(
        String::from_utf8(escaped).expect("writer emits UTF-8"),
        r#"<p:sld xmlns:p="urn:p">A &amp;&lt;&gt;; B</p:sld>"#
    );
}

#[cfg(test)]
#[test]
fn preserves_relationship_xml_semantics() {
    let raw = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Target="slides/slide1.xml" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Id="rId1"/><Relationship TargetMode="External" Target="https://example.test/" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Id="rId2"/></Relationships>"#;
    let parsed = parser::parse_document(raw).expect("relationships parse");
    let written = write_document(
        &parsed,
        &WriteOptions {
            mode: WriteMode::Deterministic,
        },
    )
    .expect("relationships write");

    let mut part = XmlPart::from_raw(written);
    let reparsed = part.parse().expect("relationships reparse");
    let root = reparsed.root_element().expect("root exists");
    assert_eq!(root.name.raw, "Relationships");
    assert_eq!(
        root.namespaces.resolve_prefix(None),
        Some("http://schemas.openxmlformats.org/package/2006/relationships")
    );
    assert!(
        root.children
            .iter()
            .filter_map(XmlNode::as_element)
            .any(|relationship| relationship
                .attributes
                .iter()
                .any(
                    |attribute| attribute.name.raw == "TargetMode" && attribute.value == "External"
                ))
    );
}

#[cfg(test)]
fn document_with_escaped_text() -> XmlDocument {
    let mut namespaces = NamespaceTable::new();
    namespaces.push(NamespaceBinding::prefixed("p", "urn:p"));
    XmlDocument {
        declaration: None,
        nodes: vec![XmlNode::Element(XmlElement {
            name: QualifiedName::from_raw("p:sld"),
            attributes: vec![XmlAttribute {
                name: QualifiedName::from_raw("xmlns:p"),
                value: "urn:p".to_owned(),
                namespace_declaration: true,
            }],
            namespaces,
            children: vec![XmlNode::Text("A &<>; B".to_owned())],
        })],
    }
}
