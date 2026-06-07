use super::ids::SpTreePath;
use crate::xml::document::{XmlElement, XmlNode};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Bounds {
    pub x: i64,
    pub y: i64,
    pub cx: i64,
    pub cy: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ShapeKind {
    AutoShape,
    TextBox,
    Picture,
    GraphicFrameChart,
    GraphicFrameTable,
    GraphicFrameDiagram,
    GraphicFrameOle,
    GraphicFrameOther,
    Group,
    Connector,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlaceholderRole {
    Title,
    Body,
    CenterTitle,
    Subtitle,
    Date,
    Footer,
    SlideNumber,
    Header,
    Other(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Shape {
    pub kind: ShapeKind,
    pub sp_tree_path: SpTreePath,
    pub cnvpr_id: Option<i64>,
    pub name: Option<String>,
    pub bounds: Option<Bounds>,
    pub rot: Option<i32>,
    pub flip_h: bool,
    pub flip_v: bool,
    pub alt_text: Option<String>,
    pub alt_text_title: Option<String>,
    pub alt_text_description: Option<String>,
    pub placeholder: Option<PlaceholderRole>,
}

#[must_use]
pub fn read_shape(element: &XmlElement, path: SpTreePath) -> Shape {
    let cnvpr = first_descendant(element, "cNvPr");
    let xfrm = transform_element(element);

    let (alt_text_title, alt_text_description) = read_alt_text_fields(cnvpr);
    Shape {
        kind: shape_kind(element),
        sp_tree_path: path,
        cnvpr_id: cnvpr
            .and_then(|element| attr(element, "id"))
            .and_then(parse_i64),
        name: cnvpr
            .and_then(|element| attr(element, "name"))
            .map(str::to_owned),
        bounds: xfrm.and_then(read_bounds),
        rot: xfrm
            .and_then(|element| attr(element, "rot"))
            .and_then(parse_i32),
        flip_h: xfrm
            .and_then(|element| attr(element, "flipH"))
            .is_some_and(parse_bool),
        flip_v: xfrm
            .and_then(|element| attr(element, "flipV"))
            .is_some_and(parse_bool),
        alt_text: alt_text_description
            .clone()
            .or_else(|| alt_text_title.clone()),
        alt_text_title,
        alt_text_description,
        placeholder: first_descendant(element, "ph").and_then(read_placeholder),
    }
}

fn shape_kind(element: &XmlElement) -> ShapeKind {
    match element.name.local_name.as_str() {
        "sp" if is_text_box(element) => ShapeKind::TextBox,
        "sp" => ShapeKind::AutoShape,
        "pic" => ShapeKind::Picture,
        "graphicFrame" => graphic_frame_kind(element),
        "grpSp" => ShapeKind::Group,
        "cxnSp" => ShapeKind::Connector,
        _ => ShapeKind::Other,
    }
}

fn graphic_frame_kind(element: &XmlElement) -> ShapeKind {
    let Some(uri) = child(element, "graphic")
        .and_then(|graphic| child(graphic, "graphicData"))
        .and_then(|graphic_data| attr(graphic_data, "uri"))
    else {
        return ShapeKind::GraphicFrameOther;
    };

    match uri {
        "http://schemas.openxmlformats.org/drawingml/2006/chart" => ShapeKind::GraphicFrameChart,
        "http://schemas.openxmlformats.org/drawingml/2006/table" => ShapeKind::GraphicFrameTable,
        "http://schemas.openxmlformats.org/drawingml/2006/diagram" => {
            ShapeKind::GraphicFrameDiagram
        }
        "http://schemas.openxmlformats.org/presentationml/2006/ole" => ShapeKind::GraphicFrameOle,
        uri if uri.ends_with("/ole") => ShapeKind::GraphicFrameOle,
        _ => ShapeKind::GraphicFrameOther,
    }
}

fn is_text_box(element: &XmlElement) -> bool {
    first_descendant(element, "cNvSpPr")
        .and_then(|element| attr(element, "txBox"))
        .is_some_and(parse_bool)
}

fn transform_element(element: &XmlElement) -> Option<&XmlElement> {
    child(element, "spPr")
        .or_else(|| child(element, "grpSpPr"))
        .and_then(|properties| child(properties, "xfrm"))
        .or_else(|| child(element, "xfrm"))
}

fn read_bounds(xfrm: &XmlElement) -> Option<Bounds> {
    let off = child(xfrm, "off")?;
    let ext = child(xfrm, "ext")?;

    Some(Bounds {
        x: attr(off, "x").and_then(parse_i64)?,
        y: attr(off, "y").and_then(parse_i64)?,
        cx: attr(ext, "cx").and_then(parse_i64)?,
        cy: attr(ext, "cy").and_then(parse_i64)?,
    })
}

fn read_alt_text_fields(cnvpr: Option<&XmlElement>) -> (Option<String>, Option<String>) {
    let Some(cnvpr) = cnvpr else {
        return (None, None);
    };
    (
        attr(cnvpr, "title").map(str::to_owned),
        attr(cnvpr, "descr").map(str::to_owned),
    )
}

fn read_placeholder(ph: &XmlElement) -> Option<PlaceholderRole> {
    let role = attr(ph, "type")?;
    Some(match role {
        "title" => PlaceholderRole::Title,
        "body" => PlaceholderRole::Body,
        "ctrTitle" => PlaceholderRole::CenterTitle,
        "subTitle" => PlaceholderRole::Subtitle,
        "dt" => PlaceholderRole::Date,
        "ftr" => PlaceholderRole::Footer,
        "sldNum" => PlaceholderRole::SlideNumber,
        "hdr" => PlaceholderRole::Header,
        other => PlaceholderRole::Other(other.to_owned()),
    })
}

fn child<'a>(element: &'a XmlElement, local_name: &str) -> Option<&'a XmlElement> {
    element
        .children
        .iter()
        .filter_map(XmlNode::as_element)
        .find(|child| child.name.local_name == local_name)
}

fn first_descendant<'a>(element: &'a XmlElement, local_name: &str) -> Option<&'a XmlElement> {
    for child in element.children.iter().filter_map(XmlNode::as_element) {
        if child.name.local_name == local_name {
            return Some(child);
        }
        if let Some(descendant) = first_descendant(child, local_name) {
            return Some(descendant);
        }
    }
    None
}

fn attr<'a>(element: &'a XmlElement, local_name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.local_name == local_name)
        .map(|attribute| attribute.value.as_str())
}

fn parse_i64(value: &str) -> Option<i64> {
    value.parse().ok()
}

fn parse_i32(value: &str) -> Option<i32> {
    value.parse().ok()
}

fn parse_bool(value: &str) -> bool {
    matches!(value, "1" | "true" | "True" | "TRUE")
}

#[cfg(test)]
mod tests {
    use super::{Bounds, PlaceholderRole, ShapeKind, read_shape};
    use crate::{pptx::ids::SpTreePath, xml::parser::parse_document};

    #[test]
    fn reads_shape_metadata() {
        let raw = br#"
<p:sp xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"
      xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <p:nvSpPr>
    <p:cNvPr id="4" name="Title 1" descr="Quarterly results title"/>
    <p:cNvSpPr txBox="1"/>
    <p:nvPr>
      <p:ph type="title"/>
    </p:nvPr>
  </p:nvSpPr>
  <p:spPr>
    <a:xfrm rot="5400000" flipH="1" flipV="true">
      <a:off x="914400" y="457200"/>
      <a:ext cx="7315200" cy="914400"/>
    </a:xfrm>
  </p:spPr>
</p:sp>
"#;
        let document = parse_document(raw).expect("fixture parses");
        let element = document.root_element().expect("fixture has root");
        let path = SpTreePath {
            sp_tree_path: vec![3],
            group_path: Vec::new(),
        };

        let shape = read_shape(element, path.clone());

        assert_eq!(shape.kind, ShapeKind::TextBox);
        assert_eq!(shape.sp_tree_path, path);
        assert_eq!(shape.cnvpr_id, Some(4));
        assert_eq!(shape.name.as_deref(), Some("Title 1"));
        assert_eq!(
            shape.bounds,
            Some(Bounds {
                x: 914_400,
                y: 457_200,
                cx: 7_315_200,
                cy: 914_400,
            })
        );
        assert_eq!(shape.rot, Some(5_400_000));
        assert!(shape.flip_h);
        assert!(shape.flip_v);
        assert_eq!(shape.alt_text.as_deref(), Some("Quarterly results title"));
        assert_eq!(shape.alt_text_title, None);
        assert_eq!(
            shape.alt_text_description.as_deref(),
            Some("Quarterly results title")
        );
        assert_eq!(shape.placeholder, Some(PlaceholderRole::Title));
    }

    #[test]
    fn reads_alt_text_title_and_description_separately() {
        let raw = br#"
<p:pic xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:nvPicPr>
    <p:cNvPr id="5" name="Picture 1" title="Accessible title" descr="Accessible description"/>
  </p:nvPicPr>
</p:pic>
"#;
        let document = parse_document(raw).expect("fixture parses");
        let element = document.root_element().expect("fixture has root");
        let shape = read_shape(
            element,
            SpTreePath {
                sp_tree_path: vec![4],
                group_path: Vec::new(),
            },
        );

        assert_eq!(shape.alt_text.as_deref(), Some("Accessible description"));
        assert_eq!(shape.alt_text_title.as_deref(), Some("Accessible title"));
        assert_eq!(
            shape.alt_text_description.as_deref(),
            Some("Accessible description")
        );
    }

    #[test]
    fn classifies_graphic_frames_by_graphic_data_uri() {
        let cases = [
            (
                Some("http://schemas.openxmlformats.org/drawingml/2006/chart"),
                ShapeKind::GraphicFrameChart,
            ),
            (
                Some("http://schemas.openxmlformats.org/drawingml/2006/table"),
                ShapeKind::GraphicFrameTable,
            ),
            (
                Some("http://schemas.openxmlformats.org/drawingml/2006/diagram"),
                ShapeKind::GraphicFrameDiagram,
            ),
            (
                Some("http://schemas.openxmlformats.org/presentationml/2006/ole"),
                ShapeKind::GraphicFrameOle,
            ),
            (
                Some("http://schemas.example.test/custom/ole"),
                ShapeKind::GraphicFrameOle,
            ),
            (
                Some("http://schemas.example.test/custom/content"),
                ShapeKind::GraphicFrameOther,
            ),
            (None, ShapeKind::GraphicFrameOther),
        ];

        for (uri, expected) in cases {
            let raw = graphic_frame_xml(uri);
            let document = parse_document(raw.as_bytes()).expect("graphic frame fixture parses");
            let element = document.root_element().expect("fixture has root");
            let shape = read_shape(
                element,
                SpTreePath {
                    sp_tree_path: vec![0],
                    group_path: Vec::new(),
                },
            );

            assert_eq!(shape.kind, expected, "uri: {uri:?}");
        }
    }

    fn graphic_frame_xml(uri: Option<&str>) -> String {
        let graphic_data = uri.map_or_else(
            || "<a:graphicData/>".to_owned(),
            |uri| format!(r#"<a:graphicData uri="{uri}"/>"#),
        );
        format!(
            r#"
<p:graphicFrame xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"
                xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <p:nvGraphicFramePr>
    <p:cNvPr id="7" name="GraphicFrame 1"/>
    <p:cNvGraphicFramePr/>
    <p:nvPr/>
  </p:nvGraphicFramePr>
  <p:xfrm/>
  <a:graphic>{graphic_data}</a:graphic>
</p:graphicFrame>
"#
        )
    }
}
