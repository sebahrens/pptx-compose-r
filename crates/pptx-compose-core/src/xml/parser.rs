use std::io::Cursor;

use quick_xml::{
    Reader, XmlVersion, escape,
    events::{BytesStart, BytesText, Event},
};

use crate::{
    error::{Error, Result},
    zip::limits::ResourceLimits,
};

use super::{
    chars::validate_xml_chars,
    document::{QualifiedName, XmlAttribute, XmlDocument, XmlElement, XmlNode},
    namespaces::{NamespaceBinding, NamespaceTable},
};

pub fn parse_document(raw: &[u8]) -> Result<XmlDocument> {
    parse_document_with_limits(raw, &ResourceLimits::default())
}

pub fn parse_document_with_limits(raw: &[u8], limits: &ResourceLimits) -> Result<XmlDocument> {
    let mut reader = Reader::from_reader(Cursor::new(raw));
    reader.config_mut().trim_text(false);

    let mut buffer = Vec::new();
    let mut declaration = None;
    let mut roots = Vec::new();
    let mut stack = Vec::new();
    let mut node_count = 0;

    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => {
                count_xml_node(&mut node_count, limits)?;
                ensure_xml_depth(stack.len() + 1, limits)?;
                stack.push(element_from_start(&event)?);
            }
            Ok(Event::Empty(event)) => {
                count_xml_node(&mut node_count, limits)?;
                ensure_xml_depth(stack.len() + 1, limits)?;
                push_node(
                    &mut roots,
                    &mut stack,
                    XmlNode::Element(element_from_start(&event)?),
                );
            }
            Ok(Event::End(_)) => {
                let Some(element) = stack.pop() else {
                    return Err(Error::malformed_xml(
                        "XML end tag encountered without a matching start tag.",
                    ));
                };
                push_node(&mut roots, &mut stack, XmlNode::Element(element));
            }
            Ok(Event::Text(event)) => {
                count_xml_node(&mut node_count, limits)?;
                push_node(&mut roots, &mut stack, XmlNode::Text(decode_text(&event)?))
            }
            Ok(Event::CData(event)) => {
                count_xml_node(&mut node_count, limits)?;
                push_node(
                    &mut roots,
                    &mut stack,
                    XmlNode::CData(decode_bytes(event.as_ref())),
                );
            }
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
            Err(source) => {
                return Err(Error::malformed_xml_with_source(
                    "XML part is not well formed.",
                    source,
                ));
            }
        }
        buffer.clear();
    }

    if !stack.is_empty() {
        return Err(Error::malformed_xml(
            "XML document ended before all elements were closed.",
        ));
    }

    Ok(XmlDocument {
        declaration,
        nodes: roots,
    })
}

fn count_xml_node(node_count: &mut u64, limits: &ResourceLimits) -> Result<()> {
    *node_count = node_count.saturating_add(1);
    if *node_count > limits.max_xml_node_count {
        return Err(Error::resource_limit_exceeded(format!(
            "XML part exceeded the maximum node count of {}.",
            limits.max_xml_node_count
        )));
    }
    Ok(())
}

fn ensure_xml_depth(depth: usize, limits: &ResourceLimits) -> Result<()> {
    if depth > limits.max_xml_depth {
        return Err(Error::resource_limit_exceeded(format!(
            "XML part exceeded the maximum element depth of {}.",
            limits.max_xml_depth
        )));
    }
    Ok(())
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
        let value = attribute
            .decoded_and_normalized_value(XmlVersion::default(), event.decoder())
            .map_err(|source| Error::parse_error("Could not decode XML attribute.", source))?
            .into_owned();
        validate_xml_chars(&value, "XML attribute value")?;
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

fn decode_text(event: &BytesText<'_>) -> Result<String> {
    let decoded = event
        .decode()
        .map_err(|source| Error::parse_error("Could not decode XML text.", source))?;
    let unescaped = escape::unescape(&decoded)
        .map_err(|source| Error::parse_error("Could not unescape XML text.", source))?;
    let text = unescaped.into_owned();
    validate_xml_chars(&text, "XML text")?;
    Ok(text)
}

#[cfg(test)]
#[test]
fn rejects_xml_exceeding_depth_limit() {
    use crate::{error::ErrorCode, zip::limits::ResourceLimits};

    let limits = ResourceLimits {
        max_xml_depth: 2,
        ..ResourceLimits::default()
    };
    let error = parse_document_with_limits(b"<a><b><c/></b></a>", &limits)
        .expect_err("deep XML must reject");

    assert_eq!(error.code(), ErrorCode::ResourceLimitExceeded);
    assert!(error.message().contains("maximum element depth"));
}

#[cfg(test)]
#[test]
fn rejects_xml_exceeding_node_count_limit() {
    use crate::{error::ErrorCode, zip::limits::ResourceLimits};

    let limits = ResourceLimits {
        max_xml_node_count: 2,
        ..ResourceLimits::default()
    };
    let error = parse_document_with_limits(b"<a>one<b/></a>", &limits)
        .expect_err("XML with too many nodes must reject");

    assert_eq!(error.code(), ErrorCode::ResourceLimitExceeded);
    assert!(error.message().contains("maximum node count"));
}

#[cfg(test)]
#[test]
fn rejects_unmatched_xml_end_tag_as_malformed_xml() {
    use crate::error::ErrorCode;

    let error = parse_document(b"<a><b></a>").expect_err("unmatched end tag must reject");

    assert_eq!(error.code(), ErrorCode::MalformedXml);
    assert!(error.message().contains("not well formed"));
}

#[cfg(test)]
#[test]
fn rejects_truncated_xml_as_malformed_xml() {
    use crate::error::ErrorCode;

    let error = parse_document(b"<a><b>").expect_err("truncated XML must reject");

    assert_eq!(error.code(), ErrorCode::MalformedXml);
    assert!(error.message().contains("before all elements were closed"));
}

#[cfg(test)]
#[test]
fn rejects_xml_illegal_text_character_as_malformed_xml() {
    use crate::error::ErrorCode;

    let error = parse_document("<a>bad\u{000B}text</a>".as_bytes())
        .expect_err("illegal XML text character must reject");

    assert_eq!(error.code(), ErrorCode::MalformedXml);
    assert!(error.message().contains("U+000B"));
}

#[cfg(test)]
#[test]
fn rejects_xml_illegal_attribute_character_as_malformed_xml() {
    use crate::error::ErrorCode;

    let error = parse_document("<a value=\"bad\u{000B}text\"/>".as_bytes())
        .expect_err("illegal XML attribute character must reject");

    assert_eq!(error.code(), ErrorCode::MalformedXml);
    assert!(error.message().contains("U+000B"));
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
    assert!(!part.is_dirty());
}

#[cfg(test)]
#[test]
fn decodes_attribute_entities_before_model_storage() {
    use crate::xml::writer::{self, WriteMode, WriteOptions};

    let raw = br#"<p:sld xmlns:p="urn:p&amp;deck" data-text="A &amp; B &lt; C &gt; D &quot;Q&quot; &apos;A&apos;"><p:cSld/></p:sld>"#;
    let parsed = parse_document(raw).expect("xml parses");
    let root = parsed.root_element().expect("root element exists");

    assert_eq!(
        root.namespaces.resolve_prefix(Some("p")),
        Some("urn:p&deck")
    );
    assert!(
        root.attributes
            .iter()
            .any(|attribute| attribute.name.raw == "xmlns:p"
                && attribute.value == "urn:p&deck"
                && attribute.namespace_declaration)
    );
    assert!(
        root.attributes
            .iter()
            .any(|attribute| attribute.name.raw == "data-text"
                && attribute.value == "A & B < C > D \"Q\" 'A'")
    );

    let written = writer::write_document(
        &parsed,
        &WriteOptions {
            mode: WriteMode::Deterministic,
        },
    )
    .expect("dirty XML writes");
    let serialized = String::from_utf8(written).expect("writer emits UTF-8");
    assert!(serialized.contains(r#"xmlns:p="urn:p&amp;deck""#));
    assert!(
        serialized.contains(r#"data-text="A &amp; B &lt; C &gt; D &quot;Q&quot; &apos;A&apos;""#)
    );
    assert!(!serialized.contains("&amp;amp;"));
    assert!(!serialized.contains("&amp;lt;"));
}

#[cfg(test)]
#[test]
fn preserves_non_semantic_xml_nodes_while_decoding_text_events() {
    use crate::xml::writer::{self, WriteOptions};

    let raw = br#"<root>plain<![CDATA[A &amp; B]]><!--C &amp; D--><?pi E &amp; F?><!DOCTYPE note SYSTEM "note.dtd">&amp;&lt;&gt;&quot;&apos;</root>"#;
    let parsed = parse_document(raw).expect("xml parses");
    let root = parsed.root_element().expect("root element exists");

    assert!(
        root.children
            .iter()
            .any(|node| matches!(node, XmlNode::Text(text) if text == "plain"))
    );
    assert!(
        root.children
            .iter()
            .any(|node| matches!(node, XmlNode::CData(text) if text == "A &amp; B"))
    );
    assert!(
        root.children
            .iter()
            .any(|node| matches!(node, XmlNode::Comment(text) if text == "C &amp; D"))
    );
    assert!(root.children.iter().any(
        |node| matches!(node, XmlNode::ProcessingInstruction(text) if text == "pi E &amp; F")
    ));
    assert!(
        root.children.iter().any(
            |node| matches!(node, XmlNode::DocType(text) if text == r#"note SYSTEM "note.dtd""#)
        )
    );

    let references = root
        .children
        .iter()
        .filter_map(|node| match node {
            XmlNode::GeneralRef(reference) => Some(reference.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(references, ["amp", "lt", "gt", "quot", "apos"]);

    let written = writer::write_document(&parsed, &WriteOptions::default()).expect("xml writes");
    let serialized = String::from_utf8(written).expect("writer emits UTF-8");
    assert!(serialized.contains("<![CDATA[A &amp; B]]>"));
    assert!(serialized.contains("<!--C &amp; D-->"));
    assert!(serialized.contains("<?pi E &amp; F?>"));
    assert!(serialized.contains("&amp;&lt;&gt;&quot;&apos;"));
    assert!(!serialized.contains("&amp;amp;"));
}
