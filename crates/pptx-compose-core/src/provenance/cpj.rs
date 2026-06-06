use std::collections::BTreeMap;
use std::fmt::Write as _;

use sha2::{Digest, Sha256};

/// Canonical Preimage JSON value used by spec 046 provenance digests.
///
/// This intentionally excludes floating-point values so provenance preimages
/// cannot depend on implementation-specific number formatting.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Cpj {
    Null,
    Int(i64),
    Str(String),
    Array(Vec<Cpj>),
    Object(BTreeMap<String, Cpj>),
}

/// Encodes a CPJ value as canonical UTF-8 JSON bytes.
#[must_use]
pub fn encode(value: &Cpj) -> Vec<u8> {
    let mut output = String::new();
    encode_value(value, &mut output);
    output.into_bytes()
}

fn encode_value(value: &Cpj, output: &mut String) {
    match value {
        Cpj::Null => output.push_str("null"),
        Cpj::Int(integer) => write!(output, "{integer}").expect("writing to String succeeds"),
        Cpj::Str(string) => encode_string(string, output),
        Cpj::Array(values) => {
            output.push('[');
            for (index, item) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                encode_value(item, output);
            }
            output.push(']');
        }
        Cpj::Object(entries) => {
            output.push('{');
            for (index, (key, item)) in entries.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                encode_string(key, output);
                output.push(':');
                encode_value(item, output);
            }
            output.push('}');
        }
    }
}

fn encode_string(value: &str, output: &mut String) {
    output.push('"');
    for char in value.chars() {
        match char {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{0000}'..='\u{001f}' => {
                write!(output, "\\u{:04x}", char as u32).expect("writing to String succeeds")
            }
            _ => output.push(char),
        }
    }
    output.push('"');
}

/// Hashes preimage bytes with SHA-256 and returns lowercase hexadecimal.
#[must_use]
pub fn sha256_hex(preimage: &[u8]) -> String {
    let digest = Sha256::digest(preimage);
    let mut output = String::with_capacity(64);

    for byte in digest {
        write!(&mut output, "{byte:02x}").expect("writing to String succeeds");
    }

    output
}

/// Hashes preimage bytes with SHA-256 and returns the V1 prefixed digest form.
#[must_use]
pub fn digest_prefixed(preimage: &[u8]) -> String {
    let mut output = String::with_capacity("sha256:".len() + 64);
    output.push_str("sha256:");
    output.push_str(&sha256_hex(preimage));
    output
}

/// Hashes the canonical CPJ encoding of a structured provenance preimage.
#[must_use]
pub fn digest_cpj(value: &Cpj) -> String {
    digest_prefixed(&encode(value))
}

#[cfg(test)]
#[test]
fn canonical_encoding() {
    let mut inner = BTreeMap::new();
    inner.insert("z".to_owned(), Cpj::Null);
    inner.insert("a".to_owned(), Cpj::Int(-42));

    let mut root = BTreeMap::new();
    root.insert("zeta".to_owned(), Cpj::Str("café".to_owned()));
    root.insert(
        "array".to_owned(),
        Cpj::Array(vec![
            Cpj::Str("quote\"slash\\control\u{0001}".to_owned()),
            Cpj::Object(inner),
        ]),
    );
    root.insert("alpha".to_owned(), Cpj::Int(0));

    assert_eq!(
        encode(&Cpj::Object(root)),
        r#"{"alpha":0,"array":["quote\"slash\\control\u0001",{"a":-42,"z":null}],"zeta":"café"}"#
            .as_bytes()
    );
    assert_eq!(
        digest_cpj(&Cpj::Object(BTreeMap::new())),
        "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
    );
}
