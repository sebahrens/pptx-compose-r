use std::collections::BTreeMap;

use crate::{
    opc::part_name::PartName,
    pptx::ids::ElementKind,
    provenance::cpj::{self, Cpj},
};

const FINGERPRINT_SCHEMA: &str = "pptx-compose.fingerprint.v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FingerprintInput {
    pub kind: ElementKind,
    pub part: PartName,
    pub sp_tree_path: Vec<u32>,
    pub group_path: Vec<u32>,
    pub cnvpr_id: Option<i64>,
    pub text_hash: Option<String>,
}

#[must_use]
pub fn fingerprint(input: &FingerprintInput) -> String {
    cpj::digest_cpj(&fingerprint_preimage(input))
}

fn fingerprint_preimage(input: &FingerprintInput) -> Cpj {
    let mut object = BTreeMap::new();
    object.insert("cnvpr_id".to_owned(), optional_int(input.cnvpr_id));
    object.insert("group_path".to_owned(), path_array(&input.group_path));
    object.insert(
        "kind".to_owned(),
        Cpj::Str(fingerprint_kind(input.kind).to_owned()),
    );
    object.insert("part".to_owned(), Cpj::Str(input.part.as_str().to_owned()));
    object.insert("schema".to_owned(), Cpj::Str(FINGERPRINT_SCHEMA.to_owned()));
    object.insert("sp_tree_path".to_owned(), path_array(&input.sp_tree_path));
    Cpj::Object(object)
}

const fn fingerprint_kind(kind: ElementKind) -> &'static str {
    match kind {
        ElementKind::TextBox => "text_box",
        ElementKind::Shape => "shape",
        ElementKind::Picture => "image",
        ElementKind::Group => "group",
        ElementKind::GraphicFrameChart => "chart",
        ElementKind::GraphicFrameTable => "table",
        ElementKind::GraphicFrameDiagram => "diagram",
        ElementKind::GraphicFrameOle => "ole",
        ElementKind::GraphicFrameOther => "graphic_frame",
        ElementKind::Connector => "connector",
        ElementKind::Other => "other",
    }
}

const fn optional_int(value: Option<i64>) -> Cpj {
    match value {
        Some(integer) => Cpj::Int(integer),
        None => Cpj::Null,
    }
}

fn path_array(path: &[u32]) -> Cpj {
    Cpj::Array(path.iter().copied().map(i64::from).map(Cpj::Int).collect())
}

#[cfg(test)]
#[test]
fn guard_value() {
    let part = PartName::from_zip_entry("/ppt/slides/slide1.xml").expect("fixture part is valid");
    let text_hash =
        "sha256:683376e290829b482c2655745caffa7a1dccfa10afaa62dac2b42dd6c68d0f83".to_owned();
    let input = FingerprintInput {
        kind: ElementKind::TextBox,
        part: part.clone(),
        sp_tree_path: vec![3],
        group_path: Vec::new(),
        cnvpr_id: Some(4),
        text_hash: Some(text_hash),
    };

    let base = fingerprint(&input);
    assert_eq!(
        base,
        "sha256:8ff2dea6b48cfc9a424f653a1776e6389916cf34d74300e86a362f1a735ff253"
    );

    let changed_text = FingerprintInput {
        text_hash: Some(
            "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_owned(),
        ),
        ..input.clone()
    };
    assert_eq!(fingerprint(&changed_text), base);

    let changed_cnvpr = FingerprintInput {
        cnvpr_id: Some(5),
        ..input.clone()
    };
    assert_ne!(fingerprint(&changed_cnvpr), base);

    let non_text = FingerprintInput {
        kind: ElementKind::Picture,
        sp_tree_path: vec![4],
        cnvpr_id: Some(9),
        text_hash: None,
        ..input
    };
    let non_text_fingerprint = fingerprint(&non_text);
    assert_ne!(non_text_fingerprint, base);
    assert_eq!(non_text_fingerprint, fingerprint(&non_text));
}
