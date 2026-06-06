use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::schemas::JsonError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewMeta {
    pub mode: String,
    pub limit: u32,
    pub next_cursor: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultLimit {
    pub mode: &'static str,
    pub limit: u32,
}

pub const DEFAULT_LIMITS: &[DefaultLimit] = &[
    DefaultLimit {
        mode: "deck_summary",
        limit: 1,
    },
    DefaultLimit {
        mode: "slide_page",
        limit: 20,
    },
    DefaultLimit {
        mode: "slide_detail",
        limit: 50,
    },
    DefaultLimit {
        mode: "element_detail",
        limit: 1,
    },
    DefaultLimit {
        mode: "media_metadata",
        limit: 50,
    },
    DefaultLimit {
        mode: "validation_report",
        limit: 50,
    },
    DefaultLimit {
        mode: "find_text",
        limit: 50,
    },
];

pub const MAX_PAGE_LIMIT: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorScope<'a> {
    pub document_id: &'a str,
    pub revision: u32,
    pub mode: &'a str,
    pub collection: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cursor {
    pub offset: u32,
    pub salt: String,
}

impl Cursor {
    pub fn encode(offset: u32, scope: CursorScope<'_>) -> Result<String, JsonError> {
        let cursor = Self {
            offset,
            salt: cursor_salt(scope),
        };
        let bytes =
            serde_json::to_vec(&cursor).map_err(|err| JsonError::InvalidCursor(err.to_string()))?;
        Ok(encode_base64(&bytes))
    }

    pub fn decode(encoded: &str, scope: CursorScope<'_>) -> Result<Self, JsonError> {
        let bytes = decode_base64(encoded)?;
        let cursor: Self = serde_json::from_slice(&bytes)
            .map_err(|err| JsonError::InvalidCursor(err.to_string()))?;

        if cursor.salt != cursor_salt(scope) {
            return Err(JsonError::InvalidCursor(
                "Cursor does not match the requested document, revision, or view mode.".to_owned(),
            ));
        }

        Ok(cursor)
    }
}

pub fn default_limit(mode: &str) -> Option<u32> {
    DEFAULT_LIMITS
        .iter()
        .find(|entry| entry.mode == mode)
        .map(|entry| entry.limit)
}

pub fn bounded_limit(mode: &str, requested: Option<u32>) -> Result<u32, JsonError> {
    let limit = requested.unwrap_or_else(|| default_limit(mode).unwrap_or(MAX_PAGE_LIMIT));
    if limit == 0 || limit > MAX_PAGE_LIMIT {
        return Err(JsonError::ResourceLimitExceeded(format!(
            "{mode} limit must be between 1 and {MAX_PAGE_LIMIT}."
        )));
    }
    Ok(limit)
}

pub fn cursor_offset(cursor: Option<&str>, scope: CursorScope<'_>) -> Result<u32, JsonError> {
    match cursor {
        Some(encoded) => Ok(Cursor::decode(encoded, scope)?.offset),
        None => Ok(0),
    }
}

pub fn paginate<'a, T>(
    items: &'a [T],
    limit: u32,
    cursor: Option<&str>,
    scope: CursorScope<'_>,
) -> Result<(Vec<&'a T>, ViewMeta, u32), JsonError> {
    let start = cursor_offset(cursor, scope)?;

    let start = usize::try_from(start).map_err(|err| JsonError::InvalidCursor(err.to_string()))?;
    if start > items.len() {
        return Err(JsonError::InvalidCursor(
            "Cursor offset is outside the requested collection.".to_owned(),
        ));
    }

    let limit_usize =
        usize::try_from(limit).map_err(|err| JsonError::InvalidCursor(err.to_string()))?;
    let end = start.saturating_add(limit_usize).min(items.len());
    let page = items[start..end].iter().collect::<Vec<_>>();
    let remaining = items.len() - end;
    let truncated = remaining > 0;
    let next_cursor = if truncated {
        Some(Cursor::encode(
            u32::try_from(end).map_err(|err| JsonError::InvalidCursor(err.to_string()))?,
            scope,
        )?)
    } else {
        None
    };

    let meta = ViewMeta {
        mode: scope.mode.to_owned(),
        limit,
        next_cursor,
        truncated,
    };
    let omitted_count =
        u32::try_from(remaining).map_err(|err| JsonError::InvalidCursor(err.to_string()))?;

    Ok((page, meta, omitted_count))
}

fn cursor_salt(scope: CursorScope<'_>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(scope.document_id.as_bytes());
    hasher.update([0]);
    hasher.update(scope.revision.to_be_bytes());
    hasher.update([0]);
    hasher.update(scope.mode.as_bytes());
    hasher.update([0]);
    if let Some(collection) = scope.collection {
        hasher.update(collection.as_bytes());
    }
    let digest = hasher.finalize();
    let mut output = String::with_capacity(64);
    for byte in digest {
        output.push(hex_digit(byte >> 4));
        output.push(hex_digit(byte & 0x0f));
    }
    output
}

fn hex_digit(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        10..=15 => (b'a' + (nibble - 10)) as char,
        _ => unreachable!("nibble is masked to four bits"),
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

fn decode_base64(encoded: &str) -> Result<Vec<u8>, JsonError> {
    let bytes = encoded.as_bytes();
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return Err(JsonError::InvalidCursor(
            "Cursor is not valid base64.".to_owned(),
        ));
    }

    let mut output = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks_exact(4) {
        let first = decode_base64_byte(chunk[0])?;
        let second = decode_base64_byte(chunk[1])?;
        let third = if chunk[2] == b'=' {
            None
        } else {
            Some(decode_base64_byte(chunk[2])?)
        };
        let fourth = if chunk[3] == b'=' {
            None
        } else {
            Some(decode_base64_byte(chunk[3])?)
        };

        if third.is_none() && fourth.is_some() {
            return Err(JsonError::InvalidCursor(
                "Cursor has invalid base64 padding.".to_owned(),
            ));
        }

        output.push((first << 2) | (second >> 4));
        if let Some(third) = third {
            output.push(((second & 0b0000_1111) << 4) | (third >> 2));
            if let Some(fourth) = fourth {
                output.push(((third & 0b0000_0011) << 6) | fourth);
            }
        }
    }

    Ok(output)
}

fn decode_base64_byte(byte: u8) -> Result<u8, JsonError> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(JsonError::InvalidCursor(
            "Cursor contains invalid base64 characters.".to_owned(),
        )),
    }
}

#[cfg(test)]
#[test]
fn cursor_roundtrip_and_truncation() {
    use crate::schemas::JsonError;

    let items = (0_u32..42).collect::<Vec<_>>();
    let scope = CursorScope {
        document_id: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        revision: 7,
        mode: "slide_page",
        collection: None,
    };

    let (page, meta, omitted_count) =
        paginate(&items, 20, None, scope).expect("first page paginates");

    assert_eq!(page.len(), 20);
    assert_eq!(*page[0], 0);
    assert_eq!(*page[19], 19);
    assert_eq!(meta.mode, "slide_page");
    assert_eq!(meta.limit, 20);
    assert!(meta.truncated);
    assert_eq!(omitted_count, 22);

    let next_cursor = meta.next_cursor.expect("truncated page has next cursor");
    let decoded = Cursor::decode(&next_cursor, scope).expect("cursor decodes in same scope");
    assert_eq!(decoded.offset, 20);

    let foreign_scope = CursorScope {
        document_id: scope.document_id,
        revision: scope.revision + 1,
        mode: scope.mode,
        collection: scope.collection,
    };
    let error =
        Cursor::decode(&next_cursor, foreign_scope).expect_err("foreign cursor must be rejected");
    assert!(matches!(error, JsonError::InvalidCursor(_)));

    assert_eq!(default_limit("slide_page"), Some(20));
}
