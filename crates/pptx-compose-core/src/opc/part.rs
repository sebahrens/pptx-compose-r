use std::collections::HashMap;

use crate::{
    error::{Error, ErrorLocation, Result},
    opc::{content_types::ContentTypes, part_name::PartName},
    xml::document::XmlPart,
    zip::{ZipEntryMetadata, reader::RawEntry},
};

pub type ContentType = String;

#[derive(Clone, Debug)]
pub struct Part {
    name: PartName,
    content_type: Option<ContentType>,
    data: PartData,
    zip_metadata: ZipEntryMetadata,
    original_zip_entry_name: String,
    control: ControlPartFlags,
}

#[derive(Clone, Debug)]
pub enum PartData {
    Xml(XmlPart),
    Binary(BinaryPart),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BinaryPart {
    bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ControlPartFlags {
    pub is_content_types: bool,
    pub is_relationships: bool,
}

impl ControlPartFlags {
    #[must_use]
    pub fn for_name(name: &PartName) -> Self {
        Self {
            is_content_types: name.as_str() == "/[Content_Types].xml",
            is_relationships: name.as_str().ends_with(".rels")
                && (name.as_str() == "/_rels/.rels" || name.as_str().contains("/_rels/")),
        }
    }

    #[must_use]
    pub const fn is_control(self) -> bool {
        self.is_content_types || self.is_relationships
    }
}

impl Part {
    pub fn from_zip_entry(zip_entry_name: impl Into<String>, bytes: Vec<u8>) -> Result<Self> {
        let original_zip_entry_name = zip_entry_name.into();
        let name = PartName::from_zip_entry(&original_zip_entry_name)?;
        let zip_metadata = synthetic_zip_metadata(&original_zip_entry_name, &bytes);

        Ok(Self::from_parts(
            name,
            original_zip_entry_name,
            bytes,
            zip_metadata,
        ))
    }

    #[must_use]
    pub fn from_raw_entry(entry: &RawEntry) -> Self {
        Self::from_parts(
            entry.name.clone(),
            entry.meta.original_name.clone(),
            entry.bytes.clone(),
            entry.meta.clone(),
        )
    }

    #[must_use]
    pub fn from_parts(
        name: PartName,
        original_zip_entry_name: String,
        bytes: Vec<u8>,
        zip_metadata: ZipEntryMetadata,
    ) -> Self {
        let control = ControlPartFlags::for_name(&name);
        Self {
            name,
            content_type: None,
            data: PartData::Binary(BinaryPart { bytes }),
            zip_metadata,
            original_zip_entry_name,
            control,
        }
    }

    #[must_use]
    pub fn name(&self) -> &PartName {
        &self.name
    }

    #[must_use]
    pub fn original_zip_entry_name(&self) -> &str {
        &self.original_zip_entry_name
    }

    #[must_use]
    pub fn zip_entry_name(&self) -> &str {
        self.name.zip_entry_name()
    }

    #[must_use]
    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    pub fn set_content_type_from(&mut self, content_types: &ContentTypes) {
        self.content_type = content_types.resolve(&self.name).map(str::to_owned);
    }

    #[must_use]
    pub const fn zip_metadata(&self) -> &ZipEntryMetadata {
        &self.zip_metadata
    }

    #[must_use]
    pub const fn control_flags(&self) -> ControlPartFlags {
        self.control
    }

    #[must_use]
    pub const fn is_control_part(&self) -> bool {
        self.control.is_control()
    }

    #[must_use]
    pub const fn data(&self) -> &PartData {
        &self.data
    }

    pub const fn data_mut(&mut self) -> &mut PartData {
        &mut self.data
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.data.bytes()
    }

    #[must_use]
    pub fn bytes_mut(&mut self) -> &mut Vec<u8> {
        self.data.bytes_mut()
    }
}

impl PartData {
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::Xml(part) => &part.raw,
            Self::Binary(part) => &part.bytes,
        }
    }

    pub fn bytes_mut(&mut self) -> &mut Vec<u8> {
        match self {
            Self::Xml(part) => &mut part.raw,
            Self::Binary(part) => &mut part.bytes,
        }
    }
}

impl PartialEq for PartData {
    fn eq(&self, other: &Self) -> bool {
        self.bytes() == other.bytes()
    }
}

impl Eq for PartData {}

impl PartialEq for Part {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.content_type == other.content_type
            && self.data == other.data
            && self.zip_metadata == other.zip_metadata
            && self.original_zip_entry_name == other.original_zip_entry_name
            && self.control == other.control
    }
}

impl Eq for Part {}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PartStore {
    parts: Vec<Part>,
    by_name: HashMap<PartName, usize>,
}

impl PartStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_zip_entry(
        &mut self,
        zip_entry_name: impl Into<String>,
        bytes: Vec<u8>,
    ) -> Result<&Part> {
        self.insert(Part::from_zip_entry(zip_entry_name, bytes)?)
    }

    pub fn insert(&mut self, part: Part) -> Result<&Part> {
        if self.by_name.contains_key(part.name()) {
            return Err(Error::unsupported_package(format!(
                "Package contains more than one part named {}.",
                part.name()
            ))
            .with_location(ErrorLocation {
                part: Some(part.name().zip_entry_name().to_owned()),
                zip_entry: Some(part.original_zip_entry_name().to_owned()),
                ..ErrorLocation::default()
            }));
        }

        let index = self.parts.len();
        self.by_name.insert(part.name().clone(), index);
        self.parts.push(part);

        Ok(&self.parts[index])
    }

    #[must_use]
    pub fn get(&self, name: &PartName) -> Option<&Part> {
        self.by_name
            .get(name)
            .and_then(|index| self.parts.get(*index))
    }

    #[must_use]
    pub fn get_mut(&mut self, name: &PartName) -> Option<&mut Part> {
        let index = *self.by_name.get(name)?;
        self.parts.get_mut(index)
    }

    pub fn remove(&mut self, name: &PartName) -> Option<Part> {
        let index = self.by_name.remove(name)?;
        let removed = self.parts.remove(index);
        for stored_index in self.by_name.values_mut() {
            if *stored_index > index {
                *stored_index -= 1;
            }
        }
        Some(removed)
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &Part> {
        self.parts.iter()
    }

    pub fn iter_mut(&mut self) -> impl ExactSizeIterator<Item = &mut Part> {
        self.parts.iter_mut()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.parts.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }
}

fn synthetic_zip_metadata(original_name: &str, bytes: &[u8]) -> ZipEntryMetadata {
    ZipEntryMetadata {
        entry_index: usize::MAX,
        original_name: original_name.to_owned(),
        compression_method: zip::CompressionMethod::Stored,
        crc32: 0,
        compressed_size: bytes.len() as u64,
        uncompressed_size: bytes.len() as u64,
        last_modified: None,
        external_attrs: None,
        is_dir: false,
    }
}
