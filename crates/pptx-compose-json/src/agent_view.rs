use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::binary_encoding::{InlineBinaryPolicy, inline_payload};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intrinsic_size_px: Option<IntrinsicSizePx>,
    pub shared_media_ref_count: u32,
}

impl ImageView {
    pub fn to_json_with_binary_policy(
        &self,
        policy: InlineBinaryPolicy,
        media_bytes: &[u8],
    ) -> Result<Value, serde_json::Error> {
        let mut value = serde_json::to_value(self)?;
        if let Some(payload) = inline_payload(policy, &self.content_type, media_bytes) {
            let payload_value = serde_json::to_value(payload)?;
            if let Some(object) = value.as_object_mut() {
                object.insert("$binary".to_owned(), payload_value);
            }
        }
        Ok(value)
    }
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

#[cfg(test)]
pub mod binary {
    use serde_json::Value;

    use super::{
        AgentView, Bounds, Capabilities, Editable, EditableSupport, ElementKind, ElementView,
        ImageView, IntrinsicSizePx, PresentationView, SlideView, XmlLocation,
    };
    use crate::binary_encoding::InlineBinaryPolicy;
    use crate::schema_versions::AGENT_VIEW_SCHEMA;

    #[test]
    fn no_inline_by_default() {
        let view = AgentView {
            schema: AGENT_VIEW_SCHEMA.to_owned(),
            version: 1,
            document_id: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_owned(),
            revision: 1,
            view_id: "sha256:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
                .to_owned(),
            capabilities: Capabilities {
                operations: vec!["replace_image".to_owned()],
                media_content_types: vec!["image/png".to_owned()],
                units: "emu".to_owned(),
            },
            presentation: PresentationView {
                part: "ppt/presentation.xml".to_owned(),
                slide_count: 1,
            },
            slides: vec![SlideView {
                id: "slide-1".to_owned(),
                index: 0,
                ppt_slide_id: Some(256),
                part: "ppt/slides/slide1.xml".to_owned(),
                relationship_id: "rId7".to_owned(),
                layout_part: "ppt/slideLayouts/slideLayout1.xml".to_owned(),
                part_checksum:
                    "sha256:1111111111111111111111111111111111111111111111111111111111111111"
                        .to_owned(),
                elements: vec![ElementView {
                    id: "slide-1:pic-5".to_owned(),
                    kind: ElementKind::Image,
                    slide_id: "slide-1".to_owned(),
                    part: "ppt/slides/slide1.xml".to_owned(),
                    xml_location: XmlLocation {
                        sp_tree_path: vec![4],
                        group_path: Vec::new(),
                        element_tag: "p:pic".to_owned(),
                        cnvpr_id: 5,
                        cnvpr_name: "Picture 1".to_owned(),
                    },
                    z_order: 4,
                    bounds: Bounds {
                        x: 914400,
                        y: 1524000,
                        cx: 3657600,
                        cy: 2743200,
                    },
                    editable: Editable {
                        text: EditableSupport {
                            supported: false,
                            reason: Some("not_text".to_owned()),
                        },
                        bounds: EditableSupport {
                            supported: true,
                            reason: None,
                        },
                        image: EditableSupport {
                            supported: true,
                            reason: None,
                        },
                    },
                    fingerprint:
                        "sha256:4444444444444444444444444444444444444444444444444444444444444444"
                            .to_owned(),
                    text: None,
                    image: Some(reference_image()),
                }],
            }],
        };

        let json = serde_json::to_value(view).expect("agent view serializes");
        assert_eq!(
            json.pointer("/slides/0/elements/0/image/media_part"),
            Some(&Value::String("ppt/media/image1.png".to_owned()))
        );
        assert_eq!(
            json.pointer("/slides/0/elements/0/image/checksum"),
            Some(&Value::String(
                "sha256:5555555555555555555555555555555555555555555555555555555555555555"
                    .to_owned()
            ))
        );
        assert_eq!(
            json.pointer("/slides/0/elements/0/image/byte_length"),
            Some(&Value::Number(12345_u64.into()))
        );
        assert_no_binary_payload(&json);
        assert_no_long_strings(&json, 96);

        let inline = reference_image()
            .to_json_with_binary_policy(InlineBinaryPolicy::InlineBase64, b"abc123")
            .expect("inline image view serializes");
        assert_eq!(
            inline.pointer("/$binary/data"),
            Some(&Value::String("YWJjMTIz".to_owned()))
        );

        let by_reference = reference_image()
            .to_json_with_binary_policy(InlineBinaryPolicy::ByReference, b"abc123")
            .expect("by-reference image view serializes");
        assert_no_binary_payload(&by_reference);
    }

    fn reference_image() -> ImageView {
        ImageView {
            relationship_id: "rId2".to_owned(),
            media_part: "ppt/media/image1.png".to_owned(),
            content_type: "image/png".to_owned(),
            byte_length: 12345,
            checksum: "sha256:5555555555555555555555555555555555555555555555555555555555555555"
                .to_owned(),
            intrinsic_size_px: Some(IntrinsicSizePx {
                width: 800,
                height: 600,
            }),
            shared_media_ref_count: 1,
        }
    }

    fn assert_no_binary_payload(value: &Value) {
        match value {
            Value::Object(object) => {
                assert!(!object.contains_key("$binary"));
                assert!(!object.contains_key("data"));
                for nested in object.values() {
                    assert_no_binary_payload(nested);
                }
            }
            Value::Array(array) => {
                for nested in array {
                    assert_no_binary_payload(nested);
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }

    fn assert_no_long_strings(value: &Value, max_chars: usize) {
        match value {
            Value::String(text) => assert!(
                text.chars().count() <= max_chars,
                "unexpected long string field: {text}"
            ),
            Value::Object(object) => {
                for nested in object.values() {
                    assert_no_long_strings(nested, max_chars);
                }
            }
            Value::Array(array) => {
                for nested in array {
                    assert_no_long_strings(nested, max_chars);
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }
}
