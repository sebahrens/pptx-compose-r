use std::collections::BTreeMap;

use crate::opc::part_name::PartName;

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

fn extension(part_name: &PartName) -> Option<&str> {
    let file_name = part_name.as_str().rsplit('/').next()?;
    let (_, extension) = file_name.rsplit_once('.')?;

    if extension.is_empty() {
        None
    } else {
        Some(extension)
    }
}
