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
    pub dirty: bool,
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
}
