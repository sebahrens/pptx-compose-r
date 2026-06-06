use std::collections::BTreeSet;

use rmcp::model::Tool;

use crate::PptxServer;

pub const DEFAULT_TOOL_NAMES: &[&str] = &[
    "pptx_open",
    "pptx_get_document_summary",
    "pptx_list_slides",
    "pptx_get_slide",
    "pptx_list_elements",
    "pptx_get_element",
    "pptx_find_text",
    "pptx_import_media",
    "pptx_validate_patch",
    "pptx_apply_patch",
    "pptx_validate",
    "pptx_export",
    "pptx_close",
];

pub const RAW_TOOL_NAMES: &[&str] = &["pptx_get_part_xml", "pptx_replace_part_xml"];

pub fn exposed_tools(server: &PptxServer) -> Vec<Tool> {
    server.tool_routes().list_all()
}

pub fn exposed_tool_names(server: &PptxServer) -> BTreeSet<String> {
    exposed_tools(server)
        .into_iter()
        .map(|tool| tool.name.into_owned())
        .collect()
}

#[cfg(test)]
pub fn assert_raw_tools_disabled_by_default() {
    use crate::{PptxServer, ServerConfig};

    let default_server = PptxServer::default();
    let default_tools = exposed_tool_names(&default_server);

    assert_eq!(default_tools.len(), DEFAULT_TOOL_NAMES.len());
    for name in DEFAULT_TOOL_NAMES {
        assert!(default_tools.contains(*name), "missing default tool {name}");
    }
    for name in RAW_TOOL_NAMES {
        assert!(
            !default_tools.contains(*name),
            "raw tool exposed by default: {name}"
        );
    }

    let raw_enabled_server = PptxServer::with_config(ServerConfig {
        enable_raw_xml_tools: true,
        ..ServerConfig::default()
    });
    let raw_enabled_tools = exposed_tool_names(&raw_enabled_server);

    assert_eq!(
        raw_enabled_tools.len(),
        DEFAULT_TOOL_NAMES.len() + RAW_TOOL_NAMES.len()
    );
    for name in DEFAULT_TOOL_NAMES.iter().chain(RAW_TOOL_NAMES.iter()) {
        assert!(
            raw_enabled_tools.contains(*name),
            "missing raw-enabled tool {name}"
        );
    }
}

#[test]
fn raw_tools_disabled_by_default() {
    assert_raw_tools_disabled_by_default();
}

#[cfg(test)]
fn open_fixture(server: &PptxServer) -> crate::sessions::OpenSession {
    let fixture = include_bytes!("../../../../fixtures/minimal.pptx").to_vec();
    server
        .sessions()
        .open_package(
            pptx_compose::PresentationDocument::from_bytes(fixture.clone()).expect("fixture opens"),
            &fixture,
        )
        .expect("session opens")
}

#[cfg(test)]
fn open_text_fixture(server: &PptxServer) -> crate::sessions::OpenSession {
    let fixture = text_deck();
    server
        .sessions()
        .open_package(
            pptx_compose::PresentationDocument::from_bytes(fixture.clone()).expect("fixture opens"),
            &fixture,
        )
        .expect("session opens")
}

#[tokio::test]
async fn apply_rejects_stale_revision() {
    let server = PptxServer::default();
    let opened = open_text_fixture(&server);

    assert_eq!(opened.revision, 1);

    let first = server
        .pptx_apply_patch(rmcp::handler::server::wrapper::Parameters(
            crate::ApplyPatchInput {
                session_id: opened.session_id.clone(),
                client_request_id: None,
                patch: replace_text_patch(&opened.document_id, opened.revision),
                dry_run: false,
            },
        ))
        .await
        .expect("current revision applies");
    assert_eq!(first.0.0.result["revision"], 2);

    let stale_result = server
        .pptx_apply_patch(rmcp::handler::server::wrapper::Parameters(
            crate::ApplyPatchInput {
                session_id: opened.session_id.clone(),
                client_request_id: None,
                patch: empty_patch(&opened.document_id, opened.revision),
                dry_run: false,
            },
        ))
        .await;
    let Err(stale) = stale_result else {
        panic!("stale revision is rejected");
    };
    let envelope = stale
        .structured_content
        .expect("stale error has structured content");

    assert_eq!(stale.is_error, Some(true));
    assert_eq!(envelope["error"]["code"], "stale_patch");
    assert_eq!(envelope["error"]["location"]["current_revision"], 2);
    assert_eq!(
        server
            .sessions()
            .get(&opened.session_id)
            .expect("session remains open")
            .revision,
        2
    );
}

#[tokio::test]
async fn tools_read_apply_and_export_mutated_deck() {
    let server = PptxServer::default();
    let opened = open_text_fixture(&server);

    let summary = server
        .pptx_get_document_summary(rmcp::handler::server::wrapper::Parameters(
            crate::SummaryInput {
                session_id: opened.session_id.clone(),
            },
        ))
        .await
        .expect("summary reads");
    assert_eq!(summary.0.0.result["schema"], "pptx-compose.agent_view.v1");
    assert_eq!(summary.0.0.result["view"]["mode"], "deck_summary");

    let applied = server
        .pptx_apply_patch(rmcp::handler::server::wrapper::Parameters(
            crate::ApplyPatchInput {
                session_id: opened.session_id.clone(),
                client_request_id: Some("tool-apply-request".to_owned()),
                patch: replace_text_patch(&opened.document_id, opened.revision),
                dry_run: false,
            },
        ))
        .await
        .expect("patch applies");
    assert_eq!(applied.0.0.result["revision"], 2);
    assert_eq!(
        applied.0.0.result["client_request_id"],
        "tool-apply-request"
    );
    assert_eq!(applied.0.0.result["request_id"], "tool-apply-request");
    assert!(
        applied.0.0.result["transaction_id"]
            .as_str()
            .is_some_and(|value| value.starts_with("txn_"))
    );
    assert_eq!(
        applied.0.0.result["changed_parts"],
        serde_json::json!(["ppt/slides/slide1.xml"])
    );
    assert_eq!(
        applied.0.0.result["report"]["client_request_id"],
        "tool-apply-request"
    );
    assert_eq!(
        applied.0.0.result["report"]["transaction_id"],
        applied.0.0.result["transaction_id"]
    );
    assert_eq!(applied.0.0.result["report"]["status"], "applied");

    let slide = server
        .pptx_get_slide(rmcp::handler::server::wrapper::Parameters(
            crate::GetSlideInput {
                session_id: opened.session_id.clone(),
                slide_id: "slide-1".to_owned(),
                expected_revision: Some(2),
            },
        ))
        .await
        .expect("slide reads");
    let elements = slide.0.0.result["slides"][0]["elements"]
        .as_array()
        .expect("slide detail has elements");
    assert!(elements.iter().any(|element| {
        element
            .pointer("/text/plain")
            .and_then(serde_json::Value::as_str)
            == Some("Updated title")
    }));

    let exported = server
        .pptx_export(rmcp::handler::server::wrapper::Parameters(
            crate::ExportInput {
                session_id: opened.session_id,
                client_request_id: Some("tool-export-request".to_owned()),
                expected_revision: Some(2),
                output_path: None,
                inline: true,
                overwrite: false,
            },
        ))
        .await
        .expect("deck exports");
    assert_eq!(
        exported.0.0.result["client_request_id"],
        "tool-export-request"
    );
    assert_eq!(exported.0.0.result["request_id"], "tool-export-request");
    assert!(
        exported.0.0.result["transaction_id"]
            .as_str()
            .is_some_and(|value| value.starts_with("txn_"))
    );
    assert_eq!(
        exported.0.0.result["changed_parts"],
        serde_json::json!(["ppt/slides/slide1.xml"])
    );
    let encoded = exported.0.0.result["inline"]["data"]
        .as_str()
        .expect("inline export has base64 data");
    let bytes = crate::decode_base64(encoded).expect("export base64 decodes");
    let entries = pptx_compose::core::zip::reader::from_bytes(&bytes).expect("export is a PPTX");
    let slide = entries
        .iter()
        .find(|entry| entry.name.zip_entry_name() == "ppt/slides/slide1.xml")
        .expect("slide entry exists");
    let slide_xml = std::str::from_utf8(&slide.bytes).expect("slide XML is UTF-8");
    assert!(slide_xml.contains(">Updated title<"));
}

#[tokio::test]
async fn export_requires_explicit_inline_opt_in_without_path() {
    let server = PptxServer::default();
    let opened = open_fixture(&server);

    let result = server
        .pptx_export(rmcp::handler::server::wrapper::Parameters(
            crate::ExportInput {
                session_id: opened.session_id,
                client_request_id: None,
                expected_revision: Some(opened.revision),
                output_path: None,
                inline: false,
                overwrite: false,
            },
        ))
        .await;
    let Err(error) = result else {
        panic!("pathless export without inline opt-in is rejected");
    };
    let envelope = error
        .structured_content
        .expect("inline opt-in error has structured content");

    assert_eq!(error.is_error, Some(true));
    assert_eq!(envelope["error"]["code"], "invalid_input");
    assert!(
        envelope["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("inline is explicitly true"))
    );
}

#[tokio::test]
async fn inline_export_enforces_configured_byte_limit() {
    let server = PptxServer::with_config(crate::ServerConfig {
        max_inline_export_bytes: 1,
        ..crate::ServerConfig::default()
    });
    let opened = open_fixture(&server);

    let result = server
        .pptx_export(rmcp::handler::server::wrapper::Parameters(
            crate::ExportInput {
                session_id: opened.session_id,
                client_request_id: None,
                expected_revision: Some(opened.revision),
                output_path: None,
                inline: true,
                overwrite: false,
            },
        ))
        .await;
    let Err(error) = result else {
        panic!("oversized inline export is rejected");
    };
    let envelope = error
        .structured_content
        .expect("inline size error has structured content");

    assert_eq!(error.is_error, Some(true));
    assert_eq!(envelope["error"]["code"], "resource_limit_exceeded");
    assert!(
        envelope["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("max_inline_export_bytes"))
    );
}

#[tokio::test]
async fn validate_rejects_wrong_document_id() {
    let server = PptxServer::default();
    let opened = open_fixture(&server);

    let result = server
        .pptx_validate_patch(rmcp::handler::server::wrapper::Parameters(
            crate::ValidatePatchInput {
                session_id: opened.session_id.clone(),
                patch: empty_patch(
                    "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                    opened.revision,
                ),
            },
        ))
        .await;
    let Err(error) = result else {
        panic!("wrong document_id is rejected");
    };
    let envelope = error
        .structured_content
        .expect("wrong-document error has structured content");

    assert_eq!(error.is_error, Some(true));
    assert_eq!(envelope["error"]["code"], "stale_patch");
    assert_eq!(envelope["error"]["location"]["current_revision"], 1);
}

#[cfg(test)]
fn empty_patch(document_id: &str, base_revision: u64) -> pptx_compose::edit::patch::Patch {
    pptx_compose::edit::patch::Patch {
        schema: pptx_compose::edit::patch::PATCH_SCHEMA.to_owned(),
        version: pptx_compose::edit::patch::PATCH_VERSION,
        document_id: document_id.to_owned(),
        base_revision: u32::try_from(base_revision).expect("test fixture revision fits u32"),
        client_request_id: "test-request".to_owned(),
        operations: Vec::new(),
    }
}

#[cfg(test)]
#[cfg(test)]
fn replace_text_patch(document_id: &str, base_revision: u64) -> pptx_compose::edit::patch::Patch {
    let value = serde_json::json!({
        "schema": pptx_compose::edit::patch::PATCH_SCHEMA,
        "version": pptx_compose::edit::patch::PATCH_VERSION,
        "document_id": document_id,
        "base_revision": u32::try_from(base_revision).expect("test fixture revision fits u32"),
        "client_request_id": "test-request",
        "operations": [{
            "operation_id": "replace-title",
            "op": "replace_text",
            "element_id": "slide-1:shape-3",
            "text": "Updated title"
        }]
    });
    serde_json::from_value(value).expect("test patch deserializes")
}

#[cfg(test)]
fn text_deck() -> Vec<u8> {
    use std::io::{Cursor, Write};
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    let content_types = content_types();
    let root_rels = root_rels();
    let presentation = presentation();
    let presentation_rels = presentation_rels();
    let text_slide = text_slide();
    let entries = [
        ("[Content_Types].xml", content_types.as_bytes()),
        ("_rels/.rels", root_rels.as_bytes()),
        ("ppt/presentation.xml", presentation.as_bytes()),
        (
            "ppt/_rels/presentation.xml.rels",
            presentation_rels.as_bytes(),
        ),
        ("ppt/slides/slide1.xml", text_slide.as_bytes()),
    ];
    let mut bytes = Vec::new();
    {
        let mut writer = ZipWriter::new(Cursor::new(&mut bytes));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, data) in entries {
            writer.start_file(name, options).expect("start ZIP entry");
            writer.write_all(data).expect("write ZIP entry");
        }
        writer.finish().expect("finish ZIP");
    }
    bytes
}

#[cfg(test)]
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

#[cfg(test)]
fn root_rels() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
</Relationships>"#
        .to_owned()
}

#[cfg(test)]
fn presentation() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst>
</p:presentation>"#
        .to_owned()
}

#[cfg(test)]
fn presentation_rels() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
</Relationships>"#
        .to_owned()
}

#[cfg(test)]
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
