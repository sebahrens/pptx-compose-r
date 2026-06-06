use std::collections::BTreeMap;

use crate::{
    error::{Error, ErrorCode, Result},
    opc::part_name::PartName,
    xml::{
        document::{QualifiedName, XmlAttribute, XmlDocument, XmlElement, XmlNode},
        namespaces::{NamespaceBinding, NamespaceTable},
        parser::parse_document_with_limits,
        writer::{WriteMode, WriteOptions, write_document},
    },
    zip::limits::ResourceLimits,
};

const CONTENT_TYPES_NS: &str = "http://schemas.openxmlformats.org/package/2006/content-types";

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ContentTypes {
    defaults: BTreeMap<String, String>,
    overrides: BTreeMap<PartName, String>,
}

impl ContentTypes {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn parse(raw: &[u8]) -> Result<Self> {
        Self::parse_with_limits(raw, &ResourceLimits::default())
    }

    pub fn parse_with_limits(raw: &[u8], limits: &ResourceLimits) -> Result<Self> {
        if raw.is_empty() || raw.iter().all(u8::is_ascii_whitespace) {
            return Err(Error::malformed_package(
                "[Content_Types].xml is missing or empty.",
            ));
        }

        let document = parse_document_with_limits(raw, limits).map_err(|source| {
            let code = if source.code() == ErrorCode::ResourceLimitExceeded {
                source.code()
            } else {
                ErrorCode::UnsupportedPackage
            };
            Error::with_source(code, "Could not parse [Content_Types].xml.", source)
        })?;
        let root = document.root_element().ok_or_else(|| {
            Error::malformed_package("[Content_Types].xml has no root Types element.")
        })?;

        if root.name.local_name != "Types" {
            return Err(Error::malformed_package(
                "[Content_Types].xml root element is not Types.",
            ));
        }

        let mut content_types = Self::new();
        for child in root.children.iter().filter_map(|node| node.as_element()) {
            match child.name.local_name.as_str() {
                "Default" => parse_default(child, &mut content_types)?,
                "Override" => parse_override(child, &mut content_types)?,
                _ => {}
            }
        }

        Ok(content_types)
    }

    pub fn insert_default(
        &mut self,
        extension: impl Into<String>,
        content_type: impl Into<String>,
    ) {
        self.defaults
            .insert(extension.into().to_ascii_lowercase(), content_type.into());
    }

    pub fn insert_override(&mut self, part_name: PartName, content_type: impl Into<String>) {
        self.overrides.insert(part_name, content_type.into());
    }

    pub fn to_xml(&self) -> Result<Vec<u8>> {
        write_document(
            &XmlDocument {
                declaration: Some(r#"version="1.0" encoding="UTF-8" standalone="yes""#.to_owned()),
                nodes: vec![XmlNode::Element(types_element(self))],
            },
            &WriteOptions {
                mode: WriteMode::Deterministic,
            },
        )
    }

    #[must_use]
    pub fn default_for_ext(&self, extension: &str) -> Option<&str> {
        self.defaults
            .get(&extension.to_ascii_lowercase())
            .map(String::as_str)
    }

    #[must_use]
    pub fn override_for(&self, part_name: &PartName) -> Option<&str> {
        self.overrides.get(part_name).map(String::as_str)
    }

    #[must_use]
    pub fn resolve(&self, part_name: &PartName) -> Option<&str> {
        self.overrides
            .get(part_name)
            .map(String::as_str)
            .or_else(|| {
                extension(part_name)
                    .and_then(|extension| self.defaults.get(&extension.to_ascii_lowercase()))
                    .map(String::as_str)
            })
    }

    pub fn defaults(&self) -> impl ExactSizeIterator<Item = (&str, &str)> {
        self.defaults
            .iter()
            .map(|(extension, content_type)| (extension.as_str(), content_type.as_str()))
    }

    pub fn overrides(&self) -> impl ExactSizeIterator<Item = (&PartName, &str)> {
        self.overrides
            .iter()
            .map(|(part_name, content_type)| (part_name, content_type.as_str()))
    }
}

fn types_element(content_types: &ContentTypes) -> XmlElement {
    let mut namespaces = NamespaceTable::new();
    namespaces.push(NamespaceBinding::default(CONTENT_TYPES_NS));

    let mut children =
        Vec::with_capacity(content_types.defaults.len() + content_types.overrides.len());
    children.extend(
        content_types
            .defaults()
            .map(|(extension, content_type)| default_element(extension, content_type)),
    );
    children.extend(
        content_types
            .overrides()
            .map(|(part_name, content_type)| override_element(part_name, content_type)),
    );

    XmlElement {
        name: QualifiedName::from_raw("Types"),
        attributes: vec![XmlAttribute {
            name: QualifiedName::from_raw("xmlns"),
            value: CONTENT_TYPES_NS.to_owned(),
            namespace_declaration: true,
        }],
        namespaces,
        children,
    }
}

fn default_element(extension: &str, content_type: &str) -> XmlNode {
    XmlNode::Element(XmlElement {
        name: QualifiedName::from_raw("Default"),
        attributes: vec![
            XmlAttribute {
                name: QualifiedName::from_raw("Extension"),
                value: extension.to_owned(),
                namespace_declaration: false,
            },
            XmlAttribute {
                name: QualifiedName::from_raw("ContentType"),
                value: content_type.to_owned(),
                namespace_declaration: false,
            },
        ],
        namespaces: NamespaceTable::new(),
        children: Vec::new(),
    })
}

fn override_element(part_name: &PartName, content_type: &str) -> XmlNode {
    XmlNode::Element(XmlElement {
        name: QualifiedName::from_raw("Override"),
        attributes: vec![
            XmlAttribute {
                name: QualifiedName::from_raw("PartName"),
                value: part_name.as_str().to_owned(),
                namespace_declaration: false,
            },
            XmlAttribute {
                name: QualifiedName::from_raw("ContentType"),
                value: content_type.to_owned(),
                namespace_declaration: false,
            },
        ],
        namespaces: NamespaceTable::new(),
        children: Vec::new(),
    })
}

fn parse_default(element: &XmlElement, content_types: &mut ContentTypes) -> Result<()> {
    let extension = required_attribute(element, "Extension", "Default")?;
    let content_type = required_attribute(element, "ContentType", "Default")?;

    content_types.insert_default(extension, content_type);
    Ok(())
}

fn parse_override(element: &XmlElement, content_types: &mut ContentTypes) -> Result<()> {
    let part_name = required_attribute(element, "PartName", "Override")?;
    let content_type = required_attribute(element, "ContentType", "Override")?;
    let part_name = PartName::from_zip_entry(part_name).map_err(|source| {
        Error::with_source(
            crate::error::ErrorCode::UnsupportedPackage,
            "[Content_Types].xml contains an invalid Override PartName.",
            source,
        )
    })?;

    content_types.insert_override(part_name, content_type);
    Ok(())
}

fn required_attribute<'a>(
    element: &'a XmlElement,
    attribute_name: &str,
    element_name: &str,
) -> Result<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.local_name == attribute_name)
        .map(|attribute| attribute.value.as_str())
        .ok_or_else(|| {
            Error::malformed_package(format!(
                "[Content_Types].xml {element_name} element is missing {attribute_name}."
            ))
        })
}

fn extension(part_name: &PartName) -> Option<&str> {
    let file_name = part_name.as_str().rsplit('/').next()?;
    let (_, extension) = file_name.rsplit_once('.')?;

    if extension.is_empty() {
        None
    } else {
        Some(extension)
    }
}

#[cfg(test)]
#[test]
fn resolution_order() {
    let mut content_types = ContentTypes::new();
    content_types.insert_default("xml", "application/xml");
    content_types.insert_default("png", "image/png");

    let slide = PartName::from_zip_entry("/ppt/slides/slide1.xml").expect("valid slide part");
    content_types.insert_override(
        slide.clone(),
        "application/vnd.openxmlformats-officedocument.presentationml.slide+xml",
    );

    let image = PartName::from_zip_entry("/ppt/media/image1.PNG").expect("valid image part");
    let unknown = PartName::from_zip_entry("/ppt/media/image1.unknown").expect("valid part");

    assert_eq!(
        content_types.resolve(&slide),
        Some("application/vnd.openxmlformats-officedocument.presentationml.slide+xml")
    );
    assert_eq!(content_types.resolve(&image), Some("image/png"));
    assert_eq!(content_types.resolve(&unknown), None);
}

#[cfg(test)]
#[test]
fn parses_defaults_and_overrides() {
    let raw = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
</Types>"#;

    let content_types = ContentTypes::parse(raw).expect("content types parse");
    let presentation =
        PartName::from_zip_entry("/ppt/presentation.xml").expect("part name is valid");

    assert_eq!(content_types.defaults.len(), 1);
    assert_eq!(content_types.overrides.len(), 1);
    assert_eq!(
        content_types.default_for_ext("RELS"),
        content_types.default_for_ext("rels")
    );
    assert_eq!(
        content_types.default_for_ext("rels"),
        Some("application/vnd.openxmlformats-package.relationships+xml")
    );
    assert_eq!(
        content_types.override_for(&presentation),
        Some("application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml")
    );
}

#[cfg(test)]
#[test]
fn rejects_missing_or_non_types_root() {
    use crate::error::ErrorCode;

    let empty = ContentTypes::parse(b"").expect_err("empty content types rejected");
    assert_eq!(empty.code(), ErrorCode::UnsupportedPackage);

    let wrong_root = ContentTypes::parse(br#"<NotTypes/>"#).expect_err("wrong root rejected");
    assert_eq!(wrong_root.code(), ErrorCode::UnsupportedPackage);
}

#[cfg(test)]
#[test]
fn serializes_deterministically() {
    let mut content_types = ContentTypes::new();
    content_types.insert_default("xml", "application/xml");
    content_types.insert_default("png", "image/png");
    content_types.insert_override(
        PartName::from_zip_entry("/ppt/slides/slide1.xml").expect("valid part name"),
        "application/vnd.openxmlformats-officedocument.presentationml.slide+xml",
    );

    let first = content_types.to_xml().expect("content types serialize");
    let second = content_types
        .to_xml()
        .expect("content types serialize again");
    assert_eq!(first, second);

    let reparsed = ContentTypes::parse(&first).expect("serialized content types parse");
    assert_eq!(reparsed, content_types);
}
