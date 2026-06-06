use pptx_compose_core::error::{Error, Result};

pub const DEFAULT_MAX_MEDIA_BYTES: u64 = 67_108_864;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaLimits {
    pub max_media_bytes: u64,
}

impl Default for MediaLimits {
    fn default() -> Self {
        Self {
            max_media_bytes: DEFAULT_MAX_MEDIA_BYTES,
        }
    }
}

pub fn check_size(media_ref: &str, byte_length: u64, limits: &MediaLimits) -> Result<()> {
    if byte_length > limits.max_media_bytes {
        return Err(Error::resource_limit_exceeded(format!(
            "Media input `{media_ref}` is {byte_length} bytes, exceeding max_media_bytes {}.",
            limits.max_media_bytes
        )));
    }

    Ok(())
}

#[cfg(test)]
pub mod enforces_max_media_bytes {
    use std::collections::HashMap;

    use pptx_compose_core::error::ErrorCode;

    use super::*;
    use crate::media_inputs::{MediaBinding, MediaInputs, MediaSource};

    #[test]
    fn rejects_default_limit_allows_raised_limit_and_checks_decoded_inline_length() {
        let oversized_len = DEFAULT_MAX_MEDIA_BYTES + 1;
        let oversized_png = png_bytes(oversized_len);
        let default_inputs = inputs_with_limits(
            "oversized",
            MediaSource::Bytes(oversized_png.clone()),
            MediaLimits::default(),
        );

        let err = default_inputs
            .resolve("oversized")
            .expect_err("default media limit rejects limit plus one byte");
        assert_eq!(err.code(), ErrorCode::ResourceLimitExceeded);
        assert!(err.message().contains("oversized"));
        assert!(err.message().contains("67108865"));
        assert!(err.message().contains("67108864"));

        let raised_inputs = inputs_with_limits(
            "oversized",
            MediaSource::Bytes(oversized_png.clone()),
            MediaLimits {
                max_media_bytes: oversized_len,
            },
        );
        let resolved = raised_inputs
            .resolve("oversized")
            .expect("raised media limit allows the same bytes");
        assert_eq!(resolved.bytes.len(), oversized_png.len());

        let inline_inputs = inputs_with_limits(
            "inline",
            MediaSource::InlineBase64("iVBORw0KGgppbmxpbmU=".to_owned()),
            MediaLimits { max_media_bytes: 8 },
        );
        let inline_err = inline_inputs
            .resolve("inline")
            .expect_err("inline media limit checks decoded bytes");
        assert_eq!(inline_err.code(), ErrorCode::ResourceLimitExceeded);
        assert!(inline_err.message().contains("inline"));
        assert!(inline_err.message().contains("14"));
        assert!(inline_err.message().contains("8"));
    }

    fn inputs_with_limits(
        media_ref: &str,
        source: MediaSource,
        limits: MediaLimits,
    ) -> MediaInputs {
        let mut bindings = HashMap::new();
        bindings.insert(
            media_ref.to_owned(),
            MediaBinding {
                content_type: "image/png".to_owned(),
                declared_sha256: None,
                declared_byte_length: None,
                source,
            },
        );
        MediaInputs::with_limits(bindings, limits)
    }

    fn png_bytes(byte_length: u64) -> Vec<u8> {
        let len = usize::try_from(byte_length).expect("test size fits usize");
        let mut bytes = vec![0_u8; len];
        bytes[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        bytes
    }
}
