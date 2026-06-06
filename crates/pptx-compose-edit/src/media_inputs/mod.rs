pub mod manifest;
pub mod sniff;

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
};

use pptx_compose_core::error::{Error, ErrorCode, Result};

pub use manifest::{InlineMedia, ManifestMediaBinding, MediaManifest, MediaManifestEntry};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MediaSource {
    Path(PathBuf),
    Bytes(Vec<u8>),
    InlineBase64(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaBinding {
    pub content_type: String,
    pub declared_sha256: Option<String>,
    pub declared_byte_length: Option<u64>,
    pub source: MediaSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedMedia {
    pub content_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MediaInputs(HashMap<String, MediaBinding>);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExtraBindingPolicy {
    Warn,
    Strict,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaInputWarning {
    pub media_ref: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MediaInputReport {
    pub warnings: Vec<MediaInputWarning>,
}

impl MediaInputs {
    #[must_use]
    pub fn new(bindings: HashMap<String, MediaBinding>) -> Self {
        Self(bindings)
    }

    pub fn from_manifest(manifest: &MediaManifest, media_root: &Path) -> Result<Self> {
        let mut bindings = HashMap::with_capacity(manifest.media.len());

        for entry in &manifest.media {
            if bindings.contains_key(&entry.media_ref) {
                return Err(Error::new(
                    ErrorCode::InvalidInput,
                    format!(
                        "Media manifest contains duplicate media_ref `{}`.",
                        entry.media_ref
                    ),
                ));
            }

            let binding = manifest_binding(&entry.binding, media_root)?;
            bindings.insert(entry.media_ref.clone(), binding);
        }

        Ok(Self(bindings))
    }

    pub fn resolve(&self, media_ref: &str) -> Result<ResolvedMedia> {
        let binding = self.0.get(media_ref).ok_or_else(|| {
            Error::new(
                ErrorCode::MissingMediaRef,
                format!("No media input is bound for media_ref `{media_ref}`."),
            )
        })?;

        let bytes = match &binding.source {
            MediaSource::Path(path) => fs::read(path).map_err(|source| {
                Error::with_source(
                    ErrorCode::InvalidInput,
                    format!("Could not read media input `{}`.", path.display()),
                    source,
                )
            })?,
            MediaSource::Bytes(bytes) => bytes.clone(),
            MediaSource::InlineBase64(encoded) => decode_base64(encoded)?,
        };
        sniff::verify_declared(&binding.content_type, &bytes)?;

        Ok(ResolvedMedia {
            content_type: binding.content_type.clone(),
            bytes,
        })
    }

    pub fn check_references<'a>(
        &self,
        referenced_media: impl IntoIterator<Item = &'a str>,
        extra_policy: ExtraBindingPolicy,
    ) -> Result<MediaInputReport> {
        let mut referenced = HashSet::new();
        for media_ref in referenced_media {
            if !self.0.contains_key(media_ref) {
                return Err(Error::new(
                    ErrorCode::MissingMediaRef,
                    format!("No media input is bound for media_ref `{media_ref}`."),
                ));
            }
            referenced.insert(media_ref);
        }

        let mut warnings = Vec::new();
        for media_ref in self.0.keys() {
            if referenced.contains(media_ref.as_str()) {
                continue;
            }

            let message = format!("Media input `{media_ref}` is bound but not referenced.");
            if extra_policy == ExtraBindingPolicy::Strict {
                return Err(Error::new(ErrorCode::InvalidInput, message));
            }
            warnings.push(MediaInputWarning {
                media_ref: media_ref.clone(),
                message,
            });
        }

        Ok(MediaInputReport { warnings })
    }
}

fn manifest_binding(binding: &ManifestMediaBinding, media_root: &Path) -> Result<MediaBinding> {
    match &binding.inline {
        Some(inline) => {
            if inline.encoding != "base64" {
                return Err(Error::new(
                    ErrorCode::InvalidInput,
                    "Inline media encoding must be `base64`.",
                ));
            }

            Ok(MediaBinding {
                content_type: inline.content_type.clone(),
                declared_sha256: binding.sha256.clone(),
                declared_byte_length: binding.byte_length,
                source: MediaSource::InlineBase64(inline.data.clone()),
            })
        }
        None => {
            let relative_path = binding.path.as_ref().ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidInput,
                    "Manifest media binding must contain either `path` or `inline`.",
                )
            })?;
            let content_type = binding.content_type.as_ref().ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidInput,
                    "Manifest path media binding must declare `content_type`.",
                )
            })?;

            Ok(MediaBinding {
                content_type: content_type.clone(),
                declared_sha256: binding.sha256.clone(),
                declared_byte_length: binding.byte_length,
                source: MediaSource::Path(resolve_media_path(media_root, relative_path)?),
            })
        }
    }
}

fn resolve_media_path(media_root: &Path, manifest_path: &Path) -> Result<PathBuf> {
    if manifest_path.is_absolute() {
        return Err(Error::unsafe_path(
            "Manifest media paths must be relative to media_root.",
        ));
    }

    let mut safe_relative = PathBuf::new();
    for component in manifest_path.components() {
        match component {
            Component::Normal(segment) => safe_relative.push(segment),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(Error::unsafe_path(
                    "Manifest media paths must not escape media_root.",
                ));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(Error::unsafe_path(
                    "Manifest media paths must be relative to media_root.",
                ));
            }
        }
    }

    Ok(media_root.join(safe_relative))
}

fn decode_base64(encoded: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity((encoded.len() / 4) * 3);
    let mut quartet = [0_u8; 4];
    let mut quartet_len = 0_usize;
    let mut saw_padding = false;

    for byte in encoded.bytes().filter(|byte| !byte.is_ascii_whitespace()) {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                saw_padding = true;
                64
            }
            _ => {
                return Err(Error::new(
                    ErrorCode::InvalidInput,
                    "Inline media contains invalid base64 characters.",
                ));
            }
        };

        if saw_padding && value != 64 {
            return Err(Error::new(
                ErrorCode::InvalidInput,
                "Inline media contains invalid base64 padding.",
            ));
        }

        quartet[quartet_len] = value;
        quartet_len += 1;

        if quartet_len == 4 {
            push_base64_quartet(quartet, &mut bytes)?;
            quartet_len = 0;
        }
    }

    if quartet_len != 0 {
        return Err(Error::new(
            ErrorCode::InvalidInput,
            "Inline media contains incomplete base64 data.",
        ));
    }

    Ok(bytes)
}

fn push_base64_quartet(quartet: [u8; 4], bytes: &mut Vec<u8>) -> Result<()> {
    if quartet[0] == 64 || quartet[1] == 64 {
        return Err(Error::new(
            ErrorCode::InvalidInput,
            "Inline media contains invalid base64 padding.",
        ));
    }
    if quartet[2] == 64 && quartet[3] != 64 {
        return Err(Error::new(
            ErrorCode::InvalidInput,
            "Inline media contains invalid base64 padding.",
        ));
    }

    bytes.push((quartet[0] << 2) | (quartet[1] >> 4));
    if quartet[2] != 64 {
        bytes.push((quartet[1] << 4) | (quartet[2] >> 2));
    }
    if quartet[3] != 64 {
        bytes.push((quartet[2] << 6) | quartet[3]);
    }

    Ok(())
}

#[cfg(test)]
pub mod resolve {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use pptx_compose_core::error::ErrorCode;

    use super::*;

    #[test]
    fn binds_and_reports_missing() {
        let media_root = temp_media_root();
        fs::create_dir_all(&media_root).expect("media root can be created");
        fs::write(
            media_root.join("image-one.png"),
            b"\x89PNG\r\n\x1a\npng bytes",
        )
        .expect("media one can be written");
        fs::write(media_root.join("image-two.jpg"), b"\xff\xd8\xffjpg bytes")
            .expect("media two can be written");

        let manifest = MediaManifest {
            schema: "pptx-compose.media_manifest.v1".to_owned(),
            version: 1,
            media: vec![
                MediaManifestEntry {
                    media_ref: "input-image-1".to_owned(),
                    binding: ManifestMediaBinding {
                        path: Some(PathBuf::from("image-one.png")),
                        content_type: Some("image/png".to_owned()),
                        inline: None,
                        sha256: None,
                        byte_length: None,
                    },
                },
                MediaManifestEntry {
                    media_ref: "input-image-2".to_owned(),
                    binding: ManifestMediaBinding {
                        path: Some(PathBuf::from("image-two.jpg")),
                        content_type: Some("image/jpeg".to_owned()),
                        inline: None,
                        sha256: None,
                        byte_length: None,
                    },
                },
            ],
        };

        let inputs = MediaInputs::from_manifest(&manifest, &media_root).expect("manifest binds");

        let first = inputs
            .resolve("input-image-1")
            .expect("first media resolves");
        assert_eq!(first.content_type, "image/png");
        assert_eq!(first.bytes, b"\x89PNG\r\n\x1a\npng bytes");

        let second = inputs
            .resolve("input-image-2")
            .expect("second media resolves");
        assert_eq!(second.content_type, "image/jpeg");
        assert_eq!(second.bytes, b"\xff\xd8\xffjpg bytes");

        let missing = inputs
            .resolve("unknown-image")
            .expect_err("unknown media_ref fails");
        assert_eq!(missing.code(), ErrorCode::MissingMediaRef);

        let duplicate_manifest = MediaManifest {
            schema: "pptx-compose.media_manifest.v1".to_owned(),
            version: 1,
            media: vec![
                MediaManifestEntry {
                    media_ref: "dup".to_owned(),
                    binding: ManifestMediaBinding {
                        path: Some(PathBuf::from("image-one.png")),
                        content_type: Some("image/png".to_owned()),
                        inline: None,
                        sha256: None,
                        byte_length: None,
                    },
                },
                MediaManifestEntry {
                    media_ref: "dup".to_owned(),
                    binding: ManifestMediaBinding {
                        path: Some(PathBuf::from("image-two.jpg")),
                        content_type: Some("image/jpeg".to_owned()),
                        inline: None,
                        sha256: None,
                        byte_length: None,
                    },
                },
            ],
        };
        let duplicate = MediaInputs::from_manifest(&duplicate_manifest, &media_root)
            .expect_err("duplicate media_ref fails");
        assert_eq!(duplicate.code(), ErrorCode::InvalidInput);

        fs::remove_dir_all(media_root).expect("temp media root can be removed");
    }

    #[test]
    fn rejects_paths_that_escape_media_root() {
        let manifest = MediaManifest {
            schema: "pptx-compose.media_manifest.v1".to_owned(),
            version: 1,
            media: vec![MediaManifestEntry {
                media_ref: "escape".to_owned(),
                binding: ManifestMediaBinding {
                    path: Some(PathBuf::from("../secret.png")),
                    content_type: Some("image/png".to_owned()),
                    inline: None,
                    sha256: None,
                    byte_length: None,
                },
            }],
        };

        let err = MediaInputs::from_manifest(&manifest, Path::new("media"))
            .expect_err("escaping path fails");
        assert_eq!(err.code(), ErrorCode::UnsafePath);
    }

    #[test]
    fn resolves_inline_base64() {
        let manifest = MediaManifest {
            schema: "pptx-compose.media_manifest.v1".to_owned(),
            version: 1,
            media: vec![MediaManifestEntry {
                media_ref: "inline".to_owned(),
                binding: ManifestMediaBinding {
                    path: None,
                    content_type: None,
                    inline: Some(InlineMedia {
                        encoding: "base64".to_owned(),
                        content_type: "image/png".to_owned(),
                        data: "iVBORw0KGgppbmxpbmUgYnl0ZXM=".to_owned(),
                    }),
                    sha256: None,
                    byte_length: Some(12),
                },
            }],
        };

        let inputs = MediaInputs::from_manifest(&manifest, Path::new("media"))
            .expect("inline manifest binds");
        let resolved = inputs.resolve("inline").expect("inline media resolves");
        assert_eq!(resolved.content_type, "image/png");
        assert_eq!(resolved.bytes, b"\x89PNG\r\n\x1a\ninline bytes");
    }

    fn temp_media_root() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("pptx-compose-media-inputs-{unique}"))
    }
}
