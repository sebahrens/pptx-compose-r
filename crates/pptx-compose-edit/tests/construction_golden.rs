use std::collections::HashMap;

use pptx_compose_core::{
    error::{Error, Result},
    opc::{package::Package, part::Part, part_name::PartName},
    xml::{
        document::{XmlDocument, XmlElement, XmlNode},
        parser::parse_document,
        writer::{WriteMode, WriteOptions, write_document},
    },
    zip::reader::{RawEntry, from_bytes},
};
use pptx_compose_edit::{
    media_inputs::{MediaBinding, MediaInputs, MediaSource},
    operations::{ResolvedSlide, add_image::AddImage, add_text_box::AddTextBox},
    patch::{Bounds, ImageDedupe, ImageFit},
};

mod construction {
    use super::*;

    const MINIMAL_PPTX: &[u8] = include_bytes!("../../../fixtures/minimal.pptx");
    const PIC_EXPECTED: &str = include_str!("../../../fixtures/construction/pic.expected.xml");
    const SP_EXPECTED: &str = include_str!("../../../fixtures/construction/sp.expected.xml");

    #[test]
    fn pic_and_sp_match_golden() -> Result<()> {
        let mut picture_package = minimal_package()?;
        let slide = minimal_slide()?;
        let image = AddImage {
            operation_id: "op-add-image".to_owned(),
            slide_id: slide.slide_id.clone(),
            media_ref: "media-1".to_owned(),
            content_type: "image/png".to_owned(),
            bounds: fixed_bounds(),
            name: None,
            alt_text: None,
            fit: ImageFit::Stretch,
            dedupe: ImageDedupe::Never,
        };
        image.apply(&mut picture_package, &slide, &media_inputs())?;
        assert_eq!(
            inserted_element_xml(&picture_package, &slide.part, "pic")?,
            PIC_EXPECTED
        );

        let mut text_package = minimal_package()?;
        let text_box = AddTextBox {
            operation_id: "op-add-text-box".to_owned(),
            slide_id: slide.slide_id.clone(),
            text: "Hello".to_owned(),
            bounds: fixed_bounds(),
            name: None,
            alt_text: None,
            style: None,
            insert: None,
        };
        text_box.apply(&mut text_package, &slide)?;
        assert_eq!(
            inserted_element_xml(&text_package, &slide.part, "sp")?,
            SP_EXPECTED
        );

        Ok(())
    }

    fn minimal_package() -> Result<Package> {
        package_from_entries(&from_bytes(MINIMAL_PPTX)?)
    }

    fn package_from_entries(entries: &[RawEntry]) -> Result<Package> {
        let mut package = Package::new();
        for entry in entries {
            package.insert_part(Part::from_zip_entry(
                entry.meta.original_name.clone(),
                entry.bytes.clone(),
            )?)?;
        }
        Ok(package)
    }

    fn minimal_slide() -> Result<ResolvedSlide> {
        Ok(ResolvedSlide {
            slide_id: "slide-1".to_owned(),
            part: PartName::from_zip_entry("ppt/slides/slide1.xml")?,
        })
    }

    fn fixed_bounds() -> Bounds {
        Bounds {
            x: 914400,
            y: 457200,
            cx: 1828800,
            cy: 914400,
        }
    }

    fn media_inputs() -> MediaInputs {
        let mut bindings = HashMap::new();
        bindings.insert(
            "media-1".to_owned(),
            MediaBinding {
                content_type: "image/png".to_owned(),
                declared_sha256: None,
                declared_byte_length: None,
                source: MediaSource::Bytes(b"\x89PNG\r\n\x1a\nfixture".to_vec()),
            },
        );
        MediaInputs::new(bindings)
    }

    fn inserted_element_xml(
        package: &Package,
        slide_part: &PartName,
        local_name: &str,
    ) -> Result<String> {
        let part = package.parts().get(slide_part).ok_or_else(|| {
            Error::unsupported_package(format!("Slide part {slide_part} was not found."))
        })?;
        let document = parse_document(part.bytes())?;
        let root = document
            .root_element()
            .ok_or_else(|| Error::malformed_xml("Slide XML does not contain a root element."))?;
        let sp_tree = first_descendant(root, "spTree").ok_or_else(|| {
            Error::unsupported_package("Minimal fixture slide does not contain p:spTree.")
        })?;
        let element = sp_tree
            .children
            .iter()
            .filter_map(XmlNode::as_element)
            .rev()
            .find(|element| element.name.local_name == local_name)
            .ok_or_else(|| {
                Error::unsupported_package(format!(
                    "Inserted {local_name} element was not found in p:spTree."
                ))
            })?;
        serialize_element(element)
    }

    fn serialize_element(element: &XmlElement) -> Result<String> {
        let document = XmlDocument {
            declaration: None,
            nodes: vec![XmlNode::Element(element.clone())],
        };
        let bytes = write_document(
            &document,
            &WriteOptions {
                mode: WriteMode::Preserve,
            },
        )?;
        String::from_utf8(bytes).map_err(|source| {
            Error::parse_error("Serialized inserted element was not UTF-8.", source)
        })
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
}
