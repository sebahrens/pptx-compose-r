use super::{
    ids::SpTreePath,
    media::resolve_embedded_media,
    shape::{Bounds, read_shape},
};
use crate::{
    error::{Error, Result},
    opc::{package::Package, part_name::PartName, relationships::RelationshipSet},
    xml::document::{XmlElement, XmlNode},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SizePx {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Picture {
    pub sp_tree_path: SpTreePath,
    pub cnvpr_id: Option<i64>,
    pub name: Option<String>,
    pub alt_text: Option<String>,
    pub bounds: Option<Bounds>,
    pub embed_rel_id: String,
    pub media_part: Option<PartName>,
    pub content_type: String,
    pub byte_length: u64,
    pub shared_media_ref_count: u32,
    pub intrinsic_size_px: Option<SizePx>,
    pub external: bool,
}

pub fn read_picture(
    element: &XmlElement,
    path: SpTreePath,
    slide_rels: &RelationshipSet,
    package: &Package,
) -> Result<Picture> {
    let blip = first_descendant(element, "blip").ok_or_else(|| {
        Error::unsupported_package("Picture element does not contain an a:blip element.")
    })?;
    let shape = read_shape(element, path);

    if let Some(embed_rel_id) = attr(blip, "embed") {
        let media = resolve_embedded_media(embed_rel_id, slide_rels, package)?;
        let intrinsic_size_px = package
            .parts()
            .get(&media.part_name)
            .and_then(|part| intrinsic_size_px(part.bytes()));

        return Ok(Picture {
            sp_tree_path: shape.sp_tree_path,
            cnvpr_id: shape.cnvpr_id,
            name: shape.name,
            alt_text: shape.alt_text,
            bounds: shape.bounds,
            embed_rel_id: embed_rel_id.to_owned(),
            media_part: Some(media.part_name),
            content_type: media.content_type,
            byte_length: media.byte_length,
            shared_media_ref_count: media.shared_ref_count,
            intrinsic_size_px,
            external: false,
        });
    }

    if let Some(link_rel_id) = attr(blip, "link") {
        return Ok(Picture {
            sp_tree_path: shape.sp_tree_path,
            cnvpr_id: shape.cnvpr_id,
            name: shape.name,
            alt_text: shape.alt_text,
            bounds: shape.bounds,
            embed_rel_id: link_rel_id.to_owned(),
            media_part: None,
            content_type: String::new(),
            byte_length: 0,
            shared_media_ref_count: 0,
            intrinsic_size_px: None,
            external: true,
        });
    }

    Err(Error::unsupported_package(
        "Picture blip has neither r:embed nor r:link.",
    ))
}

fn intrinsic_size_px(bytes: &[u8]) -> Option<SizePx> {
    png_size(bytes)
        .or_else(|| jpeg_size(bytes))
        .or_else(|| gif_size(bytes))
}

fn png_size(bytes: &[u8]) -> Option<SizePx> {
    const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 24 || bytes.get(..8)? != PNG_SIGNATURE {
        return None;
    }

    Some(SizePx {
        width: u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?),
        height: u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?),
    })
}

fn jpeg_size(bytes: &[u8]) -> Option<SizePx> {
    if bytes.get(..2)? != b"\xff\xd8" {
        return None;
    }

    let mut offset = 2usize;
    while offset + 3 < bytes.len() {
        while bytes.get(offset) == Some(&0xff) {
            offset += 1;
        }
        let marker = *bytes.get(offset)?;
        offset += 1;

        if marker == 0xd9 || marker == 0xda {
            return None;
        }
        if marker_has_no_payload(marker) {
            continue;
        }

        let length = usize::from(u16::from_be_bytes(
            bytes.get(offset..offset + 2)?.try_into().ok()?,
        ));
        if length < 2 || offset.checked_add(length)? > bytes.len() {
            return None;
        }

        if is_sof_marker(marker) {
            return Some(SizePx {
                height: u32::from(u16::from_be_bytes(
                    bytes.get(offset + 3..offset + 5)?.try_into().ok()?,
                )),
                width: u32::from(u16::from_be_bytes(
                    bytes.get(offset + 5..offset + 7)?.try_into().ok()?,
                )),
            });
        }

        offset += length;
    }

    None
}

fn gif_size(bytes: &[u8]) -> Option<SizePx> {
    if bytes.len() < 10 || !(bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return None;
    }

    Some(SizePx {
        width: u32::from(u16::from_le_bytes(bytes.get(6..8)?.try_into().ok()?)),
        height: u32::from(u16::from_le_bytes(bytes.get(8..10)?.try_into().ok()?)),
    })
}

fn marker_has_no_payload(marker: u8) -> bool {
    marker == 0x01 || (0xd0..=0xd7).contains(&marker)
}

fn is_sof_marker(marker: u8) -> bool {
    matches!(
        marker,
        0xc0 | 0xc1 | 0xc2 | 0xc3 | 0xc5 | 0xc6 | 0xc7 | 0xc9 | 0xca | 0xcb | 0xcd | 0xce | 0xcf
    )
}

fn child_elements(element: &XmlElement) -> impl Iterator<Item = &XmlElement> {
    element.children.iter().filter_map(XmlNode::as_element)
}

fn first_descendant<'a>(element: &'a XmlElement, local_name: &str) -> Option<&'a XmlElement> {
    for child in child_elements(element) {
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

#[cfg(test)]
#[test]
fn reads_embedded_image() {
    use crate::{
        opc::{
            package::Package,
            relationships::{Relationship, RelationshipSet, RelationshipSource},
        },
        pptx::{ids::SpTreePath, media::IMAGE_REL_TYPE, shape::Bounds},
        xml::parser::parse_document,
    };

    let mut package = Package::new();
    let media_part = test_part("ppt/media/image1.png");
    package
        .content_types_mut()
        .insert_default("png", "image/png");
    package
        .insert_zip_entry("ppt/media/image1.png", one_by_one_png())
        .expect("media part inserts");

    let slide_part = test_part("ppt/slides/slide1.xml");
    let slide_rels = RelationshipSet {
        source: slide_part.clone(),
        rels: vec![Relationship::internal(
            RelationshipSource::Part(slide_part),
            "rId2",
            IMAGE_REL_TYPE,
            "../media/image1.png",
        )],
    };
    package.push_relationship(slide_rels.rels[0].clone());

    let document = parse_document(picture_xml()).expect("picture fixture parses");
    let element = document.root_element().expect("picture fixture has root");
    let path = SpTreePath {
        sp_tree_path: vec![2],
        group_path: Vec::new(),
    };

    let picture =
        read_picture(element, path.clone(), &slide_rels, &package).expect("embedded image reads");

    assert_eq!(picture.sp_tree_path, path);
    assert_eq!(picture.cnvpr_id, Some(7));
    assert_eq!(picture.name.as_deref(), Some("Picture 1"));
    assert_eq!(picture.alt_text.as_deref(), Some("tiny png"));
    assert_eq!(
        picture.bounds,
        Some(Bounds {
            x: 10,
            y: 20,
            cx: 30,
            cy: 40,
        })
    );
    assert_eq!(picture.embed_rel_id, "rId2");
    assert_eq!(picture.media_part, Some(media_part));
    assert_eq!(picture.content_type, "image/png");
    assert_eq!(picture.byte_length, one_by_one_png().len() as u64);
    assert_eq!(picture.shared_media_ref_count, 1);
    assert_eq!(
        picture.intrinsic_size_px,
        Some(SizePx {
            width: 1,
            height: 1
        })
    );
    assert!(!picture.external);
}

#[cfg(test)]
#[test]
fn reads_external_image_without_synthetic_media_part() {
    use crate::{
        opc::{package::Package, relationships::RelationshipSet},
        pptx::ids::SpTreePath,
        xml::parser::parse_document,
    };

    let package = Package::new();
    let slide_part = test_part("ppt/slides/slide1.xml");
    let slide_rels = RelationshipSet {
        source: slide_part,
        rels: Vec::new(),
    };
    let document = parse_document(external_picture_xml()).expect("picture fixture parses");
    let element = document.root_element().expect("picture fixture has root");
    let path = SpTreePath {
        sp_tree_path: vec![2],
        group_path: Vec::new(),
    };

    let picture =
        read_picture(element, path, &slide_rels, &package).expect("external picture reads");

    assert!(picture.external);
    assert_eq!(picture.embed_rel_id, "rIdExternal");
    assert_eq!(picture.media_part, None);
    assert_eq!(picture.content_type, "");
    assert_eq!(picture.byte_length, 0);
    assert_eq!(picture.shared_media_ref_count, 0);
    assert_eq!(picture.intrinsic_size_px, None);
}

#[cfg(test)]
fn test_part(name: &str) -> PartName {
    PartName::from_zip_entry(name).expect("valid fixture part")
}

#[cfg(test)]
fn picture_xml() -> &'static [u8] {
    br#"
<p:pic xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"
       xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:nvPicPr>
    <p:cNvPr id="7" name="Picture 1" descr="tiny png"/>
    <p:cNvPicPr/>
    <p:nvPr/>
  </p:nvPicPr>
  <p:blipFill>
    <a:blip r:embed="rId2"/>
    <a:stretch><a:fillRect/></a:stretch>
  </p:blipFill>
  <p:spPr>
    <a:xfrm>
      <a:off x="10" y="20"/>
      <a:ext cx="30" cy="40"/>
    </a:xfrm>
  </p:spPr>
</p:pic>
"#
}

#[cfg(test)]
fn external_picture_xml() -> &'static [u8] {
    br#"
<p:pic xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"
       xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:nvPicPr>
    <p:cNvPr id="7" name="Picture 1" descr="linked image"/>
    <p:cNvPicPr/>
    <p:nvPr/>
  </p:nvPicPr>
  <p:blipFill>
    <a:blip r:link="rIdExternal"/>
    <a:stretch><a:fillRect/></a:stretch>
  </p:blipFill>
  <p:spPr/>
</p:pic>
"#
}

#[cfg(test)]
fn one_by_one_png() -> Vec<u8> {
    vec![
        0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0x00, 0x00, 0x00, 0x0d, b'I', b'H',
        b'D', b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00,
        0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0a, b'I', b'D', b'A', b'T', 0x78, 0x9c, 0x63,
        0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00,
        b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82,
    ]
}
