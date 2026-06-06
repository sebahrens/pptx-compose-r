use pptx_compose_core::error::{Error, ErrorCode, Result};

const PNG_CONTENT_TYPE: &str = "image/png";
const JPEG_CONTENT_TYPE: &str = "image/jpeg";
const GIF_CONTENT_TYPE: &str = "image/gif";
const MEDIA_CONTENT_TYPE_MISMATCH: &str = "media_content_type_mismatch";

#[must_use]
pub fn sniff_content_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(PNG_CONTENT_TYPE)
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some(JPEG_CONTENT_TYPE)
    } else if bytes.starts_with(b"GIF8") {
        Some(GIF_CONTENT_TYPE)
    } else {
        None
    }
}

pub fn verify_declared(declared: &str, bytes: &[u8]) -> Result<()> {
    let Some(sniffed) = sniff_content_type(bytes) else {
        return Err(Error::new(
            ErrorCode::UnsupportedMediaType,
            "Media input bytes are not a supported V1 raster image type.",
        ));
    };

    if sniffed != declared {
        return Err(Error::new(
            ErrorCode::UnsupportedMediaType,
            format!(
                "{MEDIA_CONTENT_TYPE_MISMATCH}: declared content type `{declared}` does not match sniffed content type `{sniffed}`."
            ),
        ));
    }

    Ok(())
}

#[cfg(test)]
pub mod detects_and_rejects {
    use pptx_compose_core::error::ErrorCode;

    use super::*;

    #[test]
    fn detects_supported_signatures() {
        assert_eq!(
            sniff_content_type(b"\x89PNG\r\n\x1a\npayload"),
            Some("image/png")
        );
        assert_eq!(
            sniff_content_type(b"\xff\xd8\xff\xe0payload"),
            Some("image/jpeg")
        );
        assert_eq!(sniff_content_type(b"GIF89a payload"), Some("image/gif"));
    }

    #[test]
    fn rejects_mismatched_and_unknown_signatures() {
        let mismatch = verify_declared("image/png", b"\xff\xd8\xff\xe0payload")
            .expect_err("declared PNG over JPEG bytes fails");
        assert_eq!(mismatch.code(), ErrorCode::UnsupportedMediaType);
        assert!(
            mismatch.message().contains("media_content_type_mismatch"),
            "mismatch should carry the canonical finding code"
        );

        let unsupported =
            verify_declared("image/bmp", b"BMpayload").expect_err("BMP bytes are unsupported");
        assert_eq!(unsupported.code(), ErrorCode::UnsupportedMediaType);

        let unknown =
            verify_declared("unknown", b"not an image").expect_err("unknown bytes are unsupported");
        assert_eq!(unknown.code(), ErrorCode::UnsupportedMediaType);
    }
}
