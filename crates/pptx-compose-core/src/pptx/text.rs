use crate::provenance::text_hash::{self, TextSegment};
use crate::xml::document::{XmlElement, XmlNode};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RunStyleSummary {}

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
                        style_summary: RunStyleSummary::default(),
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
      <a:rPr b="1"/>
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
        RunStyleSummary {}
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
