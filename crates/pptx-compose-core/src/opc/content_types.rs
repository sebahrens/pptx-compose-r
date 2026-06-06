use std::collections::BTreeMap;

use crate::{
    error::{Error, Result},
    opc::part_name::PartName,
    xml::{document::XmlElement, parser::parse_document},
};

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
        if raw.is_empty() || raw.iter().all(u8::is_ascii_whitespace) {
            return Err(Error::malformed_package(
                "[Content_Types].xml is missing or empty.",
            ));
        }

        let document = parse_document(raw).map_err(|source| {
            Error::with_source(
                crate::error::ErrorCode::UnsupportedPackage,
                "Could not parse [Content_Types].xml.",
                source,
            )
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
