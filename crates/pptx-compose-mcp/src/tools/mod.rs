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

#[tokio::test]
async fn apply_rejects_stale_revision() {
    let server = PptxServer::default();
    let opened = open_fixture(&server);

    assert_eq!(opened.revision, 1);

    let first = server
        .pptx_apply_patch(rmcp::handler::server::wrapper::Parameters(
            crate::ApplyPatchInput {
                session_id: opened.session_id.clone(),
                expected_revision: opened.revision,
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
                expected_revision: opened.revision,
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
