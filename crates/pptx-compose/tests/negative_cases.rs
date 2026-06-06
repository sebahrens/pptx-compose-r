use std::{collections::HashMap, io::Write};

use pptx_compose::{
    ApplyPatchOptions, PresentationDocument, WriteMode, WriteOptions,
    core::{
        error::{ErrorCode, Result},
        opc::{
            package::Package,
            part_name::PartName,
            relationships::{Relationship, RelationshipSource},
        },
        pptx::presentation as core_presentation,
    },
    edit::media_inputs::{MediaBinding, MediaInputs, MediaSource},
    edit::selectors::{self, Selector},
};
use serde_json::{Value, json};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

mod negative {
    use super::*;

    #[test]
    fn all_failures_return_codes_without_mutation() -> Result<()> {
        assert_open_failure(encrypted_cfbf(), ErrorCode::UnsupportedPackage);
        assert_open_failure(encrypted_zip(), ErrorCode::UnsupportedPackage);
        assert_open_failure(unsafe_path_zip(), ErrorCode::UnsafePath);
        assert_open_failure(zip_bomb(), ErrorCode::ResourceLimitExceeded);

        let deck = linked_image_deck();
        assert_patch_failure(
            &deck,
            patch_with_revision(2, "replace_text"),
            None,
            ErrorCode::StalePatch,
        )?;
        assert_patch_failure(
            &deck,
            patch_with_document_id(
                "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            ),
            None,
            ErrorCode::StalePatch,
        )?;
        assert_patch_failure(
            &deck,
            patch_with_operation(json!({
                "operation_id": "missing-element",
                "op": "replace_text",
                "element_id": "slide-1:shape-999",
                "text": "Nope"
            })),
            None,
            ErrorCode::SelectorNotFound,
        )?;
        assert_patch_failure(
            &deck,
            patch_with_operation(json!({
                "operation_id": "missing-media",
                "op": "add_image",
                "slide_id": "slide-1",
                "media_ref": "absent",
                "content_type": "image/png",
                "bounds": { "x": 0, "y": 0, "cx": 1, "cy": 1 }
            })),
            None,
            ErrorCode::MissingMediaRef,
        )?;
        assert_patch_failure(
            &deck,
            patch_with_operation(json!({
                "operation_id": "sniff-mismatch",
                "op": "add_image",
                "slide_id": "slide-1",
                "media_ref": "image",
                "content_type": "image/png",
                "bounds": { "x": 0, "y": 0, "cx": 1, "cy": 1 }
            })),
            Some(media_inputs("image", "image/png", jpeg_bytes(), None)),
            ErrorCode::UnsupportedMediaType,
        )?;
        assert_patch_failure(
            &deck,
            patch_with_operation(json!({
                "operation_id": "checksum-mismatch",
                "op": "add_image",
                "slide_id": "slide-1",
                "media_ref": "image",
                "content_type": "image/png",
                "bounds": { "x": 0, "y": 0, "cx": 1, "cy": 1 }
            })),
            Some(media_inputs(
                "image",
                "image/png",
                png_bytes(),
                Some("sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
            )),
            ErrorCode::MediaChecksumMismatch,
        )?;
        assert_patch_failure(
            &deck,
            patch_with_operation(json!({
                "operation_id": "linked-image",
                "op": "replace_image",
                "element_id": "slide-1:pic-3",
                "media_ref": "image",
                "content_type": "image/png"
            })),
            Some(media_inputs("image", "image/png", png_bytes(), None)),
            ErrorCode::UnsupportedEdit,
        )?;
        assert_ambiguous_selector()?;

        Ok(())
    }
}

fn assert_ambiguous_selector() -> Result<()> {
    let mut package = Package::new();
    package.insert_zip_entry("ppt/presentation.xml", presentation().into_bytes())?;
    package.insert_zip_entry("ppt/slides/slide1.xml", linked_image_slide().into_bytes())?;
    package.insert_zip_entry("ppt/media/image1.png", png_bytes())?;
    package.insert_zip_entry("ppt/media/image2.png", png_bytes())?;
    package.push_relationship(Relationship::internal(
        RelationshipSource::Package,
        "rId1",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument",
        "ppt/presentation.xml",
    ));
    package.push_relationship(Relationship::internal(
        RelationshipSource::Part(part("ppt/presentation.xml")),
        "rId1",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide",
        "slides/slide1.xml",
    ));
    let model = core_presentation::PresentationDocument::open(package)?;
    let error = selectors::resolve(
        &model,
        &Selector::MediaPart {
            part: None,
            guards: None,
        },
    )
    .expect_err("unqualified media selector must be ambiguous");
    assert_eq!(error.code(), ErrorCode::SelectorAmbiguous);
    Ok(())
}

fn assert_open_failure(bytes: Vec<u8>, expected: ErrorCode) {
    let error = PresentationDocument::from_bytes(bytes).expect_err("open must fail");
    assert_eq!(error.code(), expected);
}

fn assert_patch_failure(
    deck: &[u8],
    patch: Value,
    media_inputs: Option<MediaInputs>,
    expected: ErrorCode,
) -> Result<()> {
    let document = PresentationDocument::from_bytes(deck.to_vec())?;
    let before = document.write_vec_with_options(WriteOptions {
        mode: WriteMode::Preserve,
        ..WriteOptions::default()
    })?;
    let error = document
        .apply_patch_with_options(
            &patch,
            ApplyPatchOptions {
                media_inputs: media_inputs.unwrap_or_default(),
                ..ApplyPatchOptions::default()
            },
        )
        .expect_err("patch must fail");
    assert_eq!(error.code(), expected, "{error}");
    let after = document.write_vec_with_options(WriteOptions {
        mode: WriteMode::Preserve,
        ..WriteOptions::default()
    })?;
    assert_eq!(after, before, "failed patch must not mutate package bytes");
    Ok(())
}

fn patch_with_revision(base_revision: u32, op: &str) -> Value {
    let mut patch = patch_with_operation(json!({
        "operation_id": "op",
        "op": op,
        "element_id": "slide-1:shape-3",
        "text": "Updated"
    }));
    patch["base_revision"] = json!(base_revision);
    patch
}

fn patch_with_document_id(document_id: &str) -> Value {
    let mut patch = patch_with_operation(json!({
        "operation_id": "wrong-document",
        "op": "replace_text",
        "element_id": "slide-1:shape-3",
        "text": "Updated"
    }));
    patch["document_id"] = json!(document_id);
    patch
}

fn patch_with_operation(operation: Value) -> Value {
    json!({
        "schema": "pptx-compose.patch.v1",
        "version": 1,
        "document_id": document_id(&linked_image_deck()),
        "base_revision": 1,
        "client_request_id": "negative-cases",
        "operations": [operation]
    })
}

fn document_id(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(bytes);
    let mut output = String::from("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String succeeds");
    }
    output
}

fn media_inputs(
    media_ref: &str,
    content_type: &str,
    bytes: Vec<u8>,
    declared_sha256: Option<&str>,
) -> MediaInputs {
    let mut bindings = HashMap::new();
    bindings.insert(
        media_ref.to_owned(),
        MediaBinding {
            content_type: content_type.to_owned(),
            declared_sha256: declared_sha256.map(str::to_owned),
            declared_byte_length: None,
            source: MediaSource::Bytes(bytes),
        },
    );
    MediaInputs::new(bindings)
}

fn encrypted_cfbf() -> Vec<u8> {
    vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0]
}

fn encrypted_zip() -> Vec<u8> {
    zip_entries(
        [("EncryptedPackage", b"encrypted".as_slice())],
        CompressionMethod::Stored,
    )
}

fn unsafe_path_zip() -> Vec<u8> {
    zip_entries(
        [("../ppt/presentation.xml", b"unsafe".as_slice())],
        CompressionMethod::Stored,
    )
}

fn zip_bomb() -> Vec<u8> {
    zip_entries(
        [("ppt/slides/slide1.xml", vec![b'a'; 100_000].as_slice())],
        CompressionMethod::Deflated,
    )
}

fn linked_image_deck() -> Vec<u8> {
    zip_entries(
        [
            ("[Content_Types].xml", content_types().as_bytes()),
            ("_rels/.rels", root_rels().as_bytes()),
            ("ppt/presentation.xml", presentation().as_bytes()),
            (
                "ppt/_rels/presentation.xml.rels",
                presentation_rels().as_bytes(),
            ),
            ("ppt/slides/slide1.xml", linked_image_slide().as_bytes()),
            (
                "ppt/slides/_rels/slide1.xml.rels",
                slide_link_rels().as_bytes(),
            ),
        ],
        CompressionMethod::Stored,
    )
}

fn zip_entries<const N: usize>(entries: [(&str, &[u8]); N], method: CompressionMethod) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut writer = ZipWriter::new(std::io::Cursor::new(&mut bytes));
        let options = SimpleFileOptions::default().compression_method(method);
        for (name, data) in entries {
            writer.start_file(name, options).expect("start ZIP entry");
            writer.write_all(data).expect("write ZIP entry");
        }
        writer.finish().expect("finish ZIP");
    }
    bytes
}

fn content_types() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
  <Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
</Types>"#
        .to_owned()
}

fn root_rels() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
</Relationships>"#
        .to_owned()
}

fn presentation() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst>
</p:presentation>"#
        .to_owned()
}

fn presentation_rels() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
</Relationships>"#
        .to_owned()
}

fn linked_image_slide() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:cSld>
    <p:spTree>
      <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
      <p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>
      <p:pic>
        <p:nvPicPr><p:cNvPr id="2" name="Linked image"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>
        <p:blipFill><a:blip r:link="rIdLink"/><a:stretch><a:fillRect/></a:stretch></p:blipFill>
        <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>
      </p:pic>
    </p:spTree>
  </p:cSld>
</p:sld>"#
        .to_owned()
}

fn slide_link_rels() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rIdLink" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="https://example.test/image.png" TargetMode="External"/>
</Relationships>"#
        .to_owned()
}

fn png_bytes() -> Vec<u8> {
    b"\x89PNG\r\n\x1a\npayload".to_vec()
}

fn jpeg_bytes() -> Vec<u8> {
    b"\xff\xd8\xff\xe0payload".to_vec()
}

fn part(name: &str) -> PartName {
    PartName::from_zip_entry(name).expect("valid fixture part name")
}
