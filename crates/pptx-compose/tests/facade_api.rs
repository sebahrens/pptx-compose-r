use std::{fs, io::Cursor, io::Write, path::Path};

use pptx_compose::{
    AgentViewOptions, ApplyPatchOptions, MediaInputs, OpenOptions, Patch, PresentationDocument,
    WriteMode, WriteOptions,
    core::{
        provenance::document_id::document_id as provenance_document_id,
        zip::reader::{RawEntry, from_bytes},
    },
    edit::patch::parse_patch,
};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

#[test]
fn exposes_required_070_api_and_defaults() {
    let defaults = WriteOptions::default();
    assert_eq!(
        defaults,
        WriteOptions {
            mode: WriteMode::Preserve,
            overwrite: false,
            validate: true,
            atomic: true,
        }
    );

    let bytes = include_bytes!("../../../fixtures/minimal.pptx");
    let mut from_bytes = PresentationDocument::from_bytes(bytes).expect("fixture opens");
    let _from_reader = PresentationDocument::open_reader(Cursor::new(bytes)).expect("reader opens");

    let root = unique_dir();
    let input = root.join("input.pptx");
    let output = root.join("output.pptx");
    fs::write(&input, bytes).expect("fixture writes");
    let from_path = PresentationDocument::open_path(&input).expect("path opens");
    let _from_path_with_options =
        PresentationDocument::open_path_with_options(&input, OpenOptions::default())
            .expect("path opens with options");

    let _agent_json = from_path.to_agent_json().expect("agent view builds");
    let _agent_json_with_options = from_path
        .to_agent_json_with_options(AgentViewOptions::summary())
        .expect("agent view builds with options");
    let _legacy_json = from_path.to_legacy_json().expect("legacy JSON builds");
    let _validation = from_path.validate().expect("validation report builds");
    let _bytes = from_path.write_vec().expect("default write vec succeeds");
    let _bytes_with_options = from_path
        .write_vec_with_options(WriteOptions::default())
        .expect("write vec with options succeeds");
    from_path
        .write_path_with_options(
            &output,
            WriteOptions {
                overwrite: false,
                ..WriteOptions::default()
            },
        )
        .expect("write path with options succeeds");

    let report = from_bytes
        .apply_patch(noop_patch(bytes), MediaInputs::default())
        .expect("apply_patch returns a report");
    assert_eq!(
        report.status,
        pptx_compose::json::schemas::PatchStatus::Applied
    );
    let report = from_bytes
        .apply_patch_with_options(
            noop_patch(bytes),
            MediaInputs::default(),
            ApplyPatchOptions {
                dry_run: true,
                validate: true,
            },
        )
        .expect("apply_patch_with_options returns a report");
    assert_eq!(
        report.status,
        pptx_compose::json::schemas::PatchStatus::DryRunSuccess
    );

    let output_default = root.join("default-output.pptx");
    from_bytes
        .write_path(&output_default)
        .expect("default write path succeeds");

    fs::remove_dir_all(root).expect("test dir removes");
}

#[test]
fn no_edit_rezip_preserves_canonical_document_id() {
    let bytes = include_bytes!("../../../fixtures/minimal.pptx");
    let document = PresentationDocument::from_bytes(bytes).expect("fixture opens");
    let original_id = document_id(bytes);

    let written = document
        .write_vec_with_options(WriteOptions {
            mode: WriteMode::Deterministic,
            ..WriteOptions::default()
        })
        .expect("deterministic no-edit write succeeds");
    assert_eq!(document_id(&written), original_id);

    let reopened = PresentationDocument::from_bytes(&written).expect("written deck reopens");
    let validation = reopened.validate().expect("validation report builds");
    assert_eq!(validation.document_id, original_id);
}

#[test]
fn replace_text_apply_writes_only_dirtied_slide_part() {
    let bytes = text_deck();
    let mut document = PresentationDocument::from_bytes(&bytes).expect("text deck opens");
    let patch = parse_patch(serde_json::json!({
        "schema": "pptx-compose.patch.v1",
        "version": 1,
        "document_id": document_id(&bytes),
        "base_revision": 1,
        "client_request_id": "replace-text-facade",
        "operations": [{
            "operation_id": "replace-title",
            "op": "replace_text",
            "element_id": "slide-1:shape-3",
            "text": "Updated title"
        }]
    }))
    .expect("patch parses");

    let report = document
        .apply_patch(patch, MediaInputs::default())
        .expect("replace_text applies");
    assert_eq!(report.changed_parts, vec!["ppt/slides/slide1.xml"]);

    let written = document
        .write_vec_with_options(WriteOptions {
            mode: WriteMode::Preserve,
            ..WriteOptions::default()
        })
        .expect("edited deck writes");
    assert_ne!(written, bytes);

    let original_entries = from_bytes(&bytes).expect("original entries read");
    let written_entries = from_bytes(&written).expect("written entries read");
    let changed_parts = original_entries
        .iter()
        .zip(written_entries.iter())
        .filter_map(|(original, written)| {
            if original.bytes == written.bytes {
                None
            } else {
                Some(original.name.zip_entry_name().to_owned())
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(changed_parts, vec!["ppt/slides/slide1.xml"]);

    let slide = written_entries
        .iter()
        .find(|entry| entry.name.zip_entry_name() == "ppt/slides/slide1.xml")
        .expect("slide entry exists");
    let slide_xml = std::str::from_utf8(&slide.bytes).expect("slide XML is UTF-8");
    assert!(slide_xml.contains(">Updated title<"));
}

fn noop_patch(bytes: &[u8]) -> Patch {
    serde_json::from_value(serde_json::json!({
        "schema": "pptx-compose.patch.v1",
        "version": 1,
        "document_id": document_id(bytes),
        "base_revision": 1,
        "client_request_id": "facade-api-noop",
        "operations": []
    }))
    .expect("noop patch parses")
}

fn document_id(bytes: &[u8]) -> String {
    let entries = from_bytes(bytes).expect("package entries read");
    document_id_from_entries(&entries)
}

fn document_id_from_entries(entries: &[RawEntry]) -> String {
    let content_types_bytes = entries
        .iter()
        .find(|entry| !entry.meta.is_dir && entry.name.as_str() == "/[Content_Types].xml")
        .map(|entry| entry.bytes.as_slice())
        .expect("package has content types");
    let ordinary_parts = entries
        .iter()
        .filter(|entry| !entry.meta.is_dir && entry.name.as_str() != "/[Content_Types].xml")
        .map(|entry| (entry.name.clone(), entry.bytes.as_slice()))
        .collect::<Vec<_>>();

    provenance_document_id(&ordinary_parts, content_types_bytes)
}

fn unique_dir() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    let base_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_nanos();
    let temp_dir = std::env::temp_dir();

    for _ in 0..100 {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = temp_dir.join(format!(
            "pptx-compose-facade-api-{}-{base_nanos}-{id}",
            std::process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return path,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("test dir creates: {error}"),
        }
    }

    panic!("could not create a unique facade test directory")
}

#[allow(dead_code)]
fn _assert_no_internal_parser_types_in_primary_api(_path: &Path) {}

fn text_deck() -> Vec<u8> {
    zip_entries(
        [
            ("[Content_Types].xml", content_types().as_bytes()),
            ("_rels/.rels", root_rels().as_bytes()),
            ("ppt/presentation.xml", presentation().as_bytes()),
            (
                "ppt/_rels/presentation.xml.rels",
                presentation_rels().as_bytes(),
            ),
            ("ppt/slides/slide1.xml", text_slide().as_bytes()),
        ],
        CompressionMethod::Stored,
    )
}

fn zip_entries<const N: usize>(entries: [(&str, &[u8]); N], method: CompressionMethod) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut writer = ZipWriter::new(Cursor::new(&mut bytes));
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

fn text_slide() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
      <p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="3" name="Title"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="914400" y="457200"/><a:ext cx="3657600" cy="914400"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>
        <p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Original title</a:t></a:r></a:p></p:txBody>
      </p:sp>
    </p:spTree>
  </p:cSld>
</p:sld>"#
        .to_owned()
}
