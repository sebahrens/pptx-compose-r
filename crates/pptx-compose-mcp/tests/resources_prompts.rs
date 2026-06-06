use std::{
    collections::BTreeSet,
    fs,
    io::{Cursor, Write},
    path::PathBuf,
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

use pptx_compose_mcp::{
    PptxServer,
    prompts::PromptRegistry,
    resources::{ResourceRegistry, ResourceUri},
    sessions::SessionStore,
    tools::{DEFAULT_TOOL_NAMES, RAW_TOOL_NAMES},
};
use rmcp::{
    ClientHandler, ServerHandler, ServiceExt,
    model::{CallToolRequestParams, CallToolResult, ClientInfo},
};
use serde_json::{Value, json};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

#[derive(Clone, Debug, Default)]
struct TestClient;

impl ClientHandler for TestClient {
    fn get_info(&self) -> ClientInfo {
        ClientInfo::default()
    }
}

#[tokio::test]
async fn resources_and_prompts_match_072_contract() {
    let registry = ResourceRegistry::new();
    let resources = registry.list_resources(None);
    let resource_uris = resources
        .iter()
        .map(|resource| resource.uri.as_str())
        .collect::<Vec<_>>();

    assert!(resource_uris.contains(&"pptx://schemas/agent-view/v1"));
    assert!(resource_uris.contains(&"pptx://schemas/patch/v1"));
    assert!(resource_uris.contains(&"pptx://schemas/patch-report/v1"));
    assert!(resource_uris.contains(&"pptx://schemas/error/v1"));
    assert!(resources.iter().all(|resource| resource.read_only));

    let templates = registry.list_resource_templates();
    let uri_templates = templates
        .iter()
        .map(|template| template.uri_template.as_str())
        .collect::<Vec<_>>();
    assert!(uri_templates.contains(&"pptx://sessions/{session_id}/summary"));
    assert!(uri_templates.contains(&"pptx://sessions/{session_id}/slides"));
    assert!(uri_templates.contains(&"pptx://sessions/{session_id}/slides/{slide_id}"));
    assert!(uri_templates.contains(&"pptx://sessions/{session_id}/elements/{element_id}"));
    assert!(uri_templates.contains(&"pptx://sessions/{session_id}/media/{media_id}/metadata"));
    assert!(uri_templates.contains(&"pptx://sessions/{session_id}/validation/latest"));
    assert!(templates.iter().all(|template| template.read_only));

    let prompts = PromptRegistry::new();
    let prompt_names = prompts
        .list_prompts()
        .into_iter()
        .map(|prompt| prompt.name)
        .collect::<Vec<_>>();
    assert_eq!(
        prompt_names,
        [
            "inspect_deck",
            "edit_deck_safely",
            "replace_text_across_deck",
            "add_image_to_slide",
            "explain_validation_errors",
        ]
    );
    let edit_prompt = prompts
        .build_messages("edit_deck_safely")
        .expect("prompt builds");
    assert!(edit_prompt[0].text.contains("dry-run"));
    assert!(
        !edit_prompt[0]
            .text
            .contains("raw XML replacement as a normal V1 path")
    );

    let sessions = SessionStore::default();
    let fixture = include_bytes!("../../../fixtures/minimal.pptx").to_vec();
    let opened = sessions
        .open_package(
            pptx_compose::PresentationDocument::from_bytes(fixture.clone()).expect("fixture opens"),
            &fixture,
        )
        .expect("session opens");

    let session_resources = registry.list_resources(Some(&opened.session_id));
    assert!(session_resources.iter().all(|resource| resource.read_only));
    assert!(session_resources
        .iter()
        .any(|resource| resource.uri == format!("pptx://sessions/{}/slides", opened.session_id)));

    let uri = ResourceUri::from_str(&format!(
        "pptx://sessions/{}/slides?limit=1",
        opened.session_id
    ))
    .expect("slide list URI parses");
    let content = registry
        .read_resource(&uri, &sessions)
        .await
        .expect("slide resource reads");
    assert_eq!(content.mime_type, "application/json");
    assert_eq!(content.content["view"]["mode"], "slide_page");
    assert_eq!(content.content["view"]["limit"], 1);
    assert!(content.content["view"].get("truncated").is_some());

    let serialized = serde_json::to_string(&content).expect("content serializes");
    assert!(!serialized.contains("\"$binary\""));
    assert!(!serialized.contains("\"base64\""));
    assert!(!serialized.contains("\"data\""));

    let malformed = ResourceUri::from_str("pptx://sessions");
    assert!(malformed.is_err());
    assert_eq!(
        malformed.expect_err("malformed URI rejected").code(),
        pptx_compose::core::error::ErrorCode::InvalidInput
    );
}

#[tokio::test]
async fn mcp_client_can_enumerate_and_read_072_resources_and_prompts() {
    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
    let server_handle = tokio::spawn(async move {
        let service = PptxServer::default()
            .serve(server_transport)
            .await
            .expect("server starts");
        service.waiting().await.expect("server runs");
    });
    let client = TestClient
        .serve(client_transport)
        .await
        .expect("client starts");

    let info = ServerHandler::get_info(&PptxServer::default());
    assert!(info.capabilities.resources.is_some());
    assert!(info.capabilities.prompts.is_some());

    let resources = client.list_resources(None).await.expect("resources list");
    let resource_uris = resources
        .resources
        .iter()
        .map(|resource| resource.raw.uri.as_str())
        .collect::<Vec<_>>();
    for uri in [
        "pptx://schemas/agent-view/v1",
        "pptx://schemas/patch/v1",
        "pptx://schemas/patch-report/v1",
        "pptx://schemas/error/v1",
    ] {
        assert!(resource_uris.contains(&uri), "missing resource {uri}");
        let content = client
            .read_resource(rmcp::model::ReadResourceRequestParams::new(uri))
            .await
            .unwrap_or_else(|error| panic!("resource {uri} reads: {error}"));
        assert_eq!(content.contents.len(), 1);
    }

    let templates = client
        .list_resource_templates(None)
        .await
        .expect("resource templates list");
    let uri_templates = templates
        .resource_templates
        .iter()
        .map(|template| template.raw.uri_template.as_str())
        .collect::<Vec<_>>();
    for uri_template in [
        "pptx://sessions/{session_id}/summary",
        "pptx://sessions/{session_id}/slides",
        "pptx://sessions/{session_id}/slides/{slide_id}",
        "pptx://sessions/{session_id}/elements/{element_id}",
        "pptx://sessions/{session_id}/media/{media_id}/metadata",
        "pptx://sessions/{session_id}/validation/latest",
    ] {
        assert!(
            uri_templates.contains(&uri_template),
            "missing resource template {uri_template}"
        );
    }

    let prompts = client.list_prompts(None).await.expect("prompts list");
    let prompt_names = prompts
        .prompts
        .iter()
        .map(|prompt| prompt.name.as_str())
        .collect::<Vec<_>>();
    for name in [
        "inspect_deck",
        "edit_deck_safely",
        "replace_text_across_deck",
        "add_image_to_slide",
        "explain_validation_errors",
    ] {
        assert!(prompt_names.contains(&name), "missing prompt {name}");
        let prompt = client
            .get_prompt(rmcp::model::GetPromptRequestParams::new(name))
            .await
            .unwrap_or_else(|error| panic!("prompt {name} reads: {error}"));
        assert_eq!(prompt.messages.len(), 1);
    }

    drop(client);
    server_handle.await.expect("server task joins");
}

#[tokio::test]
async fn mcp_client_can_drive_protocol_tools_and_structured_errors() {
    let deck_path = write_temp_deck();
    let (server_handle, client) = start_protocol_client().await;

    let info = ServerHandler::get_info(&PptxServer::default());
    assert!(info.capabilities.tools.is_some());
    assert!(info.capabilities.resources.is_some());
    assert!(info.capabilities.prompts.is_some());

    let tools = client.list_tools(None).await.expect("tools list");
    let tool_names = tools
        .tools
        .iter()
        .map(|tool| tool.name.as_ref().to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(tool_names.len(), DEFAULT_TOOL_NAMES.len());
    for name in DEFAULT_TOOL_NAMES {
        assert!(tool_names.contains(*name), "missing default tool {name}");
    }
    for name in RAW_TOOL_NAMES {
        assert!(
            !tool_names.contains(*name),
            "raw tool exposed by default: {name}"
        );
    }
    for tool in tools.tools {
        assert!(
            tool.input_schema.get("type") == Some(&Value::String("object".to_owned()))
                && tool.input_schema.contains_key("properties"),
            "tool {} lacks an object input schema",
            tool.name
        );
    }

    let opened = call_tool(
        &client,
        "pptx_open",
        json!({
            "path": deck_path.display().to_string()
        }),
    )
    .await;
    assert_success_envelope(&opened);
    let open_result = structured(&opened);
    let session_id = open_result["result"]["session_id"]
        .as_str()
        .expect("open result has session_id")
        .to_owned();
    let document_id = open_result["result"]["document_id"]
        .as_str()
        .expect("open result has document_id")
        .to_owned();
    let revision = open_result["result"]["revision"]
        .as_u64()
        .expect("open result has revision");

    for (tool, arguments, mode) in [
        (
            "pptx_get_document_summary",
            json!({ "session_id": session_id }),
            "deck_summary",
        ),
        (
            "pptx_list_slides",
            json!({ "session_id": session_id, "cursor": null, "limit": 1 }),
            "slide_page",
        ),
        (
            "pptx_get_slide",
            json!({
                "session_id": session_id,
                "slide_id": "slide-1",
                "expected_revision": revision
            }),
            "slide_detail",
        ),
        (
            "pptx_list_elements",
            json!({
                "session_id": session_id,
                "slide_id": "slide-1",
                "cursor": null,
                "limit": 10
            }),
            "slide_detail",
        ),
        (
            "pptx_get_element",
            json!({
                "session_id": session_id,
                "element_id": "slide-1:shape-3"
            }),
            "element_detail",
        ),
    ] {
        let result = call_tool(&client, tool, arguments).await;
        assert_success_envelope(&result);
        assert_eq!(structured(&result)["result"]["view"]["mode"], mode);
    }

    let found = call_tool(
        &client,
        "pptx_find_text",
        json!({
            "session_id": session_id,
            "query": "Original",
            "scope": { "type": "deck" },
            "cursor": null,
            "limit": 10
        }),
    )
    .await;
    assert_success_envelope(&found);
    assert_eq!(structured(&found)["result"]["query"], "Original");

    let validation = call_tool(
        &client,
        "pptx_validate",
        json!({
            "session_id": session_id
        }),
    )
    .await;
    assert_success_envelope(&validation);
    assert!(structured(&validation)["result"].is_object());

    let imported = call_tool(
        &client,
        "pptx_import_media",
        json!({
            "session_id": session_id,
            "media_path": null,
            "inline": {
                "encoding": "base64",
                "data": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII="
            },
            "content_type": "image/png"
        }),
    )
    .await;
    assert_success_envelope(&imported);
    let media_ref = structured(&imported)["result"]["media_ref"]
        .as_str()
        .expect("import result has media_ref")
        .to_owned();
    assert_eq!(structured(&imported)["result"]["sha256"], tiny_png_sha256());

    let media_resource = client
        .read_resource(rmcp::model::ReadResourceRequestParams::new(format!(
            "pptx://sessions/{session_id}/media/{media_ref}/metadata"
        )))
        .await
        .expect("media metadata reads");
    assert_eq!(media_resource.contents.len(), 1);

    let patch = replace_text_patch(&document_id, revision);
    let dry_run = call_tool(
        &client,
        "pptx_validate_patch",
        json!({
            "session_id": session_id,
            "patch": patch
        }),
    )
    .await;
    assert_success_envelope(&dry_run);
    assert_eq!(structured(&dry_run)["result"]["status"], "dry_run_success");

    let applied = call_tool(
        &client,
        "pptx_apply_patch",
        json!({
            "session_id": session_id,
            "client_request_id": "protocol-apply",
            "patch": patch,
            "dry_run": false
        }),
    )
    .await;
    assert_success_envelope(&applied);
    assert_eq!(structured(&applied)["result"]["revision"], revision + 1);
    assert_eq!(
        structured(&applied)["result"]["client_request_id"],
        "protocol-apply"
    );

    let stale = call_tool(
        &client,
        "pptx_apply_patch",
        json!({
            "session_id": session_id,
            "patch": patch,
            "dry_run": false
        }),
    )
    .await;
    assert_eq!(stale.is_error, Some(true));
    let stale_envelope = structured(&stale);
    assert_eq!(stale_envelope["schema"], "pptx-compose.error.v1");
    assert_eq!(stale_envelope["error"]["code"], "stale_patch");
    assert_eq!(
        stale_envelope["error"]["location"]["current_revision"],
        revision + 1
    );

    let exported = call_tool(
        &client,
        "pptx_export",
        json!({
            "session_id": session_id,
            "client_request_id": "protocol-export",
            "expected_revision": revision + 1,
            "output_path": null,
            "overwrite": false
        }),
    )
    .await;
    assert_success_envelope(&exported);
    let exported_result = structured(&exported);
    assert_eq!(
        exported_result["result"]["client_request_id"],
        "protocol-export"
    );
    assert!(
        exported_result["result"]["inline"]["data"]
            .as_str()
            .is_some_and(|data| !data.is_empty())
    );

    let closed = call_tool(
        &client,
        "pptx_close",
        json!({
            "session_id": session_id
        }),
    )
    .await;
    assert_success_envelope(&closed);
    assert_eq!(structured(&closed)["result"]["closed"], true);

    let missing_media = client
        .read_resource(rmcp::model::ReadResourceRequestParams::new(format!(
            "pptx://sessions/{session_id}/media/{media_ref}/metadata"
        )))
        .await;
    assert!(missing_media.is_err(), "closed session media is released");

    drop(client);
    server_handle.await.expect("server task joins");
    fs::remove_file(deck_path).expect("remove temp deck");
}

async fn start_protocol_client() -> (
    tokio::task::JoinHandle<()>,
    rmcp::service::RunningService<rmcp::RoleClient, TestClient>,
) {
    let (server_transport, client_transport) = tokio::io::duplex(256 * 1024);
    let server_handle = tokio::spawn(async move {
        let service = PptxServer::default()
            .serve(server_transport)
            .await
            .expect("server starts");
        service.waiting().await.expect("server runs");
    });
    let client = TestClient
        .serve(client_transport)
        .await
        .expect("client starts");
    (server_handle, client)
}

async fn call_tool(
    client: &rmcp::service::RunningService<rmcp::RoleClient, TestClient>,
    name: &'static str,
    arguments: Value,
) -> CallToolResult {
    let arguments = arguments
        .as_object()
        .cloned()
        .expect("tool arguments are an object");
    client
        .call_tool(CallToolRequestParams::new(name).with_arguments(arguments))
        .await
        .unwrap_or_else(|error| panic!("tool {name} call succeeds at protocol level: {error}"))
}

fn assert_success_envelope(result: &CallToolResult) {
    assert_eq!(
        result.is_error,
        Some(false),
        "unexpected tool error: {:?}",
        result.structured_content
    );
    assert!(!result.content.is_empty(), "tool result has text content");
    let envelope = structured(result);
    assert_eq!(envelope["schema"], "pptx-compose.result.v1");
    assert_eq!(envelope["status"], "success");
    assert!(envelope.get("result").is_some());
}

fn structured(result: &CallToolResult) -> &Value {
    result
        .structured_content
        .as_ref()
        .expect("tool result has structured content")
}

fn replace_text_patch(document_id: &str, base_revision: u64) -> Value {
    json!({
        "schema": pptx_compose::edit::patch::PATCH_SCHEMA,
        "version": pptx_compose::edit::patch::PATCH_VERSION,
        "document_id": document_id,
        "base_revision": base_revision,
        "client_request_id": "protocol-test",
        "operations": [{
            "operation_id": "replace-title",
            "op": "replace_text",
            "element_id": "slide-1:shape-3",
            "text": "Updated title"
        }]
    })
}

fn write_temp_deck() -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("pptx-compose-mcp-{}.pptx", unique_suffix()));
    fs::write(&path, text_deck()).expect("write temp deck");
    path
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after unix epoch")
        .as_nanos()
}

fn text_deck() -> Vec<u8> {
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

fn tiny_png_sha256() -> &'static str {
    "sha256:4b5c5c92cec3b23e6a294fc0eea43234ef5126c5a64f4c6c531ac8430ab0b844"
}
