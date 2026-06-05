use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentView {
    pub schema: String,
    pub version: u32,
    pub document_id: String,
    pub revision: u32,
    pub view_id: String,
    pub capabilities: Capabilities,
    pub presentation: PresentationView,
    pub slides: Vec<SlideView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub operations: Vec<String>,
    pub media_content_types: Vec<String>,
    pub units: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PresentationView {
    pub part: String,
    pub slide_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SlideView {
    pub id: String,
    pub index: u32,
    pub ppt_slide_id: Option<u32>,
    pub part: String,
    pub relationship_id: String,
    pub layout_part: String,
    pub part_checksum: String,
    pub elements: Vec<ElementView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ElementView {
    pub id: String,
    pub kind: ElementKind,
    pub slide_id: String,
    pub part: String,
    pub xml_location: XmlLocation,
    pub z_order: u32,
    pub bounds: Bounds,
    pub editable: Editable,
    pub fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<TextView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    TextBox,
    Image,
    Shape,
    Group,
    Table,
    Chart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct XmlLocation {
    pub sp_tree_path: Vec<u32>,
    pub group_path: Vec<u32>,
    pub element_tag: String,
    pub cnvpr_id: u32,
    pub cnvpr_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    pub x: i64,
    pub y: i64,
    pub cx: i64,
    pub cy: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Editable {
    pub text: EditableSupport,
    pub bounds: EditableSupport,
    pub image: EditableSupport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditableSupport {
    pub supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextView {
    pub plain: String,
    pub normalized: String,
    pub paragraphs: Vec<Paragraph>,
    pub text_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Paragraph {
    pub id: String,
    pub text: String,
    pub runs: Vec<Run>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub id: String,
    pub text: String,
    pub style_summary: StyleSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StyleSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size_pt: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImageView {
    pub relationship_id: String,
    pub media_part: String,
    pub content_type: String,
    pub byte_length: u64,
    pub checksum: String,
    pub intrinsic_size_px: IntrinsicSizePx,
    pub shared_media_ref_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntrinsicSizePx {
    pub width: u32,
    pub height: u32,
}

#[cfg(test)]
pub mod schema {
    use serde_json::Value;

    use super::AgentView;
    use crate::schema_versions::AGENT_VIEW_SCHEMA;

    #[test]
    fn roundtrip_matches_spec_042() {
        let input: Value = serde_json::from_str(
            r#"{
                "schema": "pptx-compose.agent_view.v1",
                "version": 1,
                "document_id": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "revision": 1,
                "view_id": "sha256:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
                "capabilities": {
                    "operations": [
                        "replace_text",
                        "add_text_box",
                        "move_resize_element",
                        "set_alt_text",
                        "add_image",
                        "replace_image"
                    ],
                    "media_content_types": ["image/png", "image/jpeg"],
                    "units": "emu"
                },
                "presentation": {
                    "part": "ppt/presentation.xml",
                    "slide_count": 1
                },
                "slides": [
                    {
                        "id": "slide-1",
                        "index": 0,
                        "ppt_slide_id": 256,
                        "part": "ppt/slides/slide1.xml",
                        "relationship_id": "rId7",
                        "layout_part": "ppt/slideLayouts/slideLayout1.xml",
                        "part_checksum": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
                        "elements": [
                            {
                                "id": "slide-1:shape-4",
                                "kind": "text_box",
                                "slide_id": "slide-1",
                                "part": "ppt/slides/slide1.xml",
                                "xml_location": {
                                    "sp_tree_path": [3],
                                    "group_path": [],
                                    "element_tag": "p:sp",
                                    "cnvpr_id": 4,
                                    "cnvpr_name": "Title 1"
                                },
                                "z_order": 3,
                                "bounds": { "x": 914400, "y": 457200, "cx": 7315200, "cy": 914400 },
                                "editable": {
                                    "text": { "supported": true },
                                    "bounds": { "supported": true },
                                    "image": { "supported": false, "reason": "not_picture" }
                                },
                                "fingerprint": "sha256:2222222222222222222222222222222222222222222222222222222222222222",
                                "text": {
                                    "plain": "Quarterly Results",
                                    "normalized": "Quarterly Results",
                                    "paragraphs": [
                                        {
                                            "id": "slide-1:shape-4:p0",
                                            "text": "Quarterly Results",
                                            "runs": [
                                                {
                                                    "id": "slide-1:shape-4:p0:r0",
                                                    "text": "Quarterly Results",
                                                    "style_summary": { "font_size_pt": 32, "bold": false }
                                                }
                                            ]
                                        }
                                    ],
                                    "text_hash": "sha256:3333333333333333333333333333333333333333333333333333333333333333"
                                }
                            },
                            {
                                "id": "slide-1:pic-5",
                                "kind": "image",
                                "slide_id": "slide-1",
                                "part": "ppt/slides/slide1.xml",
                                "xml_location": {
                                    "sp_tree_path": [4],
                                    "group_path": [],
                                    "element_tag": "p:pic",
                                    "cnvpr_id": 5,
                                    "cnvpr_name": "Picture 1"
                                },
                                "z_order": 4,
                                "bounds": { "x": 914400, "y": 1524000, "cx": 3657600, "cy": 2743200 },
                                "editable": {
                                    "text": { "supported": false, "reason": "not_text" },
                                    "bounds": { "supported": true },
                                    "image": { "supported": true }
                                },
                                "fingerprint": "sha256:4444444444444444444444444444444444444444444444444444444444444444",
                                "image": {
                                    "relationship_id": "rId2",
                                    "media_part": "ppt/media/image1.png",
                                    "content_type": "image/png",
                                    "byte_length": 12345,
                                    "checksum": "sha256:5555555555555555555555555555555555555555555555555555555555555555",
                                    "intrinsic_size_px": { "width": 800, "height": 600 },
                                    "shared_media_ref_count": 1
                                }
                            }
                        ]
                    }
                ]
            }"#,
        )
        .expect("test fixture is valid JSON");

        let view: AgentView = serde_json::from_value(input.clone()).expect("spec example parses");
        assert_eq!(view.schema, AGENT_VIEW_SCHEMA);

        let output: Value = serde_json::to_value(view).expect("agent view serializes");
        assert_eq!(output, input);
    }
}
