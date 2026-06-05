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

fn encode_base64(bytes: &[u8]) -> String {
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

#[cfg(test)]
mod tests {
    use super::{InlineBinaryPolicy, inline_payload};

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
}
