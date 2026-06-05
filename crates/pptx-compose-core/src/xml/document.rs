use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};

use crate::error::{Error, Result};

use super::{namespaces::NamespaceTable, parser};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QualifiedName {
    pub raw: String,
    pub prefix: Option<String>,
    pub local_name: String,
}

impl QualifiedName {
    #[must_use]
    pub fn from_raw(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        if let Some((prefix, local_name)) = raw.split_once(':') {
            Self {
                prefix: Some(prefix.to_owned()),
                local_name: local_name.to_owned(),
                raw,
            }
        } else {
            Self {
                local_name: raw.clone(),
                prefix: None,
                raw,
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XmlAttribute {
    pub name: QualifiedName,
    pub value: String,
    pub namespace_declaration: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XmlElement {
    pub name: QualifiedName,
    pub attributes: Vec<XmlAttribute>,
    pub namespaces: NamespaceTable,
    pub children: Vec<XmlNode>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum XmlNode {
    Element(XmlElement),
    Text(String),
    CData(String),
    Comment(String),
    ProcessingInstruction(String),
    DocType(String),
    GeneralRef(String),
}

impl XmlNode {
    #[must_use]
    pub const fn as_element(&self) -> Option<&XmlElement> {
        match self {
            Self::Element(element) => Some(element),
            Self::Text(_)
            | Self::CData(_)
            | Self::Comment(_)
            | Self::ProcessingInstruction(_)
            | Self::DocType(_)
            | Self::GeneralRef(_) => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XmlDocument {
    pub declaration: Option<String>,
    pub nodes: Vec<XmlNode>,
}

impl XmlDocument {
    #[must_use]
    pub fn root_element(&self) -> Option<&XmlElement> {
        self.nodes.iter().find_map(XmlNode::as_element)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XmlDiagnostic {
    pub part_path: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct XmlPart {
    pub raw: Vec<u8>,
    pub parsed: Option<XmlDocument>,
    dirty: bool,
    pub diagnostics: Vec<XmlDiagnostic>,
}

impl XmlPart {
    #[must_use]
    pub fn from_raw(raw: Vec<u8>) -> Self {
        Self {
            raw,
            parsed: None,
            dirty: false,
            diagnostics: Vec::new(),
        }
    }

    pub fn parse(&mut self) -> Result<&XmlDocument> {
        self.parse_with_part_path(None)
    }

    pub fn parse_with_part_path(&mut self, part_path: Option<&str>) -> Result<&XmlDocument> {
        if self.parsed.is_none() {
            match parser::parse_document(&self.raw) {
                Ok(document) => {
                    self.parsed = Some(document);
                }
                Err(error) => {
                    self.diagnostics.push(XmlDiagnostic {
                        part_path: part_path.map(str::to_owned),
                        message: error.message().to_owned(),
                    });
                    return Err(error);
                }
            }
        }

        self.parsed.as_ref().ok_or_else(|| {
            Error::unsupported_package("XML parse finished without producing a document.")
        })
    }

    #[must_use]
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub const fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Replace this XML part with raw bytes after a basic well-formedness scan.
    ///
    /// This is the advanced raw escape hatch from spec 020. It validates only
    /// XML well-formedness here; package graph validation is enforced by the
    /// validation/write layer before a package is written.
    pub fn replace_raw(&mut self, bytes: Vec<u8>) -> Result<()> {
        ensure_well_formed(&bytes)?;
        self.raw = bytes;
        self.parsed = None;
        self.dirty = true;
        Ok(())
    }
}

fn ensure_well_formed(bytes: &[u8]) -> Result<()> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut stack: Vec<Vec<u8>> = Vec::new();

    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => {
                validate_attributes(&event)?;
                stack.push(event.name().as_ref().to_vec());
            }
            Ok(Event::Empty(event)) => validate_attributes(&event)?,
            Ok(Event::End(event)) => {
                let Some(start_name) = stack.pop() else {
                    return Err(Error::malformed_xml(
                        "XML end tag encountered without a matching start tag.",
                    ));
                };
                if start_name != event.name().as_ref() {
                    return Err(Error::malformed_xml(
                        "XML end tag does not match the current start tag.",
                    ));
                }
            }
            Ok(Event::Eof) => break,
            Ok(
                Event::Text(_)
                | Event::CData(_)
                | Event::Comment(_)
                | Event::PI(_)
                | Event::DocType(_)
                | Event::Decl(_)
                | Event::GeneralRef(_),
            ) => {}
            Err(source) => {
                return Err(Error::malformed_xml_with_source(
                    "Raw XML replacement is not well-formed.",
                    source,
                ));
            }
        }
        buffer.clear();
    }

    if stack.is_empty() {
        Ok(())
    } else {
        Err(Error::malformed_xml(
            "XML document ended before all elements were closed.",
        ))
    }
}

fn validate_attributes(event: &BytesStart<'_>) -> Result<()> {
    for attribute in event.attributes().with_checks(true) {
        attribute.map_err(|source| {
            Error::malformed_xml_with_source(
                "Raw XML replacement has an invalid attribute.",
                source,
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
#[test]
fn dirty_transitions() {
    use std::{
        collections::hash_map::DefaultHasher,
        hash::{Hash, Hasher},
    };

    fn raw_checksum(part: &XmlPart) -> u64 {
        let mut hasher = DefaultHasher::new();
        part.raw.hash(&mut hasher);
        hasher.finish()
    }

    let raw = br#"<p:sld xmlns:p="urn:p"><p:cSld><p:spTree/></p:cSld></p:sld>"#.to_vec();
    let mut target = XmlPart::from_raw(raw.clone());
    let untouched = XmlPart::from_raw(raw);

    assert!(!target.is_dirty());
    assert!(!untouched.is_dirty());

    let document = target.parse().expect("xml parses");
    assert_eq!(
        document
            .root_element()
            .map(|element| element.name.raw.as_str()),
        Some("p:sld")
    );
    assert!(!target.is_dirty());

    let checksum = raw_checksum(&target);
    assert_eq!(checksum, raw_checksum(&target));
    assert!(!target.is_dirty());

    target.mark_dirty();

    assert!(target.is_dirty());
    assert!(!untouched.is_dirty());
}

#[cfg(test)]
#[test]
fn raw_escape_hatch() {
    use crate::error::ErrorCode;

    let original = br#"<p:sld xmlns:p="urn:p"><p:cSld/></p:sld>"#.to_vec();
    let replacement = br#"<p:sld xmlns:p="urn:p"><p:txBody>updated</p:txBody></p:sld>"#.to_vec();
    let mut part = XmlPart::from_raw(original.clone());

    assert_eq!(
        part.parse()
            .expect("original XML parses")
            .root_element()
            .map(|element| element.name.raw.as_str()),
        Some("p:sld")
    );
    assert!(part.parsed.is_some());

    part.replace_raw(replacement.clone())
        .expect("valid replacement succeeds");

    assert_eq!(part.raw, replacement);
    assert!(part.is_dirty());
    assert!(part.parsed.is_none());
    let document = part.parse().expect("replacement reparses lazily");
    let root = document.root_element().expect("root element exists");
    assert_eq!(root.name.raw, "p:sld");
    assert!(root.children.iter().any(|child| {
        child
            .as_element()
            .is_some_and(|element| element.name.raw == "p:txBody")
    }));

    let raw_before_malformed = part.raw.clone();
    let dirty_before_malformed = part.is_dirty();
    let parsed_before_malformed = part.parsed.clone();
    let error = part
        .replace_raw(br#"<p:sld xmlns:p="urn:p"><p:cSld></p:sld>"#.to_vec())
        .expect_err("malformed replacement fails");

    assert_eq!(error.code(), ErrorCode::MalformedXml);
    assert_eq!(part.raw, raw_before_malformed);
    assert_eq!(part.is_dirty(), dirty_before_malformed);
    assert_eq!(part.parsed, parsed_before_malformed);
}
