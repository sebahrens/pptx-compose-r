use serde_json::{Map, Value};

use crate::{
    binary_encoding::{InlineBinaryPayload, decode_base64, encode_base64},
    schemas::JsonError,
};
use pptx_compose_core::opc::{
    content_types::ContentTypes, package::Package, part::Part, part_name::PartName,
};

/// Path-keyed JSON compatibility export for durable file dumps only.
///
/// This preserves the legacy part-name map shape for compatibility. It is not
/// the default agent edit surface and must not be wired into primary V1 patching.
pub fn to_legacy_map(package: &Package) -> Result<Value, JsonError> {
    let mut map = Map::new();

    for part in package.parts().iter() {
        let key = part.name().zip_entry_name().to_owned();
        let value = if is_xml_part(package, part.name()) {
            xml_envelope(part.bytes())?
        } else {
            binary_envelope(package, part)?
        };
        map.insert(key, value);
    }

    Ok(Value::Object(map))
}

pub fn from_legacy_map(value: Value) -> Result<Package, JsonError> {
    let object = value.as_object().ok_or_else(|| {
        JsonError::MalformedLegacyEnvelope("Legacy path map must be a JSON object.".to_owned())
    })?;
    let mut package = Package::new();

    for (path, envelope) in object {
        if path.starts_with('$') {
            return Err(JsonError::MalformedLegacyEnvelope(format!(
                "Legacy path map contains reserved non-part key {path}."
            )));
        }

        let part_name = PartName::from_zip_entry(path).map_err(|err| {
            JsonError::MalformedLegacyEnvelope(format!("Invalid legacy part name {path}: {err}"))
        })?;
        let decoded = decode_envelope(envelope)?;
        package
            .insert_part(Part::from_zip_entry(path, decoded.bytes).map_err(|err| {
                JsonError::MalformedLegacyEnvelope(format!(
                    "Could not reconstruct legacy part {path}: {err}"
                ))
            })?)
            .map_err(|err| {
                JsonError::MalformedLegacyEnvelope(format!(
                    "Could not insert legacy part {path}: {err}"
                ))
            })?;

        if let Some(content_type) = decoded.content_type {
            package
                .content_types_mut()
                .insert_override(part_name, content_type);
        }
    }

    hydrate_content_types(&mut package)?;

    Ok(package)
}

struct DecodedEnvelope {
    bytes: Vec<u8>,
    content_type: Option<String>,
}

fn xml_envelope(bytes: &[u8]) -> Result<Value, JsonError> {
    let xml = std::str::from_utf8(bytes).map_err(|err| {
        JsonError::MalformedLegacyEnvelope(format!(
            "XML legacy envelope requires UTF-8 bytes: {err}"
        ))
    })?;
    Ok(Value::Object(Map::from_iter([(
        "$xml".to_owned(),
        Value::String(xml.to_owned()),
    )])))
}

fn binary_envelope(package: &Package, part: &Part) -> Result<Value, JsonError> {
    let content_type = package
        .content_types()
        .resolve(part.name())
        .ok_or_else(|| {
            JsonError::MalformedLegacyEnvelope(format!(
                "Binary legacy envelope requires a content type for {}.",
                part.name()
            ))
        })?;
    let payload = InlineBinaryPayload {
        encoding: "base64".to_owned(),
        content_type: content_type.to_owned(),
        data: encode_base64(part.bytes()),
    };
    serde_json::to_value(serde_json::json!({ "$binary": payload }))
        .map_err(|err| JsonError::SerializeSchema(err.to_string()))
}

fn decode_envelope(value: &Value) -> Result<DecodedEnvelope, JsonError> {
    let object = value.as_object().ok_or_else(|| {
        JsonError::MalformedLegacyEnvelope(
            "Legacy part value must be an envelope object.".to_owned(),
        )
    })?;

    match (object.get("$xml"), object.get("$binary"), object.len()) {
        (Some(xml), None, 1) => {
            let xml = xml.as_str().ok_or_else(|| {
                JsonError::MalformedLegacyEnvelope(
                    "$xml envelope value must be a string.".to_owned(),
                )
            })?;
            Ok(DecodedEnvelope {
                bytes: xml.as_bytes().to_vec(),
                content_type: None,
            })
        }
        (None, Some(binary), 1) => {
            let payload: InlineBinaryPayload =
                serde_json::from_value(binary.clone()).map_err(|err| {
                    JsonError::MalformedLegacyEnvelope(format!(
                        "$binary envelope is malformed: {err}"
                    ))
                })?;
            if payload.encoding != "base64" {
                return Err(JsonError::MalformedLegacyEnvelope(format!(
                    "Unsupported $binary encoding {}.",
                    payload.encoding
                )));
            }
            Ok(DecodedEnvelope {
                bytes: decode_base64(&payload.data)?,
                content_type: Some(payload.content_type),
            })
        }
        _ => Err(JsonError::MalformedLegacyEnvelope(
            "Legacy part envelope must contain exactly one of $xml or $binary.".to_owned(),
        )),
    }
}

fn is_xml_part(package: &Package, part_name: &PartName) -> bool {
    package
        .content_types()
        .resolve(part_name)
        .is_some_and(is_xml_content_type)
        || part_name.zip_entry_name().ends_with(".xml")
        || part_name.zip_entry_name().ends_with(".rels")
}

fn is_xml_content_type(content_type: &str) -> bool {
    content_type == "application/xml"
        || content_type == "text/xml"
        || content_type.ends_with("+xml")
        || content_type.ends_with(".relationships+xml")
}

fn hydrate_content_types(package: &mut Package) -> Result<(), JsonError> {
    let content_types_part = PartName::from_zip_entry("[Content_Types].xml").map_err(|err| {
        JsonError::MalformedLegacyEnvelope(format!(
            "Could not resolve legacy [Content_Types].xml part name: {err}"
        ))
    })?;
    let Some(part) = package.parts().get(&content_types_part) else {
        return Ok(());
    };
    let content_types = ContentTypes::parse(part.bytes()).map_err(|err| {
        JsonError::MalformedLegacyEnvelope(format!(
            "Could not parse legacy [Content_Types].xml: {err}"
        ))
    })?;
    *package.content_types_mut() = content_types;
    Ok(())
}

#[cfg(test)]
#[test]
fn roundtrip_envelopes() {
    let mut package = Package::new();
    let slide_path = "ppt/slides/slide1.xml";
    let image_path = "ppt/media/image1.png";
    let slide_bytes = br#"<?xml version="1.0"?><p:sld><p:cSld/></p:sld>"#.to_vec();
    let image_bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3];

    package
        .insert_zip_entry(slide_path, slide_bytes.clone())
        .expect("slide inserts");
    package
        .insert_zip_entry(image_path, image_bytes.clone())
        .expect("image inserts");
    package.content_types_mut().insert_override(
        PartName::from_zip_entry(slide_path).expect("slide part name"),
        "application/vnd.openxmlformats-officedocument.presentationml.slide+xml",
    );
    package
        .content_types_mut()
        .insert_default("png", "image/png");

    let legacy = to_legacy_map(&package).expect("legacy map builds");
    assert_eq!(
        legacy.get(slide_path).expect("slide envelope"),
        &serde_json::json!({ "$xml": String::from_utf8(slide_bytes.clone()).expect("xml utf8") })
    );
    assert_eq!(
        legacy.get(image_path).expect("image envelope"),
        &serde_json::json!({
            "$binary": {
                "encoding": "base64",
                "content_type": "image/png",
                "data": "iVBORw0KGgoBAgM="
            }
        })
    );

    let reconstructed = from_legacy_map(legacy).expect("legacy map parses");
    assert_eq!(
        reconstructed
            .parts()
            .get(&PartName::from_zip_entry(slide_path).expect("slide name"))
            .expect("slide part")
            .bytes(),
        slide_bytes.as_slice()
    );
    assert_eq!(
        reconstructed
            .parts()
            .get(&PartName::from_zip_entry(image_path).expect("image name"))
            .expect("image part")
            .bytes(),
        image_bytes.as_slice()
    );
    assert_eq!(
        reconstructed
            .content_types()
            .override_for(&PartName::from_zip_entry(image_path).expect("image name")),
        Some("image/png")
    );
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::from_legacy_map;

    #[test]
    fn rejects_malformed_envelopes() {
        let malformed: Value = json!({ "ppt/media/image1.png": { "$binary": { "encoding": "hex", "content_type": "image/png", "data": "00" } } });
        assert!(from_legacy_map(malformed).is_err());

        let ambiguous: Value =
            json!({ "ppt/slides/slide1.xml": { "$xml": "<p:sld/>", "$binary": {} } });
        assert!(from_legacy_map(ambiguous).is_err());
    }
}
