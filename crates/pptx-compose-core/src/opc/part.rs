use std::collections::HashMap;

use crate::{
    error::{Error, Result},
    opc::part_name::PartName,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Part {
    name: PartName,
    original_zip_entry_name: String,
    bytes: Vec<u8>,
}

impl Part {
    pub fn from_zip_entry(zip_entry_name: impl Into<String>, bytes: Vec<u8>) -> Result<Self> {
        let original_zip_entry_name = zip_entry_name.into();
        let name = PartName::from_zip_entry(&original_zip_entry_name)?;

        Ok(Self {
            name,
            original_zip_entry_name,
            bytes,
        })
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
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub fn bytes_mut(&mut self) -> &mut Vec<u8> {
        &mut self.bytes
    }
}

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
            return Err(Error::duplicate_part(format!(
                "Package contains more than one part named {}.",
                part.name()
            )));
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
