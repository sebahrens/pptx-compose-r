use crate::provenance::text_hash::{self, TextSegment};
use crate::xml::document::{XmlElement, XmlNode};

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
    pub runs: Vec<Run>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextBody {
    pub paragraphs: Vec<Paragraph>,
    pub plain: String,
    pub normalized: String,
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

        for child in paragraph_element
            .children
            .iter()
            .filter_map(XmlNode::as_element)
        {
            match child.name.local_name.as_str() {
                "r" => {
                    let text = run_text(child);
                    projection.push(ProjectionSegment::Text(text.clone()));
                    runs.push(Run {
                        index: runs.len() as u32,
                        text,
                        style_summary: run_style_summary(child),
                    });
                }
                "br" => projection.push(ProjectionSegment::SoftBreak),
                _ => {}
            }
        }

        paragraphs.push(Paragraph {
            index: paragraph_index,
            runs,
        });
    }

    TextBody {
        paragraphs,
        plain: plain_projection(&projection),
        normalized: normalize_projection(&projection),
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
            XmlNode::Comment(_)
            | XmlNode::ProcessingInstruction(_)
            | XmlNode::DocType(_)
            | XmlNode::GeneralRef(_) => {}
        }
    }
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
