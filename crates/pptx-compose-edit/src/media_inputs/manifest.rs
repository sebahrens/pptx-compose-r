use std::{fmt, path::PathBuf};

use pptx_compose_core::error::{Error, ErrorCode};
use schemars::JsonSchema;
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, MapAccess, Visitor},
};
use serde_json::{Value, json};

pub const MEDIA_MANIFEST_SCHEMA: &str = "pptx-compose.media_manifest.v1";
pub const MEDIA_MANIFEST_VERSION: u32 = 1;

pub fn media_manifest_json_schema() -> pptx_compose_core::error::Result<Value> {
    let binding_schema = schemars::schema_for!(ManifestMediaBinding);
    let mut binding_value = serde_json::to_value(binding_schema).map_err(|source| {
        Error::with_source(
            ErrorCode::InternalError,
            "Could not serialize media manifest JSON schema.",
            source,
        )
    })?;
    let definitions = binding_value
        .as_object_mut()
        .and_then(|object| object.remove("$defs"))
        .unwrap_or_else(|| json!({}));
    if let Some(object) = binding_value.as_object_mut() {
        object.remove("$schema");
        object.remove("title");
    }

    Ok(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": MEDIA_MANIFEST_SCHEMA,
        "$defs": definitions,
        "title": "MediaManifest",
        "type": "object",
        "additionalProperties": false,
        "required": ["schema", "version", "media"],
        "properties": {
            "schema": {
                "type": "string",
                "const": MEDIA_MANIFEST_SCHEMA
            },
            "version": {
                "type": "integer",
                "const": MEDIA_MANIFEST_VERSION
            },
            "media": {
                "type": "object",
                "additionalProperties": binding_value
            }
        }
    }))
}

#[derive(Clone, Debug, Eq, PartialEq, JsonSchema, Serialize)]
pub struct MediaManifest {
    pub schema: String,
    pub version: u32,
    pub media: Vec<MediaManifestEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, JsonSchema, Serialize)]
pub struct MediaManifestEntry {
    pub media_ref: String,
    pub binding: ManifestMediaBinding,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestMediaBinding {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline: Option<InlineMedia>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub byte_length: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InlineMedia {
    pub encoding: String,
    pub content_type: String,
    pub data: String,
}

impl<'de> Deserialize<'de> for MediaManifest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(MediaManifestVisitor)
    }
}

struct MediaManifestVisitor;

impl<'de> Visitor<'de> for MediaManifestVisitor {
    type Value = MediaManifest;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a pptx-compose media manifest object")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut schema = None;
        let mut version = None;
        let mut media = None;

        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "schema" => {
                    if schema.is_some() {
                        return Err(de::Error::duplicate_field("schema"));
                    }
                    schema = Some(map.next_value()?);
                }
                "version" => {
                    if version.is_some() {
                        return Err(de::Error::duplicate_field("version"));
                    }
                    version = Some(map.next_value()?);
                }
                "media" => {
                    if media.is_some() {
                        return Err(de::Error::duplicate_field("media"));
                    }
                    media = Some(map.next_value::<MediaEntries>()?.0);
                }
                _ => {
                    return Err(de::Error::unknown_field(
                        &key,
                        &["schema", "version", "media"],
                    ));
                }
            }
        }

        Ok(MediaManifest {
            schema: schema.ok_or_else(|| de::Error::missing_field("schema"))?,
            version: version.ok_or_else(|| de::Error::missing_field("version"))?,
            media: media.ok_or_else(|| de::Error::missing_field("media"))?,
        })
    }
}

struct MediaEntries(Vec<MediaManifestEntry>);

impl<'de> Deserialize<'de> for MediaEntries {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(MediaEntriesVisitor)
    }
}

struct MediaEntriesVisitor;

impl<'de> Visitor<'de> for MediaEntriesVisitor {
    type Value = MediaEntries;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a media binding object keyed by media_ref")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries = Vec::new();

        while let Some(media_ref) = map.next_key::<String>()? {
            entries.push(MediaManifestEntry {
                media_ref,
                binding: map.next_value()?,
            });
        }

        Ok(MediaEntries(entries))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_duplicate_media_refs_for_binder_rejection() {
        let manifest: MediaManifest = serde_json::from_str(
            r#"{
                "schema": "pptx-compose.media_manifest.v1",
                "version": 1,
                "media": {
                    "image": { "path": "one.png", "content_type": "image/png" },
                    "image": { "path": "two.png", "content_type": "image/png" }
                }
            }"#,
        )
        .expect("manifest parses");

        assert_eq!(manifest.media.len(), 2);
        assert_eq!(manifest.media[0].media_ref, "image");
        assert_eq!(manifest.media[1].media_ref, "image");
    }
}
