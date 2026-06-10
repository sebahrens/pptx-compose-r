use crate::{
    error::{Error, Result},
    opc::{
        package::Package,
        part_name::PartName,
        relationships::{RelationshipSet, TargetMode},
    },
    pptx::ids::ElementKind,
    provenance::text_hash::{self, TextSegment},
    xml::{
        document::{XmlElement, XmlNode},
        parser::parse_document,
    },
};

const DIAGRAM_DRAWING_REL_TYPE: &str =
    "http://schemas.microsoft.com/office/2007/relationships/diagramDrawing";

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RunStyleSummary {
    pub font_size_pt: Option<u32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<String>,
    pub font_color_rgb: Option<String>,
    pub latin_typeface: Option<String>,
    pub language: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Run {
    pub index: u32,
    pub text: String,
    pub style_summary: RunStyleSummary,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Paragraph {
    pub index: u32,
    pub text: String,
    pub normalized: String,
    pub runs: Vec<Run>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextBody {
    pub paragraphs: Vec<Paragraph>,
    pub plain: String,
    pub normalized: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ElementTextProjection {
    pub text: Option<TextBody>,
    pub has_uneditable_chart_cache_text: bool,
    pub diagram_cache_mapping_unsupported: bool,
    pub diagram_drawing_cache_text_absent: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ProjectionSegment {
    Text(String),
    SoftBreak,
}

#[must_use]
pub fn read_text_body(txbody: &XmlElement) -> TextBody {
    let mut paragraphs = Vec::new();
    let mut projection = Vec::new();

    for paragraph_element in child_elements(txbody, "p") {
        let paragraph_index = paragraphs.len() as u32;
        let mut runs = Vec::new();
        let mut paragraph_projection = Vec::new();

        for child in paragraph_element
            .children
            .iter()
            .filter_map(XmlNode::as_element)
        {
            match child.name.local_name.as_str() {
                "r" => {
                    let text = run_text(child);
                    projection.push(ProjectionSegment::Text(text.clone()));
                    paragraph_projection.push(ProjectionSegment::Text(text.clone()));
                    runs.push(Run {
                        index: runs.len() as u32,
                        text,
                        style_summary: run_style_summary(child),
                    });
                }
                "br" => {
                    projection.push(ProjectionSegment::SoftBreak);
                    paragraph_projection.push(ProjectionSegment::SoftBreak);
                }
                _ => {}
            }
        }

        paragraphs.push(Paragraph {
            index: paragraph_index,
            text: plain_projection(&paragraph_projection),
            normalized: normalize_projection(&paragraph_projection),
            runs,
        });
    }

    TextBody {
        paragraphs,
        plain: plain_projection(&projection),
        normalized: normalize_projection(&projection),
    }
}

pub fn project_element_text(
    element: &XmlElement,
    kind: ElementKind,
    slide_rels: &RelationshipSet,
    package: &Package,
) -> Result<ElementTextProjection> {
    if kind == ElementKind::GraphicFrameTable {
        return Ok(ElementTextProjection {
            text: table_text_body(element),
            ..ElementTextProjection::default()
        });
    }
    if let Some(tx_body) = child_element(element, "txBody") {
        return Ok(ElementTextProjection {
            text: Some(read_text_body(tx_body)),
            ..ElementTextProjection::default()
        });
    }
    match kind {
        ElementKind::GraphicFrameChart => project_chart_text(element, slide_rels, package),
        ElementKind::GraphicFrameDiagram => project_diagram_text(element, slide_rels, package),
        _ => Ok(ElementTextProjection::default()),
    }
}

#[must_use]
pub fn merge_text_bodies(bodies: Vec<TextBody>) -> TextBody {
    let mut paragraphs = Vec::new();
    for body in bodies {
        for mut paragraph in body.paragraphs {
            paragraph.index = u32::try_from(paragraphs.len()).unwrap_or(u32::MAX);
            paragraphs.push(paragraph);
        }
    }
    text_body_from_paragraphs(paragraphs)
}

#[must_use]
pub fn read_related_text_body(root: &XmlElement) -> TextBody {
    let mut paragraphs = Vec::new();
    collect_related_paragraphs(root, &mut paragraphs);
    text_body_from_paragraphs(paragraphs)
}

fn project_chart_text(
    element: &XmlElement,
    slide_rels: &RelationshipSet,
    package: &Package,
) -> Result<ElementTextProjection> {
    let mut bodies = Vec::new();
    let mut has_uneditable_chart_cache_text = false;
    for part_name in related_graphic_text_parts(element, slide_rels, package)? {
        let Some(part) = package.parts().get(&part_name) else {
            continue;
        };
        let document = parse_related_text_document(part.bytes(), &part_name)?;
        let Some(root) = document.root_element() else {
            continue;
        };
        has_uneditable_chart_cache_text |= chart_has_uneditable_cache_text(root);
        let body = read_related_text_body(root);
        if !body.normalized.is_empty() {
            bodies.push(body);
        }
    }

    Ok(ElementTextProjection {
        text: (!bodies.is_empty()).then(|| merge_text_bodies(bodies)),
        has_uneditable_chart_cache_text,
        diagram_cache_mapping_unsupported: false,
        diagram_drawing_cache_text_absent: false,
    })
}

fn project_diagram_text(
    element: &XmlElement,
    slide_rels: &RelationshipSet,
    package: &Package,
) -> Result<ElementTextProjection> {
    let Some(data_part_name) = related_graphic_text_parts(element, slide_rels, package)?
        .into_iter()
        .find(|part_name| diagram_part_stem(part_name).is_some())
    else {
        return Ok(ElementTextProjection::default());
    };
    let Some(data_part) = package.parts().get(&data_part_name) else {
        return Ok(ElementTextProjection::default());
    };
    let data_document = parse_related_text_document(data_part.bytes(), &data_part_name)?;
    let Some(data_root) = data_document.root_element() else {
        return Ok(ElementTextProjection::default());
    };
    let data_paragraphs = related_text_paragraph_infos(data_root);
    if data_paragraphs.is_empty() {
        return Ok(ElementTextProjection::default());
    }
    let data_body = related_text_body_from_infos(&data_paragraphs);
    let Some(drawing_part_name) = diagram_drawing_mirror_part(slide_rels, &data_part_name) else {
        return Ok(ElementTextProjection {
            text: Some(data_body),
            ..ElementTextProjection::default()
        });
    };
    let Some(drawing_part) = package.parts().get(&drawing_part_name) else {
        return Ok(ElementTextProjection {
            text: Some(data_body),
            ..ElementTextProjection::default()
        });
    };
    let drawing_document = parse_related_text_document(drawing_part.bytes(), &drawing_part_name)?;
    let Some(drawing_root) = drawing_document.root_element() else {
        return Ok(ElementTextProjection {
            text: Some(data_body),
            ..ElementTextProjection::default()
        });
    };
    let drawing_paragraphs = related_text_paragraph_infos(drawing_root);
    if drawing_paragraphs.is_empty() {
        return Ok(ElementTextProjection {
            text: Some(data_body),
            has_uneditable_chart_cache_text: false,
            diagram_cache_mapping_unsupported: false,
            diagram_drawing_cache_text_absent: true,
        });
    }
    if diagram_cache_mapping_is_unsupported(&data_paragraphs, &drawing_paragraphs) {
        return Ok(ElementTextProjection {
            text: Some(related_text_body_from_infos(&drawing_paragraphs)),
            has_uneditable_chart_cache_text: false,
            diagram_cache_mapping_unsupported: true,
            diagram_drawing_cache_text_absent: false,
        });
    }
    Ok(ElementTextProjection {
        text: Some(data_body),
        ..ElementTextProjection::default()
    })
}

fn table_text_body(element: &XmlElement) -> Option<TextBody> {
    let table = first_descendant(element, "tbl")?;
    let bodies = table
        .children
        .iter()
        .filter_map(XmlNode::as_element)
        .filter(|child| child.name.local_name == "tr")
        .flat_map(|row| {
            row.children
                .iter()
                .filter_map(XmlNode::as_element)
                .filter(|child| child.name.local_name == "tc")
        })
        .filter_map(|cell| child_element(cell, "txBody"))
        .map(read_text_body)
        .filter(|body| !body.normalized.is_empty())
        .collect::<Vec<_>>();
    (!bodies.is_empty()).then(|| merge_text_bodies(bodies))
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

fn related_graphic_text_parts(
    element: &XmlElement,
    slide_rels: &RelationshipSet,
    package: &Package,
) -> Result<Vec<PartName>> {
    let mut parts = Vec::new();
    let mut rel_ids = Vec::new();
    collect_relationship_ids(element, &mut rel_ids);
    for rel_id in rel_ids {
        let Some(relationship) = slide_rels.get(&rel_id) else {
            continue;
        };
        if relationship.target_mode != TargetMode::Internal {
            continue;
        }
        let Some(part_name) = &relationship.resolved_target else {
            return Err(Error::unsupported_package(format!(
                "Relationship {rel_id} from {} did not resolve to a package part.",
                slide_rels.source
            )));
        };
        if package.parts().get(part_name).is_some() && !parts.contains(part_name) {
            parts.push(part_name.clone());
        }
    }
    Ok(parts)
}

fn collect_relationship_ids(element: &XmlElement, output: &mut Vec<String>) {
    for attribute in &element.attributes {
        if matches!(
            attribute.name.prefix.as_deref(),
            Some("r") | Some("relationships")
        ) && !output.contains(&attribute.value)
        {
            output.push(attribute.value.clone());
        }
    }
    for child in element.children.iter().filter_map(XmlNode::as_element) {
        collect_relationship_ids(child, output);
    }
}

fn parse_related_text_document(
    bytes: &[u8],
    part_name: &PartName,
) -> Result<crate::xml::document::XmlDocument> {
    parse_document(bytes).map_err(|source| {
        Error::with_source(
            source.code(),
            format!("Could not parse related text part {part_name}."),
            source,
        )
    })
}

fn diagram_drawing_mirror_part(
    slide_rels: &RelationshipSet,
    data_part_name: &PartName,
) -> Option<PartName> {
    let data_stem = diagram_part_stem(data_part_name)?;
    slide_rels
        .rels
        .iter()
        .filter_map(|relationship| {
            if relationship.target_mode != TargetMode::Internal
                || relationship.rel_type != DIAGRAM_DRAWING_REL_TYPE
            {
                return None;
            }
            relationship.resolved_target.clone()
        })
        .find(|part_name| diagram_part_stem(part_name).as_deref() == Some(data_stem.as_str()))
}

fn diagram_part_stem(part_name: &PartName) -> Option<String> {
    let file_name = part_name.zip_entry_name().rsplit('/').next()?;
    let stem = file_name.strip_suffix(".xml")?;
    stem.strip_prefix("data")
        .or_else(|| stem.strip_prefix("drawing"))
        .map(str::to_owned)
}

#[derive(Clone)]
struct RelatedParagraphInfo {
    model_id: Option<String>,
    paragraph: Paragraph,
}

fn related_text_paragraph_infos(root: &XmlElement) -> Vec<RelatedParagraphInfo> {
    let mut paragraphs = Vec::new();
    collect_related_paragraph_infos(root, &mut paragraphs, None);
    paragraphs
}

fn collect_related_paragraph_infos(
    element: &XmlElement,
    paragraphs: &mut Vec<RelatedParagraphInfo>,
    current_model_id: Option<&str>,
) {
    let model_id = attribute(element, "modelId").or(current_model_id);
    if element.name.local_name == "p" {
        let text_body = read_text_body(&XmlElement {
            name: element.name.clone(),
            attributes: Vec::new(),
            namespaces: element.namespaces.clone(),
            children: vec![XmlNode::Element(element.clone())],
        });
        paragraphs.extend(
            text_body
                .paragraphs
                .into_iter()
                .filter(|paragraph| !paragraph.normalized.is_empty())
                .map(|paragraph| RelatedParagraphInfo {
                    model_id: model_id.map(str::to_owned),
                    paragraph,
                }),
        );
        return;
    }
    for child in element.children.iter().filter_map(XmlNode::as_element) {
        collect_related_paragraph_infos(child, paragraphs, model_id);
    }
}

fn related_text_body_from_infos(infos: &[RelatedParagraphInfo]) -> TextBody {
    text_body_from_paragraphs(
        infos
            .iter()
            .map(|info| info.paragraph.clone())
            .collect::<Vec<_>>(),
    )
}

fn diagram_cache_mapping_is_unsupported(
    data: &[RelatedParagraphInfo],
    drawing: &[RelatedParagraphInfo],
) -> bool {
    let data_model_ids = data
        .iter()
        .map(|paragraph| paragraph.model_id.as_deref())
        .collect::<Vec<_>>();
    let drawing_model_ids = drawing
        .iter()
        .map(|paragraph| paragraph.model_id.as_deref())
        .collect::<Vec<_>>();
    if data_model_ids.iter().all(Option::is_some) && drawing_model_ids.iter().all(Option::is_some) {
        let mut sorted_data = data_model_ids;
        let mut sorted_drawing = drawing_model_ids;
        sorted_data.sort_unstable();
        sorted_drawing.sort_unstable();
        return sorted_data != sorted_drawing;
    }

    let data_text = data
        .iter()
        .map(|paragraph| paragraph.paragraph.normalized.as_str())
        .collect::<Vec<_>>();
    let drawing_text = drawing
        .iter()
        .map(|paragraph| paragraph.paragraph.normalized.as_str())
        .collect::<Vec<_>>();
    if data_text == drawing_text {
        return !all_unique(&data_text) || !all_unique(&drawing_text);
    }
    true
}

fn all_unique(values: &[&str]) -> bool {
    let mut seen = std::collections::BTreeSet::new();
    values.iter().all(|value| seen.insert(*value))
}

fn chart_has_uneditable_cache_text(element: &XmlElement) -> bool {
    matches!(element.name.local_name.as_str(), "strCache" | "strRef")
        || element
            .children
            .iter()
            .filter_map(XmlNode::as_element)
            .any(chart_has_uneditable_cache_text)
}

fn collect_related_paragraphs(element: &XmlElement, paragraphs: &mut Vec<Paragraph>) {
    if element.name.local_name == "p" {
        let text_body = read_text_body(&XmlElement {
            name: element.name.clone(),
            attributes: Vec::new(),
            namespaces: element.namespaces.clone(),
            children: vec![XmlNode::Element(element.clone())],
        });
        if text_body.normalized.is_empty() {
            return;
        }
        for mut paragraph in text_body.paragraphs {
            paragraph.index = u32::try_from(paragraphs.len()).unwrap_or(u32::MAX);
            paragraphs.push(paragraph);
        }
        return;
    }
    for child in element.children.iter().filter_map(XmlNode::as_element) {
        collect_related_paragraphs(child, paragraphs);
    }
}

fn text_body_from_paragraphs(paragraphs: Vec<Paragraph>) -> TextBody {
    let plain = paragraphs
        .iter()
        .map(|paragraph| paragraph.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let normalized = paragraphs
        .iter()
        .map(|paragraph| paragraph.normalized.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    TextBody {
        paragraphs,
        plain,
        normalized,
    }
}

fn child_elements<'a>(
    element: &'a XmlElement,
    local_name: &str,
) -> impl Iterator<Item = &'a XmlElement> {
    element
        .children
        .iter()
        .filter_map(XmlNode::as_element)
        .filter(move |child| child.name.local_name == local_name)
}

fn run_text(run: &XmlElement) -> String {
    let mut text = String::new();
    collect_text_children(run, &mut text);
    text
}

fn run_style_summary(run: &XmlElement) -> RunStyleSummary {
    let Some(run_properties) = child_element(run, "rPr") else {
        return RunStyleSummary::default();
    };

    RunStyleSummary {
        font_size_pt: attribute(run_properties, "sz").and_then(font_size_pt),
        bold: attribute(run_properties, "b").and_then(boolean),
        italic: attribute(run_properties, "i").and_then(boolean),
        underline: attribute(run_properties, "u")
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        font_color_rgb: srgb_color(run_properties),
        latin_typeface: child_element(run_properties, "latin")
            .and_then(|latin| attribute(latin, "typeface"))
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        language: attribute(run_properties, "lang")
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
    }
}

fn child_element<'a>(element: &'a XmlElement, local_name: &str) -> Option<&'a XmlElement> {
    element
        .children
        .iter()
        .filter_map(XmlNode::as_element)
        .find(|child| child.name.local_name == local_name)
}

fn attribute<'a>(element: &'a XmlElement, local_name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.local_name == local_name)
        .map(|attribute| attribute.value.as_str())
}

fn font_size_pt(value: &str) -> Option<u32> {
    value.parse::<u32>().ok().map(|hundredths| hundredths / 100)
}

fn boolean(value: &str) -> Option<bool> {
    match value {
        "1" | "true" | "True" | "TRUE" => Some(true),
        "0" | "false" | "False" | "FALSE" => Some(false),
        _ => None,
    }
}

fn srgb_color(run_properties: &XmlElement) -> Option<String> {
    let solid_fill = child_element(run_properties, "solidFill")?;
    let srgb = child_element(solid_fill, "srgbClr")?;
    attribute(srgb, "val")
        .filter(|value| value.len() == 6 && value.chars().all(|ch| ch.is_ascii_hexdigit()))
        .map(|value| value.to_ascii_uppercase())
}

fn collect_text_children(element: &XmlElement, text: &mut String) {
    for child in &element.children {
        match child {
            XmlNode::Element(element) if element.name.local_name == "t" => {
                collect_text_nodes(element, text);
            }
            XmlNode::Element(element) => collect_text_children(element, text),
            XmlNode::Text(_)
            | XmlNode::CData(_)
            | XmlNode::Comment(_)
            | XmlNode::ProcessingInstruction(_)
            | XmlNode::DocType(_)
            | XmlNode::GeneralRef(_) => {}
        }
    }
}

fn collect_text_nodes(element: &XmlElement, text: &mut String) {
    for child in &element.children {
        match child {
            XmlNode::Text(value) | XmlNode::CData(value) => text.push_str(value),
            XmlNode::Element(element) => collect_text_nodes(element, text),
            XmlNode::GeneralRef(reference) => {
                if let Some(decoded) = decode_text_reference(reference) {
                    text.push(decoded);
                }
            }
            XmlNode::Comment(_) | XmlNode::ProcessingInstruction(_) | XmlNode::DocType(_) => {}
        }
    }
}

fn decode_text_reference(reference: &str) -> Option<char> {
    match reference {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => decode_numeric_reference(reference),
    }
}

fn decode_numeric_reference(reference: &str) -> Option<char> {
    let codepoint = if let Some(hex) = reference
        .strip_prefix("#x")
        .or_else(|| reference.strip_prefix("#X"))
    {
        u32::from_str_radix(hex, 16).ok()?
    } else {
        reference.strip_prefix('#')?.parse::<u32>().ok()?
    };

    char::from_u32(codepoint)
}

fn plain_projection(segments: &[ProjectionSegment]) -> String {
    let mut plain = String::new();
    for segment in segments {
        match segment {
            ProjectionSegment::Text(text) => plain.push_str(text),
            ProjectionSegment::SoftBreak => plain.push('\n'),
        }
    }
    plain
}

fn normalize_projection(segments: &[ProjectionSegment]) -> String {
    let projection = segments
        .iter()
        .map(|segment| match segment {
            ProjectionSegment::Text(text) => TextSegment::Run(text.as_str()),
            ProjectionSegment::SoftBreak => TextSegment::SoftBreak,
        })
        .collect::<Vec<_>>();
    text_hash::normalize_segments(&projection)
}

#[cfg(test)]
#[test]
fn normalized_projection() {
    let raw = format!(
        r#"
<a:txBody xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <a:p>
    <a:r>
      <a:rPr lang="en-US" dirty="0" sz="3200" b="1" i="0" u="sng">
        <a:solidFill><a:srgbClr val="112233"/></a:solidFill>
        <a:latin typeface="Aptos"/>
      </a:rPr>
      <a:t>  Cafe{}	 results  </a:t>
    </a:r>
    <a:br/>
    <a:fld id="{{field}}" type="slidenum">
      <a:t>ignored</a:t>
    </a:fld>
    <a:r>
      <a:t>
Q1
      </a:t>
    </a:r>
  </a:p>
  <a:p>
    <a:r>
      <a:t>  next   line  </a:t>
    </a:r>
  </a:p>
</a:txBody>
"#,
        "\u{301}"
    );
    let document = crate::xml::parser::parse_document(raw.as_bytes()).expect("txBody parses");
    let txbody = document.root_element().expect("txBody root exists");

    let text_body = read_text_body(txbody);

    assert_eq!(text_body.paragraphs.len(), 2);
    assert_eq!(text_body.paragraphs[0].index, 0);
    assert_eq!(text_body.paragraphs[1].index, 1);
    assert_eq!(
        text_body.paragraphs[0].text,
        "  Cafe\u{301}\t results  \n\nQ1\n      "
    );
    assert_eq!(text_body.paragraphs[0].normalized, "Caf\u{e9} results\nQ1");
    assert_eq!(text_body.paragraphs[1].text, "  next   line  ");
    assert_eq!(text_body.paragraphs[1].normalized, "next line");
    assert_eq!(text_body.paragraphs[0].runs.len(), 2);
    assert_eq!(text_body.paragraphs[0].runs[0].index, 0);
    assert_eq!(text_body.paragraphs[0].runs[1].index, 1);
    assert_eq!(text_body.paragraphs[1].runs[0].index, 0);
    assert_eq!(
        text_body.paragraphs[0].runs[0].style_summary,
        RunStyleSummary {
            font_size_pt: Some(32),
            bold: Some(true),
            italic: Some(false),
            underline: Some("sng".to_owned()),
            font_color_rgb: Some("112233".to_owned()),
            latin_typeface: Some("Aptos".to_owned()),
            language: Some("en-US".to_owned()),
        }
    );
    assert_eq!(
        text_body.paragraphs[0].runs[0].text,
        "  Cafe\u{301}\t results  "
    );
    assert_eq!(text_body.paragraphs[0].runs[1].text, "\nQ1\n      ");
    assert_eq!(
        text_body.plain,
        "  Cafe\u{301}\t results  \n\nQ1\n        next   line  "
    );
    assert_eq!(text_body.normalized, "Caf\u{e9} results\nQ1 next line");
}

#[cfg(test)]
#[test]
fn text_projection_decodes_xml_references() {
    let raw = br##"
<a:txBody xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <a:p>
    <a:r>
      <a:t>Profit &amp; Loss &lt;2026&gt; &#8364;5 &#x1F4C8; &unknown;</a:t>
    </a:r>
  </a:p>
</a:txBody>
"##;
    let document = crate::xml::parser::parse_document(raw).expect("txBody parses");
    let txbody = document.root_element().expect("txBody root exists");

    let text_body = read_text_body(txbody);

    assert_eq!(text_body.plain, "Profit & Loss <2026> \u{20ac}5 \u{1f4c8} ");
    assert_eq!(
        text_body.paragraphs[0].runs[0].text,
        "Profit & Loss <2026> \u{20ac}5 \u{1f4c8} "
    );
    assert_eq!(
        text_body.normalized,
        "Profit & Loss <2026> \u{20ac}5 \u{1f4c8}"
    );
}
