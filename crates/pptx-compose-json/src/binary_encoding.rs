use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InlineBinaryPolicy {
    #[default]
    ByReference,
    InlineBase64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InlineBinaryPayload {
    pub encoding: String,
    pub content_type: String,
    pub data: String,
}

pub fn inline_payload(
    policy: InlineBinaryPolicy,
    content_type: &str,
    bytes: &[u8],
) -> Option<InlineBinaryPayload> {
    match policy {
        InlineBinaryPolicy::ByReference => None,
        InlineBinaryPolicy::InlineBase64 => Some(InlineBinaryPayload {
            encoding: "base64".to_owned(),
            content_type: content_type.to_owned(),
            data: encode_base64(bytes),
        }),
    }
}

#[must_use]
pub fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);

    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);

        output.push(TABLE[(b0 >> 2) as usize] as char);
        output.push(TABLE[(((b0 & 0b0000_0011) << 4) | (b1 >> 4)) as usize] as char);

        if chunk.len() > 1 {
            output.push(TABLE[(((b1 & 0b0000_1111) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            output.push('=');
        }

        if chunk.len() > 2 {
            output.push(TABLE[(b2 & 0b0011_1111) as usize] as char);
        } else {
            output.push('=');
        }
    }

    output
}

pub fn decode_base64(encoded: &str) -> Result<Vec<u8>, crate::schemas::JsonError> {
    let bytes = encoded.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return Err(crate::schemas::JsonError::MalformedLegacyEnvelope(
            "Base64 data length must be a multiple of four.".to_owned(),
        ));
    }

    let mut output = Vec::with_capacity((bytes.len() / 4) * 3);
    for (chunk_index, chunk) in bytes.chunks(4).enumerate() {
        let pad = chunk.iter().rev().take_while(|byte| **byte == b'=').count();
        if pad > 2 {
            return Err(crate::schemas::JsonError::MalformedLegacyEnvelope(
                "Base64 data contains too much padding.".to_owned(),
            ));
        }
        if pad > 0 && chunk_index != (bytes.len() / 4) - 1 {
            return Err(crate::schemas::JsonError::MalformedLegacyEnvelope(
                "Base64 padding is only valid in the final quartet.".to_owned(),
            ));
        }

        let b0 = decode_base64_byte(chunk[0])?;
        let b1 = decode_base64_byte(chunk[1])?;
        let b2 = if chunk[2] == b'=' {
            0
        } else {
            decode_base64_byte(chunk[2])?
        };
        let b3 = if chunk[3] == b'=' {
            0
        } else {
            decode_base64_byte(chunk[3])?
        };

        if chunk[2] == b'=' && chunk[3] != b'=' {
            return Err(crate::schemas::JsonError::MalformedLegacyEnvelope(
                "Base64 padding must be contiguous.".to_owned(),
            ));
        }

        output.push((b0 << 2) | (b1 >> 4));
        if chunk[2] != b'=' {
            output.push(((b1 & 0b0000_1111) << 4) | (b2 >> 2));
        }
        if chunk[3] != b'=' {
            output.push(((b2 & 0b0000_0011) << 6) | b3);
        }
    }

    Ok(output)
}

fn decode_base64_byte(byte: u8) -> Result<u8, crate::schemas::JsonError> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(crate::schemas::JsonError::MalformedLegacyEnvelope(
            "Base64 data contains a non-base64 character.".to_owned(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{InlineBinaryPolicy, decode_base64, inline_payload};

    #[test]
    fn by_reference_is_default() {
        assert_eq!(
            InlineBinaryPolicy::default(),
            InlineBinaryPolicy::ByReference
        );
        assert_eq!(
            inline_payload(InlineBinaryPolicy::ByReference, "image/png", b"abc"),
            None
        );
    }

    #[test]
    fn inline_base64_encodes_payload() {
        let payload = inline_payload(InlineBinaryPolicy::InlineBase64, "image/png", b"abc123")
            .expect("inline policy returns a payload");

        assert_eq!(payload.encoding, "base64");
        assert_eq!(payload.content_type, "image/png");
        assert_eq!(payload.data, "YWJjMTIz");
    }

    #[test]
    fn base64_decode_rejects_malformed_input() {
        assert_eq!(decode_base64("YWJjMTIz").expect("valid base64"), b"abc123");
        assert!(decode_base64("abc").is_err());
        assert!(decode_base64("ab=c").is_err());
        assert!(decode_base64("!!!!").is_err());
    }
}
