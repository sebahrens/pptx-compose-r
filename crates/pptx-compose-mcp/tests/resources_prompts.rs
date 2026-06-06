use std::str::FromStr;

use pptx_compose_mcp::{
    PptxServer,
    prompts::PromptRegistry,
    resources::{ResourceRegistry, ResourceUri},
    sessions::SessionStore,
};
use rmcp::{ClientHandler, ServiceExt, model::ClientInfo};

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
