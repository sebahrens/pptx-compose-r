use std::io::Cursor;

use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};

use crate::error::{Error, Result};

use super::{
    document::{QualifiedName, XmlAttribute, XmlDocument, XmlElement, XmlNode},
    namespaces::{NamespaceBinding, NamespaceTable},
};

pub fn parse_document(raw: &[u8]) -> Result<XmlDocument> {
    let mut reader = Reader::from_reader(Cursor::new(raw));
    reader.config_mut().trim_text(false);

    let mut buffer = Vec::new();
    let mut declaration = None;
    let mut roots = Vec::new();
    let mut stack = Vec::new();

    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => stack.push(element_from_start(&event)?),
            Ok(Event::Empty(event)) => push_node(
                &mut roots,
                &mut stack,
                XmlNode::Element(element_from_start(&event)?),
            ),
            Ok(Event::End(_)) => {
                let Some(element) = stack.pop() else {
                    return Err(Error::unsupported_package(
                        "XML end tag encountered without a matching start tag.",
                    ));
                };
                push_node(&mut roots, &mut stack, XmlNode::Element(element));
            }
            Ok(Event::Text(event)) => push_node(
                &mut roots,
                &mut stack,
                XmlNode::Text(decode_bytes(event.as_ref())),
            ),
            Ok(Event::CData(event)) => push_node(
                &mut roots,
                &mut stack,
                XmlNode::CData(decode_bytes(event.as_ref())),
            ),
            Ok(Event::Comment(event)) => push_node(
                &mut roots,
                &mut stack,
                XmlNode::Comment(decode_bytes(event.as_ref())),
            ),
            Ok(Event::PI(event)) => push_node(
                &mut roots,
                &mut stack,
                XmlNode::ProcessingInstruction(decode_bytes(event.as_ref())),
            ),
            Ok(Event::DocType(event)) => push_node(
                &mut roots,
                &mut stack,
                XmlNode::DocType(decode_bytes(event.as_ref())),
            ),
            Ok(Event::Decl(event)) => {
                declaration = Some(decode_bytes(event.as_ref()));
            }
            Ok(Event::GeneralRef(event)) => push_node(
                &mut roots,
                &mut stack,
                XmlNode::GeneralRef(decode_bytes(event.as_ref())),
            ),
            Ok(Event::Eof) => break,
            Err(source) => return Err(Error::parse_error("Could not parse XML part.", source)),
        }
        buffer.clear();
    }

    if !stack.is_empty() {
        return Err(Error::unsupported_package(
            "XML document ended before all elements were closed.",
        ));
    }

    Ok(XmlDocument {
        declaration,
        nodes: roots,
    })
}

fn push_node(roots: &mut Vec<XmlNode>, stack: &mut [XmlElement], node: XmlNode) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(node);
    } else {
        roots.push(node);
    }
}

fn element_from_start(event: &BytesStart<'_>) -> Result<XmlElement> {
    let mut namespaces = NamespaceTable::new();
    let mut attributes = Vec::new();

    for attribute in event.attributes().with_checks(false) {
        let attribute = attribute
            .map_err(|source| Error::parse_error("Could not parse XML attribute.", source))?;
        let name = qname(attribute.key.as_ref());
        let value = decode_bytes(attribute.value.as_ref());
        let namespace_declaration = namespace_declaration(&name, &value, &mut namespaces);
        attributes.push(XmlAttribute {
            name,
            value,
            namespace_declaration,
        });
    }

    Ok(XmlElement {
        name: qname(event.name().as_ref()),
        attributes,
        namespaces,
        children: Vec::new(),
    })
}

fn qname(raw: &[u8]) -> QualifiedName {
    QualifiedName::from_raw(decode_bytes(raw))
}

fn namespace_declaration(
    name: &QualifiedName,
    value: &str,
    namespaces: &mut NamespaceTable,
) -> bool {
    if name.raw == "xmlns" {
        namespaces.push(NamespaceBinding::default(value));
        true
    } else if name.prefix.as_deref() == Some("xmlns") {
        namespaces.push(NamespaceBinding::prefixed(&name.local_name, value));
        true
    } else {
        false
    }
}

fn decode_bytes(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
#[test]
fn preserves_unknown_and_mc() {
    use crate::xml::document::XmlPart;

    let raw = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"
       xmlns:cust="urn:custom"
       xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"
       xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main"
       mc:Ignorable="a14 cust">
  <cust:unknown cust:flag="yes" data-id="42">
    <mc:AlternateContent>
      <mc:Choice Requires="a14">
        <a14:extension a14:value="kept">Text</a14:extension>
      </mc:Choice>
      <mc:Fallback>
        <cust:fallback cust:mode="legacy"/>
      </mc:Fallback>
    </mc:AlternateContent>
  </cust:unknown>
</p:sld>"#;

    let mut part = XmlPart::from_raw(raw.to_vec());
    let document = part.parse().expect("xml parses");
    let root = document.root_element().expect("root element exists");

    assert_eq!(root.name.raw, "p:sld");
    assert_eq!(root.name.prefix.as_deref(), Some("p"));
    assert_eq!(root.name.local_name, "sld");
    assert!(
        root.attributes
            .iter()
            .any(|attribute| attribute.name.raw == "mc:Ignorable" && attribute.value == "a14 cust")
    );
    assert_eq!(
        root.namespaces.resolve_prefix(Some("cust")),
        Some("urn:custom")
    );

    let unknown = root
        .children
        .iter()
        .find_map(XmlNode::as_element)
        .expect("unknown element exists");
    assert_eq!(unknown.name.raw, "cust:unknown");
    assert!(
        unknown
            .attributes
            .iter()
            .any(|attribute| attribute.name.raw == "cust:flag" && attribute.value == "yes")
    );
    assert!(
        unknown
            .attributes
            .iter()
            .any(|attribute| attribute.name.raw == "data-id" && attribute.value == "42")
    );

    let alternate_content = unknown
        .children
        .iter()
        .find_map(XmlNode::as_element)
        .expect("AlternateContent exists");
    assert_eq!(alternate_content.name.raw, "mc:AlternateContent");

    let choice = alternate_content
        .children
        .iter()
        .find_map(XmlNode::as_element)
        .expect("Choice exists");
    assert_eq!(choice.name.raw, "mc:Choice");
    assert!(
        choice
            .attributes
            .iter()
            .any(|attribute| attribute.name.raw == "Requires" && attribute.value == "a14")
    );

    let extension = choice
        .children
        .iter()
        .find_map(XmlNode::as_element)
        .expect("extension exists");
    assert_eq!(extension.name.raw, "a14:extension");
    assert!(
        extension
            .attributes
            .iter()
            .any(|attribute| attribute.name.raw == "a14:value" && attribute.value == "kept")
    );

    let fallback = alternate_content
        .children
        .iter()
        .filter_map(XmlNode::as_element)
        .find(|element| element.name.raw == "mc:Fallback")
        .expect("Fallback exists");
    let custom_fallback = fallback
        .children
        .iter()
        .find_map(XmlNode::as_element)
        .expect("custom fallback exists");
    assert_eq!(custom_fallback.name.raw, "cust:fallback");
    assert!(
        custom_fallback
            .attributes
            .iter()
            .any(|attribute| attribute.name.raw == "cust:mode" && attribute.value == "legacy")
    );
    assert!(!part.dirty);
}
