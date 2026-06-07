use pptx_compose::core::error::{Error, ErrorCode};
use schemars::JsonSchema;
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PromptDescriptor {
    pub name: String,
    pub title: String,
    pub description: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PromptRole {
    User,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PromptMessage {
    pub role: PromptRole,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PromptRegistry;

impl PromptRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    #[must_use]
    pub fn list_prompts(&self) -> Vec<PromptDescriptor> {
        PROMPTS
            .iter()
            .map(|prompt| PromptDescriptor {
                name: prompt.name.to_owned(),
                title: prompt.title.to_owned(),
                description: prompt.description.to_owned(),
            })
            .collect()
    }

    pub fn build_messages(&self, name: &str) -> Result<Vec<PromptMessage>, Error> {
        let prompt = PROMPTS
            .iter()
            .find(|prompt| prompt.name == name)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidInput,
                    format!("Unknown MCP prompt {name}."),
                )
            })?;

        Ok(vec![PromptMessage {
            role: PromptRole::User,
            text: prompt.text.to_owned(),
        }])
    }
}

struct PromptSpec {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    text: &'static str,
}

const PROMPTS: &[PromptSpec] = &[
    PromptSpec {
        name: "inspect_deck",
        title: "Inspect deck",
        description: "Open a PPTX and inspect bounded session resources before planning edits.",
        text: "Open the deck with pptx_open, then inspect pptx://sessions/{session_id}/summary and pptx://sessions/{session_id}/slides. Fetch scoped slide or element resources only as needed. Use structured agent views and validation resources; do not request raw XML for normal V1 work. Report warnings and any unsupported content before proposing edits.",
    },
    PromptSpec {
        name: "edit_deck_safely",
        title: "Edit deck safely",
        description: "Guide the inspect, guarded patch, dry-run, apply, validate, export workflow.",
        text: "Inspect summary, slide, and element resources first. Build a patch with document_id, base_revision, client_request_id, operation_id values, selectors, and guard fields such as current text matches. Run pptx_validate_patch or dry-run apply before a real apply. Apply only after validation succeeds, validate the session again, then export. Prefer unsupported_edit over risky changes and avoid raw XML replacement as a V1 path.",
    },
    PromptSpec {
        name: "replace_text_across_deck",
        title: "Replace text across deck",
        description: "Produce guarded replace_text patches from scoped views.",
        text: "List slides, inspect candidate elements, and prepare replace_text operations only for matching editable text elements. Include the current text match guard for every operation and keep operation_id values stable and descriptive. Dry-run the patch, handle selector or guard failures by re-inspecting, then apply and validate. Do not use raw XML text replacement for supported V1 text edits.",
    },
    PromptSpec {
        name: "add_image_to_slide",
        title: "Add image to slide",
        description: "Stage media and produce add_image patches without inlining binary resources.",
        text: "Inspect the target slide resource and choose EMU bounds from existing layout context. Stage media with pptx_import_media to obtain a media_ref and metadata. Build an add_image operation with slide_id, media_ref, content_type, bounds, optional name, and alt_text. Dry-run before apply, then validate and export. Resource reads expose metadata only by default and should not inline binary bytes.",
    },
    PromptSpec {
        name: "explain_validation_errors",
        title: "Explain validation errors",
        description: "Turn validation findings into safe next actions.",
        text: "Read pptx://sessions/{session_id}/validation/latest and summarize fatal, error, warning, and info findings. Explain blocking findings first, name affected slides, elements, parts, and relationships when present, and recommend the smallest safe next action. If a finding implies unsupported edits or package risk, stop before apply and ask for a supported V1 operation path. Avoid raw XML workflows unless explicitly enabled outside normal V1 editing.",
    },
];
