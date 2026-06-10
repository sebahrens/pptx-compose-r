use std::collections::HashMap;

use pptx_compose_core::{
    error::{Error, ErrorCode, Result},
    opc::{
        package::Package,
        part::Part,
        part_name::PartName,
        relationships::{Relationship, RelationshipSource},
    },
    pptx::{ids::ElementKind, media::IMAGE_REL_TYPE},
    xml::{
        document::{XmlDocument, XmlElement, XmlNode},
        parser::parse_document,
        writer::{WriteMode, WriteOptions, write_document},
    },
    zip::reader::{RawEntry, from_bytes},
};
use pptx_compose_edit::{
    media_inputs::{MediaBinding, MediaInputs, MediaSource},
    operations::{
        ResolvedElement, ResolvedNotesSlide, ResolvedSlide,
        add_image::AddImage,
        add_text_box::AddTextBox,
        move_resize::MoveResize,
        replace_image::ReplaceImage,
        replace_text::{ReplaceNotesText, ReplaceText},
        set_alt_text::SetAltText,
    },
    patch::{
        Bounds, FormatPolicy, ImageDedupe, ImageFit, InsertOptions, ReplaceTextMode, TextAlign,
        TextRunStyle, ZOrder, ZOrderKeyword,
    },
    selectors::RunSelector,
};

mod construction_golden {
    use super::*;

    const MINIMAL_PPTX: &[u8] = include_bytes!("../../../fixtures/minimal.pptx");
    const PIC_EXPECTED: &str = include_str!("../../../fixtures/construction/pic.expected.xml");
    const PIC_RELS_EXPECTED: &str =
        include_str!("../../../fixtures/construction/pic.rels.expected.xml");
    const SP_EXPECTED: &str = include_str!("../../../fixtures/construction/sp.expected.xml");
    const REPLACE_TEXT_EXPECTED: &str =
        include_str!("../../../fixtures/construction/replace_text.sp.expected.xml");
    const REPLACE_IMAGE_RELS_EXPECTED: &str =
        include_str!("../../../fixtures/construction/replace_image.rels.expected.xml");
    const MOVE_RESIZE_EXPECTED: &str =
        include_str!("../../../fixtures/construction/move_resize.sp.expected.xml");
    const SET_ALT_TEXT_EXPECTED: &str =
        include_str!("../../../fixtures/construction/set_alt_text.pic.expected.xml");

    #[test]
    fn generated_drawingml_matches_fixtures() -> Result<()> {
        let mut picture_package = minimal_package()?;
        let slide = minimal_slide()?;
        let clean_slide_bytes = part_bytes(&picture_package, &slide.part)?.to_vec();
        let clean_presentation_part = PartName::from_zip_entry("ppt/presentation.xml")?;
        let clean_presentation_bytes =
            part_bytes(&picture_package, &clean_presentation_part)?.to_vec();
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
            insert: None,
        };
        let image_effects = image.apply(&mut picture_package, &slide, &media_inputs())?;
        let picture_slide_xml = part_xml(&picture_package, &slide.part)?;
        assert_slide_namespaces(&picture_slide_xml);
        let picture_xml = inserted_element_xml(&picture_package, &slide.part, "pic")?;
        assert_eq!(picture_xml, golden(PIC_EXPECTED));
        assert_contains_bounds(&picture_xml, &fixed_bounds());
        assert!(
            picture_xml.contains(r#"<p:cNvPr id="2" name="Picture 2"/>"#),
            "add_image must allocate max(existing cNvPr id) + 1 and use the default name"
        );
        assert!(
            picture_xml.contains(r#"<a:picLocks noChangeAspect="1"/>"#),
            "add_image must emit the fixed V1 picture lock"
        );
        assert!(
            picture_xml.contains(r#"<a:stretch><a:fillRect/></a:stretch>"#),
            "add_image must use deterministic stretch fill"
        );
        assert_ne!(
            part_bytes(&picture_package, &slide.part)?,
            clean_slide_bytes
        );
        assert_eq!(
            part_bytes(&picture_package, &clean_presentation_part)?,
            clean_presentation_bytes
        );
        assert_eq!(
            image_effects.changed_parts,
            vec![
                "ppt/slides/slide1.xml",
                "ppt/slides/_rels/slide1.xml.rels",
                "ppt/media/image1.png",
                "[Content_Types].xml",
            ]
        );
        assert_eq!(
            image_effects.created_element_ids,
            vec!["slide-1:pic-2".to_owned()]
        );
        assert_eq!(
            part_xml(
                &picture_package,
                &PartName::from_zip_entry("ppt/slides/_rels/slide1.xml.rels")?
            )?,
            golden(PIC_RELS_EXPECTED)
        );
        assert!(picture_xml.contains(r#"<a:blip r:embed="rId1"/>"#));
        let image_rel = picture_package
            .relationships()
            .set_for(&slide.part)
            .and_then(|rels| rels.rels.iter().find(|rel| rel.id == "rId1"))
            .ok_or_else(|| Error::unsupported_package("Inserted image relationship not found."))?;
        assert_eq!(
            image_rel.source,
            RelationshipSource::Part(slide.part.clone())
        );
        assert_eq!(image_rel.rel_type, IMAGE_REL_TYPE);
        assert_eq!(image_rel.target, "../media/image1.png");
        assert_eq!(
            image_rel.resolved_target.as_ref(),
            Some(&PartName::from_zip_entry("ppt/media/image1.png")?)
        );
        assert_eq!(
            picture_package
                .content_types()
                .resolve(&PartName::from_zip_entry("ppt/media/image1.png")?),
            Some("image/png")
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
        let text_slide_xml = part_xml(&text_package, &slide.part)?;
        assert_slide_namespaces(&text_slide_xml);
        let text_box_xml = inserted_element_xml(&text_package, &slide.part, "sp")?;
        assert_eq!(text_box_xml, golden(SP_EXPECTED));
        assert_contains_bounds(&text_box_xml, &fixed_bounds());
        assert!(
            text_box_xml.contains(r#"<p:cNvPr id="2" name="TextBox 2"/>"#),
            "add_text_box must allocate max(existing cNvPr id) + 1 and use the default name"
        );
        assert!(
            text_box_xml.contains(r#"<p:cNvSpPr txBox="1"/>"#),
            "add_text_box must mark the shape as a text box"
        );
        assert!(
            text_box_xml.contains(
                r#"<a:bodyPr wrap="square" rtlCol="0"><a:spAutoFit/></a:bodyPr><a:lstStyle/>"#
            ),
            "add_text_box must emit the deterministic default body style"
        );
        assert!(
            text_box_xml.contains(r#"<a:rPr lang="en-US" dirty="0"/>"#),
            "add_text_box default run style must inherit font size, color, and typeface"
        );

        Ok(())
    }

    #[test]
    fn add_text_box_honors_z_order_front_back_and_index() -> Result<()> {
        assert_text_box_insert_order(None, &["Back", "Front", "Inserted"], &[3])?;
        assert_text_box_insert_order(
            Some(InsertOptions {
                z_order: Some(ZOrder::Keyword(ZOrderKeyword::Front)),
            }),
            &["Back", "Front", "Inserted"],
            &[3],
        )?;
        assert_text_box_insert_order(
            Some(InsertOptions {
                z_order: Some(ZOrder::Keyword(ZOrderKeyword::Back)),
            }),
            &["Inserted", "Back", "Front"],
            &[1],
        )?;
        assert_text_box_insert_order(
            Some(InsertOptions {
                z_order: Some(ZOrder::Index(2)),
            }),
            &["Back", "Inserted", "Front"],
            &[2],
        )?;
        Ok(())
    }

    #[test]
    fn add_text_box_normalizes_carriage_returns_and_rejects_illegal_text() -> Result<()> {
        let mut package = minimal_package()?;
        let slide = minimal_slide()?;
        let text_box = AddTextBox {
            operation_id: "op-add-text-box".to_owned(),
            slide_id: slide.slide_id.clone(),
            text: "Line 1\r\nLine 2\rLine 3".to_owned(),
            bounds: fixed_bounds(),
            name: None,
            alt_text: None,
            style: None,
            insert: None,
        };

        let effects = text_box.apply(&mut package, &slide)?;
        let text_box_xml = inserted_element_xml(&package, &slide.part, "sp")?;

        assert!(text_box_xml.contains("<a:t>Line 1</a:t>"));
        assert!(text_box_xml.contains("<a:t>Line 2</a:t>"));
        assert!(text_box_xml.contains("<a:t>Line 3</a:t>"));
        assert!(!text_box_xml.contains('\r'));
        assert!(effects.warnings.contains(&serde_json::json!({
            "line_break_normalization": "crlf_cr_to_lf"
        })));

        let invalid = AddTextBox {
            text: "bad\u{0001}text".to_owned(),
            ..text_box
        };
        let error = invalid
            .apply(&mut package, &slide)
            .expect_err("illegal XML text is rejected before writing");

        assert_eq!(error.code(), ErrorCode::InvalidInput);
        assert!(error.message().contains("add_text_box text"));
        Ok(())
    }

    #[test]
    fn add_image_honors_z_order_front_back_and_index() -> Result<()> {
        assert_image_insert_order(None, &["Back", "Front", "Inserted"], &[3])?;
        assert_image_insert_order(
            Some(InsertOptions {
                z_order: Some(ZOrder::Keyword(ZOrderKeyword::Front)),
            }),
            &["Back", "Front", "Inserted"],
            &[3],
        )?;
        assert_image_insert_order(
            Some(InsertOptions {
                z_order: Some(ZOrder::Keyword(ZOrderKeyword::Back)),
            }),
            &["Inserted", "Back", "Front"],
            &[1],
        )?;
        assert_image_insert_order(
            Some(InsertOptions {
                z_order: Some(ZOrder::Index(2)),
            }),
            &["Back", "Inserted", "Front"],
            &[2],
        )?;
        Ok(())
    }

    #[test]
    fn replace_text_matches_golden() -> Result<()> {
        let slide_part = slide_part()?;
        let mut package = package_with_slide(TARGET_SLIDE_XML)?;
        let target = target(ElementKind::TextBox);
        let operation = ReplaceText {
            operation_id: "op-replace-text".to_owned(),
            element_id: target.element_id.clone(),
            text: "Updated\nCopy".to_owned(),
            current_text_match: Some("Old copy".to_owned()),
            mode: ReplaceTextMode::WholeElement,
            format_policy: FormatPolicy::PreserveFirstRun,
            allow_formatting_simplification: false,
            run: None,
            run_style: None,
            fit_policy: None,
        };

        operation.apply(&mut package, &target)?;

        assert_eq!(
            element_xml_at_path(&package, &slide_part, &[1])?,
            golden(REPLACE_TEXT_EXPECTED)
        );
        Ok(())
    }

    #[test]
    fn replace_text_whole_element_normalizes_carriage_returns_to_paragraphs() -> Result<()> {
        let slide_part = slide_part()?;
        let mut package = package_with_slide(TARGET_SLIDE_XML)?;
        let target = target(ElementKind::TextBox);
        let operation = ReplaceText {
            operation_id: "op-replace-text".to_owned(),
            element_id: target.element_id.clone(),
            text: "Line 1\r\nLine 2\rLine 3".to_owned(),
            current_text_match: Some("Old copy".to_owned()),
            mode: ReplaceTextMode::WholeElement,
            format_policy: FormatPolicy::PreserveFirstRun,
            allow_formatting_simplification: false,
            run: None,
            run_style: None,
            fit_policy: None,
        };

        let effects = operation.apply(&mut package, &target)?;
        let output = element_xml_at_path(&package, &slide_part, &[1])?;

        assert!(output.contains("<a:t>Line 1</a:t>"));
        assert!(output.contains("<a:t>Line 2</a:t>"));
        assert!(output.contains("<a:t>Line 3</a:t>"));
        assert!(!output.contains('\r'));
        assert!(effects.warnings.contains(&serde_json::json!({
            "newline_mapping": "paragraph",
            "line_break_normalization": "crlf_cr_to_lf"
        })));
        Ok(())
    }

    #[test]
    fn replace_text_warns_and_collapses_multi_run_formatting_to_first_run() -> Result<()> {
        let slide_part = slide_part()?;
        let mut package = package_with_slide(MULTI_RUN_SLIDE_XML)?;
        let target = target(ElementKind::TextBox);
        let operation = ReplaceText {
            operation_id: "op-replace-text".to_owned(),
            element_id: target.element_id.clone(),
            text: "Updated".to_owned(),
            current_text_match: Some("FirstSecond".to_owned()),
            mode: ReplaceTextMode::WholeElement,
            format_policy: FormatPolicy::PreserveExistingRuns,
            allow_formatting_simplification: true,
            run: None,
            run_style: None,
            fit_policy: None,
        };

        let effects = operation.apply(&mut package, &target)?;
        let output = element_xml_at_path(&package, &slide_part, &[1])?;

        assert!(has_warning_code(&effects.warnings, "formatting_simplified"));
        assert!(output.contains(r#"<a:rPr lang="en-US" sz="1800" b="1"/>"#));
        assert!(!output.contains(r#"sz="2400""#));
        assert!(!output.contains(r#"i="1""#));
        assert_eq!(output.matches("<a:r>").count(), 1);
        Ok(())
    }

    #[test]
    fn replace_text_warns_and_drops_rich_text_constructs() -> Result<()> {
        let slide_part = slide_part()?;
        let mut package = package_with_slide(RICH_TEXT_SLIDE_XML)?;
        let target = target(ElementKind::TextBox);
        let operation = ReplaceText {
            operation_id: "op-replace-text".to_owned(),
            element_id: target.element_id.clone(),
            text: "Updated".to_owned(),
            current_text_match: None,
            mode: ReplaceTextMode::WholeElement,
            format_policy: FormatPolicy::PreserveExistingRuns,
            allow_formatting_simplification: true,
            run: None,
            run_style: None,
            fit_policy: None,
        };

        let effects = operation.apply(&mut package, &target)?;
        let output = element_xml_at_path(&package, &slide_part, &[1])?;

        assert!(has_warning_code(&effects.warnings, "formatting_simplified"));
        assert!(!output.contains("hlinkClick"));
        assert!(!output.contains("<a:fld"));
        assert!(!output.contains("<a:br"));
        assert_eq!(output.matches("<a:r>").count(), 1);
        Ok(())
    }

    #[test]
    fn replace_text_whole_element_refuses_numbered_paragraph_without_confirmation() -> Result<()> {
        let package = package_with_slide(NUMBERED_PARAGRAPH_SLIDE_XML)?;
        let target = target(ElementKind::TextBox);
        let operation = ReplaceText {
            operation_id: "op-replace-text".to_owned(),
            element_id: target.element_id.clone(),
            text: "Updated".to_owned(),
            current_text_match: Some("Numbered item".to_owned()),
            mode: ReplaceTextMode::WholeElement,
            format_policy: FormatPolicy::PreserveExistingRuns,
            allow_formatting_simplification: false,
            run: None,
            run_style: None,
            fit_policy: None,
        };

        let error = operation
            .validate(&package, &target)
            .expect_err("numbered paragraph rewrite requires confirmation");

        assert_eq!(error.code(), ErrorCode::UnsupportedEdit);
        Ok(())
    }

    #[test]
    fn replace_text_warns_when_confirmed_rewrite_drops_literal_line_breaks() -> Result<()> {
        let slide_part = slide_part()?;
        let mut package = package_with_slide(LITERAL_LINE_BREAK_SLIDE_XML)?;
        let target = target(ElementKind::TextBox);
        let operation = ReplaceText {
            operation_id: "op-replace-text".to_owned(),
            element_id: target.element_id.clone(),
            text: "Updated".to_owned(),
            current_text_match: Some("First\nSecond".to_owned()),
            mode: ReplaceTextMode::WholeElement,
            format_policy: FormatPolicy::PreserveExistingRuns,
            allow_formatting_simplification: true,
            run: None,
            run_style: None,
            fit_policy: None,
        };

        let effects = operation.apply(&mut package, &target)?;
        let output = element_xml_at_path(&package, &slide_part, &[1])?;

        assert!(has_warning_code(&effects.warnings, "formatting_simplified"));
        assert!(!output.contains("First\nSecond"));
        assert!(output.contains(r#"<a:t>Updated</a:t>"#));
        Ok(())
    }

    #[test]
    fn replace_text_run_scoped_preserves_sibling_runs_and_rich_constructs() -> Result<()> {
        let slide_part = slide_part()?;
        let mut package = package_with_slide(RICH_TEXT_SLIDE_XML)?;
        let target = target(ElementKind::TextBox);
        let operation = ReplaceText {
            operation_id: "op-replace-text".to_owned(),
            element_id: target.element_id.clone(),
            text: "Updated".to_owned(),
            current_text_match: Some("Linked".to_owned()),
            mode: ReplaceTextMode::RunScoped,
            format_policy: FormatPolicy::PreserveExistingRuns,
            allow_formatting_simplification: false,
            run: Some(RunSelector {
                paragraph_index: 0,
                run_index: 0,
                run_end_index: None,
                text_hash: None,
            }),
            run_style: None,
            fit_policy: None,
        };

        let effects = operation.apply(&mut package, &target)?;
        let output = element_xml_at_path(&package, &slide_part, &[1])?;

        assert!(!has_warning_code(
            &effects.warnings,
            "formatting_simplified"
        ));
        assert!(output.contains(r#"<a:t>Updated</a:t>"#));
        assert!(!output.contains(r#"<a:t>Linked</a:t>"#));
        assert!(output.contains(r#"<a:hlinkClick r:id="rId2"/>"#));
        assert!(output.contains(r#"<a:fld id="{00000000-0000-0000-0000-000000000000}" type="slidenum"><a:rPr lang="en-US"/><a:t>Field</a:t></a:fld>"#));
        assert!(output.contains(r#"<a:br/>"#));
        assert!(output.contains(r#"<a:r><a:t>Break</a:t></a:r>"#));
        assert_eq!(output.matches("<a:r>").count(), 2);
        Ok(())
    }

    #[test]
    fn replace_text_run_scoped_maps_vertical_tab_to_soft_break() -> Result<()> {
        let slide_part = slide_part()?;
        let mut package = package_with_slide(MULTI_RUN_SLIDE_XML)?;
        let target = target(ElementKind::TextBox);
        let operation = ReplaceText {
            operation_id: "op-replace-text".to_owned(),
            element_id: target.element_id.clone(),
            text: "Line 1\u{000B}Line 2".to_owned(),
            current_text_match: Some("First".to_owned()),
            mode: ReplaceTextMode::RunScoped,
            format_policy: FormatPolicy::PreserveExistingRuns,
            allow_formatting_simplification: false,
            run: Some(RunSelector {
                paragraph_index: 0,
                run_index: 0,
                run_end_index: None,
                text_hash: None,
            }),
            run_style: None,
            fit_policy: None,
        };

        operation.apply(&mut package, &target)?;
        let output = element_xml_at_path(&package, &slide_part, &[1])?;

        assert!(output.contains(r#"<a:t>Line 1</a:t></a:r><a:br/><a:r>"#));
        assert!(output.contains(r#"<a:t>Line 2</a:t></a:r>"#));
        assert!(!output.contains('\u{000B}'));
        assert!(output.contains(r#"<a:t>Second</a:t>"#));
        Ok(())
    }

    #[test]
    fn replace_text_run_scoped_replaces_range_spanning_soft_break() -> Result<()> {
        let slide_part = slide_part()?;
        let mut package = package_with_slide(RICH_TEXT_SLIDE_XML)?;
        let target = target(ElementKind::TextBox);
        let operation = ReplaceText {
            operation_id: "op-replace-text".to_owned(),
            element_id: target.element_id.clone(),
            text: "Localized debt\u{000B}Localized association".to_owned(),
            current_text_match: Some("Linked\nBreak".to_owned()),
            mode: ReplaceTextMode::RunScoped,
            format_policy: FormatPolicy::PreserveExistingRuns,
            allow_formatting_simplification: false,
            run: Some(RunSelector {
                paragraph_index: 0,
                run_index: 0,
                run_end_index: Some(1),
                text_hash: None,
            }),
            run_style: None,
            fit_policy: None,
        };

        operation.apply(&mut package, &target)?;
        let output = element_xml_at_path(&package, &slide_part, &[1])?;

        assert!(output.contains(r#"<a:t>Localized debt</a:t></a:r><a:br/><a:r>"#));
        assert!(output.contains(r#"<a:t>Localized association</a:t></a:r>"#));
        assert!(!output.contains(r#"<a:t>Linked</a:t>"#));
        assert!(!output.contains(r#"<a:t>Break</a:t>"#));
        assert_eq!(output.matches("<a:br/>").count(), 1);
        assert!(output.contains(r#"<a:fld id="{00000000-0000-0000-0000-000000000000}" type="slidenum"><a:rPr lang="en-US"/><a:t>Field</a:t></a:fld>"#));
        Ok(())
    }

    #[test]
    fn replace_text_run_scoped_rejects_carriage_return() -> Result<()> {
        let mut package = package_with_slide(MULTI_RUN_SLIDE_XML)?;
        let target = target(ElementKind::TextBox);
        let operation = ReplaceText {
            operation_id: "op-replace-text".to_owned(),
            element_id: target.element_id.clone(),
            text: "Line 1\rLine 2".to_owned(),
            current_text_match: Some("First".to_owned()),
            mode: ReplaceTextMode::RunScoped,
            format_policy: FormatPolicy::PreserveExistingRuns,
            allow_formatting_simplification: false,
            run: Some(RunSelector {
                paragraph_index: 0,
                run_index: 0,
                run_end_index: None,
                text_hash: None,
            }),
            run_style: None,
            fit_policy: None,
        };

        let error = operation
            .apply(&mut package, &target)
            .expect_err("carriage returns are rejected for run-scoped text");

        assert_eq!(error.code(), ErrorCode::InvalidInput);
        assert!(error.message().contains("line-break characters"));
        Ok(())
    }

    #[test]
    fn replace_text_run_scoped_applies_run_style_without_touching_sibling_runs() -> Result<()> {
        let slide_part = slide_part()?;
        let mut package = package_with_slide(MULTI_RUN_SLIDE_XML)?;
        let target = target(ElementKind::TextBox);
        let operation = ReplaceText {
            operation_id: "op-replace-text".to_owned(),
            element_id: target.element_id.clone(),
            text: "Updated".to_owned(),
            current_text_match: Some("First".to_owned()),
            mode: ReplaceTextMode::RunScoped,
            format_policy: FormatPolicy::PreserveExistingRuns,
            allow_formatting_simplification: false,
            run: Some(RunSelector {
                paragraph_index: 0,
                run_index: 0,
                run_end_index: None,
                text_hash: None,
            }),
            run_style: Some(TextRunStyle {
                font_size_pt: Some(20),
                bold: Some(false),
                italic: Some(true),
                font_family: Some("Aptos".to_owned()),
                color_hex: Some("112233".to_owned()),
                align: Some(TextAlign::Center),
            }),
            fit_policy: None,
        };

        operation.apply(&mut package, &target)?;
        let output = element_xml_at_path(&package, &slide_part, &[1])?;

        assert!(output.contains(r#"<a:pPr algn="ctr"/>"#));
        assert!(output.contains(r#"<a:r><a:rPr lang="en-US" sz="2000" b="0" i="1"><a:solidFill><a:srgbClr val="112233"/></a:solidFill><a:latin typeface="Aptos"/></a:rPr><a:t>Updated</a:t></a:r>"#));
        assert!(
            output.contains(r#"<a:r><a:rPr lang="en-US" sz="2400" i="1"/><a:t>Second</a:t></a:r>"#)
        );
        Ok(())
    }

    #[test]
    fn replace_text_run_scoped_inserts_fill_before_existing_latin_font() -> Result<()> {
        let slide_part = slide_part()?;
        let mut package = package_with_slide(RUN_STYLE_ORDER_SLIDE_XML)?;
        let target = target(ElementKind::TextBox);
        let operation = ReplaceText {
            operation_id: "op-replace-text".to_owned(),
            element_id: target.element_id.clone(),
            text: "Updated".to_owned(),
            current_text_match: Some("First".to_owned()),
            mode: ReplaceTextMode::RunScoped,
            format_policy: FormatPolicy::PreserveExistingRuns,
            allow_formatting_simplification: false,
            run: Some(RunSelector {
                paragraph_index: 0,
                run_index: 0,
                run_end_index: None,
                text_hash: None,
            }),
            run_style: Some(TextRunStyle {
                font_size_pt: None,
                bold: None,
                italic: None,
                font_family: Some("Aptos".to_owned()),
                color_hex: Some("112233".to_owned()),
                align: None,
            }),
            fit_policy: None,
        };

        operation.apply(&mut package, &target)?;
        let output = element_xml_at_path(&package, &slide_part, &[1])?;

        assert!(output.contains(r#"<a:rPr lang="en-US"><a:solidFill><a:srgbClr val="112233"/></a:solidFill><a:latin typeface="Aptos"/><a:hlinkClick r:id="rId2"/></a:rPr>"#));
        Ok(())
    }

    #[test]
    fn replace_text_run_scoped_rejects_invalid_font_size_points() -> Result<()> {
        let package = package_with_slide(MULTI_RUN_SLIDE_XML)?;
        let target = target(ElementKind::TextBox);
        let operation = ReplaceText {
            operation_id: "op-replace-text".to_owned(),
            element_id: target.element_id.clone(),
            text: "Updated".to_owned(),
            current_text_match: Some("First".to_owned()),
            mode: ReplaceTextMode::RunScoped,
            format_policy: FormatPolicy::PreserveExistingRuns,
            allow_formatting_simplification: false,
            run: Some(RunSelector {
                paragraph_index: 0,
                run_index: 0,
                run_end_index: None,
                text_hash: None,
            }),
            run_style: Some(TextRunStyle {
                font_size_pt: Some(4_000_000_000),
                bold: None,
                italic: None,
                font_family: None,
                color_hex: None,
                align: None,
            }),
            fit_policy: None,
        };

        let error = operation
            .validate(&package, &target)
            .expect_err("font size outside DrawingML ST_TextFontSize is invalid");

        assert_eq!(error.code(), ErrorCode::InvalidInput);
        Ok(())
    }

    #[test]
    fn replace_notes_text_rewrites_only_notes_body_run() -> Result<()> {
        let slide_part = slide_part()?;
        let notes_part = notes_part()?;
        let original_slide = NOTES_LINKED_SLIDE_XML.as_bytes().to_vec();
        let mut package = package_with_notes()?;
        let target = notes_target()?;
        let operation = ReplaceNotesText {
            operation_id: "op-replace-notes-text".to_owned(),
            slide_id: target.slide_id.clone(),
            text: "Updated notes".to_owned(),
            current_text_match: Some("Linked".to_owned()),
            run: Some(RunSelector {
                paragraph_index: 0,
                run_index: 0,
                run_end_index: None,
                text_hash: None,
            }),
        };

        let effects = operation.apply(&mut package, &target)?;
        let output = part_xml(&package, &notes_part)?;

        assert_eq!(
            effects.changed_parts,
            vec!["ppt/notesSlides/notesSlide1.xml"]
        );
        assert!(package.dirty_parts().contains(&notes_part));
        assert!(!package.dirty_parts().contains(&slide_part));
        assert_eq!(
            package
                .parts()
                .get(&slide_part)
                .expect("slide part exists")
                .bytes(),
            original_slide.as_slice()
        );
        assert!(output.contains(r#"<a:t>Updated notes</a:t>"#));
        assert!(!output.contains(r#"<a:t>Linked</a:t>"#));
        assert!(output.contains(r#"<a:hlinkClick r:id="rId9"/>"#));
        assert!(output.contains(r#"<a:fld id="{00000000-0000-0000-0000-000000000000}" type="datetime"><a:t>Field</a:t></a:fld>"#));
        assert!(output.contains(r#"<a:br/>"#));
        assert!(output.contains(r#"<a:r><a:t>Sibling</a:t></a:r>"#));
        Ok(())
    }

    #[test]
    fn replace_notes_text_rejects_missing_notes_body_part() -> Result<()> {
        let package = package_with_slide(NOTES_LINKED_SLIDE_XML)?;
        let target = ResolvedNotesSlide {
            slide_id: "slide-1".to_owned(),
            slide_part: slide_part()?,
            notes_part: notes_part()?,
            element_id: "slide-1:notes".to_owned(),
        };
        let operation = ReplaceNotesText {
            operation_id: "op-replace-notes-text".to_owned(),
            slide_id: target.slide_id.clone(),
            text: "Updated notes".to_owned(),
            current_text_match: None,
            run: Some(RunSelector {
                paragraph_index: 0,
                run_index: 0,
                run_end_index: None,
                text_hash: None,
            }),
        };

        let error = operation
            .validate(&package, &target)
            .expect_err("missing notes part is unsupported");

        assert_eq!(
            error.code(),
            pptx_compose_core::error::ErrorCode::UnsupportedEdit
        );
        Ok(())
    }

    #[test]
    fn replace_text_whole_element_rejects_run_style() -> Result<()> {
        let package = package_with_slide(MULTI_RUN_SLIDE_XML)?;
        let target = target(ElementKind::TextBox);
        let operation = ReplaceText {
            operation_id: "op-replace-text".to_owned(),
            element_id: target.element_id.clone(),
            text: "Updated".to_owned(),
            current_text_match: None,
            mode: ReplaceTextMode::WholeElement,
            format_policy: FormatPolicy::PreserveExistingRuns,
            allow_formatting_simplification: true,
            run: None,
            run_style: Some(TextRunStyle {
                font_size_pt: Some(20),
                bold: None,
                italic: None,
                font_family: None,
                color_hex: None,
                align: None,
            }),
            fit_policy: None,
        };

        let error = operation
            .validate(&package, &target)
            .expect_err("whole-element mode rejects run_style");

        assert_eq!(
            error.code(),
            pptx_compose_core::error::ErrorCode::UnsupportedEdit
        );
        Ok(())
    }

    #[test]
    fn replace_text_whole_element_refuses_rich_text_without_confirmation() -> Result<()> {
        let package = package_with_slide(RICH_TEXT_SLIDE_XML)?;
        let target = target(ElementKind::TextBox);
        let operation = ReplaceText {
            operation_id: "op-replace-text".to_owned(),
            element_id: target.element_id.clone(),
            text: "Updated".to_owned(),
            current_text_match: None,
            mode: ReplaceTextMode::WholeElement,
            format_policy: FormatPolicy::PreserveExistingRuns,
            allow_formatting_simplification: false,
            run: None,
            run_style: None,
            fit_policy: None,
        };

        let error = operation
            .validate(&package, &target)
            .expect_err("rich whole-element rewrite requires confirmation");

        assert_eq!(
            error.code(),
            pptx_compose_core::error::ErrorCode::UnsupportedEdit
        );
        Ok(())
    }

    #[test]
    fn replace_text_run_scoped_match_guard_fails_on_selected_run() -> Result<()> {
        let package = package_with_slide(MULTI_RUN_SLIDE_XML)?;
        let target = target(ElementKind::TextBox);
        let operation = ReplaceText {
            operation_id: "op-replace-text".to_owned(),
            element_id: target.element_id.clone(),
            text: "Updated".to_owned(),
            current_text_match: Some("FirstSecond".to_owned()),
            mode: ReplaceTextMode::RunScoped,
            format_policy: FormatPolicy::PreserveExistingRuns,
            allow_formatting_simplification: false,
            run: Some(RunSelector {
                paragraph_index: 0,
                run_index: 1,
                run_end_index: None,
                text_hash: None,
            }),
            run_style: None,
            fit_policy: None,
        };

        let error = operation
            .validate(&package, &target)
            .expect_err("run-level match guard uses selected run text");

        assert_eq!(
            error.code(),
            pptx_compose_core::error::ErrorCode::SelectorGuardFailed
        );
        Ok(())
    }

    #[test]
    fn replace_text_does_not_warn_for_single_plain_run() -> Result<()> {
        let mut package = package_with_slide(SINGLE_PLAIN_RUN_SLIDE_XML)?;
        let target = target(ElementKind::TextBox);
        let operation = ReplaceText {
            operation_id: "op-replace-text".to_owned(),
            element_id: target.element_id.clone(),
            text: "Updated".to_owned(),
            current_text_match: Some("Old copy".to_owned()),
            mode: ReplaceTextMode::WholeElement,
            format_policy: FormatPolicy::PreserveExistingRuns,
            allow_formatting_simplification: false,
            run: None,
            run_style: None,
            fit_policy: None,
        };

        let effects = operation.apply(&mut package, &target)?;

        assert!(!has_warning_code(
            &effects.warnings,
            "formatting_simplified"
        ));
        Ok(())
    }

    #[test]
    fn replace_image_retargets_relationship_matches_golden() -> Result<()> {
        let slide_part = slide_part()?;
        let rels_part = PartName::from_zip_entry("ppt/slides/_rels/slide1.xml.rels")?;
        let mut package = package_with_picture_rels()?;
        let target = target(ElementKind::Picture);
        let operation = ReplaceImage {
            operation_id: "op-replace-image".to_owned(),
            element_id: target.element_id.clone(),
            media_ref: "media-1".to_owned(),
            content_type: "image/png".to_owned(),
        };

        operation.apply(&mut package, &target, &media_inputs())?;

        assert_eq!(
            part_xml(&package, &rels_part)?,
            golden(REPLACE_IMAGE_RELS_EXPECTED)
        );
        assert!(
            package
                .parts()
                .get(&PartName::from_zip_entry("ppt/media/image1.png")?)
                .is_none()
        );
        assert!(
            package
                .parts()
                .get(&PartName::from_zip_entry("ppt/media/image2.png")?)
                .is_some()
        );
        assert!(package.dirty_parts().contains(&rels_part));
        assert!(!package.dirty_parts().contains(&slide_part));
        Ok(())
    }

    #[test]
    fn move_resize_matches_golden() -> Result<()> {
        let slide_part = slide_part()?;
        let mut package = package_with_slide(TARGET_SLIDE_XML)?;
        let target = target(ElementKind::TextBox);
        let operation = MoveResize {
            element_id: target.element_id.clone(),
            bounds: Bounds {
                x: 10,
                y: 20,
                cx: 300,
                cy: 400,
            },
        };

        operation.apply(&mut package, &target)?;

        assert_eq!(
            element_xml_at_path(&package, &slide_part, &[1])?,
            golden(MOVE_RESIZE_EXPECTED)
        );
        Ok(())
    }

    #[test]
    fn set_alt_text_matches_golden() -> Result<()> {
        let slide_part = slide_part()?;
        let mut package = package_with_slide(PICTURE_SLIDE_XML)?;
        let target = target(ElementKind::Picture);
        let operation = SetAltText {
            operation_id: "op-set-alt-text".to_owned(),
            element_id: target.element_id.clone(),
            title: Some("Accessible title".to_owned()),
            description: Some("Readable description".to_owned()),
        };

        operation.apply(&mut package, &target)?;

        assert_eq!(
            element_xml_at_path(&package, &slide_part, &[1])?,
            golden(SET_ALT_TEXT_EXPECTED)
        );
        Ok(())
    }

    const TARGET_SLIDE_XML: &str = r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="9" name="Body" descr="Original alt"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm rot="5400000" flipH="1"><a:off x="1" y="2"/><a:ext cx="3" cy="4"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr><p:txBody><a:bodyPr wrap="square"/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US" dirty="0" sz="1800" b="1"><a:solidFill><a:srgbClr val="112233"/></a:solidFill><a:latin typeface="Aptos"/></a:rPr><a:t>Old copy</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#;

    const MULTI_RUN_SLIDE_XML: &str = r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="9" name="Body"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US" sz="1800" b="1"/><a:t>First</a:t></a:r><a:r><a:rPr lang="en-US" sz="2400" i="1"/><a:t>Second</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#;

    const RUN_STYLE_ORDER_SLIDE_XML: &str = r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="9" name="Body"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US"><a:latin typeface="Calibri"/><a:hlinkClick r:id="rId2"/></a:rPr><a:t>First</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#;

    const RICH_TEXT_SLIDE_XML: &str = r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="9" name="Body"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:fld id="{00000000-0000-0000-0000-000000000000}" type="slidenum"><a:rPr lang="en-US"/><a:t>Field</a:t></a:fld><a:r><a:rPr lang="en-US"><a:hlinkClick r:id="rId2"/></a:rPr><a:t>Linked</a:t></a:r><a:br/><a:r><a:t>Break</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#;

    const NUMBERED_PARAGRAPH_SLIDE_XML: &str = r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="9" name="Body"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:buAutoNum type="romanUcPeriod"/></a:pPr><a:r><a:t>Numbered item</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#;

    const LITERAL_LINE_BREAK_SLIDE_XML: &str = r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="9" name="Body"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>First
Second</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#;

    const NOTES_LINKED_SLIDE_XML: &str = r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/></p:spTree></p:cSld></p:sld>"#;

    const NOTES_SLIDE_XML: &str = r#"<p:notes xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="2" name="Slide Image Placeholder 1"/><p:cNvSpPr/><p:nvPr><p:ph type="sldImg"/></p:nvPr></p:nvSpPr><p:spPr/></p:sp><p:sp><p:nvSpPr><p:cNvPr id="3" name="Notes Placeholder 2"/><p:cNvSpPr txBox="1"/><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:fld id="{00000000-0000-0000-0000-000000000000}" type="datetime"><a:t>Field</a:t></a:fld><a:r><a:rPr lang="en-US"><a:hlinkClick r:id="rId9"/></a:rPr><a:t>Linked</a:t></a:r><a:br/><a:r><a:t>Sibling</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:notes>"#;

    const SINGLE_PLAIN_RUN_SLIDE_XML: &str = r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="9" name="Body"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Old copy</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#;

    const PICTURE_SLIDE_XML: &str = r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:pic><p:nvPicPr><p:cNvPr id="9" name="Picture" hidden="0"/><p:cNvPicPr><a:picLocks noChangeAspect="1"/></p:cNvPicPr><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rId5"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x="1" y="2"/><a:ext cx="3" cy="4"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic></p:spTree></p:cSld></p:sld>"#;

    const PICTURE_RELS_XML: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/></Relationships>"#;

    const ORDER_SLIDE_XML: &str = r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="9" name="Back"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Back</a:t></a:r></a:p></p:txBody></p:sp><p:sp><p:nvSpPr><p:cNvPr id="10" name="Front"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Front</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#;

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

    fn slide_part() -> Result<PartName> {
        PartName::from_zip_entry("ppt/slides/slide1.xml")
    }

    fn notes_part() -> Result<PartName> {
        PartName::from_zip_entry("ppt/notesSlides/notesSlide1.xml")
    }

    fn package_with_slide(slide_xml: &str) -> Result<Package> {
        let mut package = Package::new();
        package.insert_zip_entry("ppt/slides/slide1.xml", slide_xml.as_bytes().to_vec())?;
        Ok(package)
    }

    fn package_with_notes() -> Result<Package> {
        let slide_part = slide_part()?;
        let mut package = package_with_slide(NOTES_LINKED_SLIDE_XML)?;
        package.insert_zip_entry(
            notes_part()?.zip_entry_name(),
            NOTES_SLIDE_XML.as_bytes().to_vec(),
        )?;
        package.push_relationship(Relationship::internal(
            RelationshipSource::Part(slide_part),
            "rIdNotes",
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide",
            "../notesSlides/notesSlide1.xml",
        ));
        Ok(package)
    }

    fn package_with_picture_rels() -> Result<Package> {
        let slide_part = slide_part()?;
        let old_media_part = PartName::from_zip_entry("ppt/media/image1.png")?;
        let mut package = package_with_slide(PICTURE_SLIDE_XML)?;
        package.insert_zip_entry(
            "ppt/slides/_rels/slide1.xml.rels",
            PICTURE_RELS_XML.as_bytes().to_vec(),
        )?;
        package.insert_zip_entry(old_media_part.zip_entry_name(), b"old image".to_vec())?;
        package
            .content_types_mut()
            .insert_default("png", "image/png");
        package.push_relationship(Relationship::internal(
            RelationshipSource::Part(slide_part),
            "rId5",
            IMAGE_REL_TYPE,
            "../media/image1.png",
        ));
        Ok(package)
    }

    fn assert_text_box_insert_order(
        insert: Option<InsertOptions>,
        expected_names: &[&str],
        expected_path: &[u32],
    ) -> Result<()> {
        let slide_part = slide_part()?;
        let mut package = package_with_slide(ORDER_SLIDE_XML)?;
        let slide = minimal_slide()?;
        let operation = AddTextBox {
            operation_id: "op-order-text".to_owned(),
            slide_id: slide.slide_id.clone(),
            text: "Inserted".to_owned(),
            bounds: fixed_bounds(),
            name: Some("Inserted".to_owned()),
            alt_text: None,
            style: None,
            insert,
        };

        let effects = operation.apply(&mut package, &slide)?;

        assert_eq!(shape_tree_names(&package, &slide_part)?, expected_names);
        assert_eq!(effects.created_element_ids, vec!["slide-1:shape-11"]);
        assert_inserted_path(&package, &slide_part, expected_path)?;
        Ok(())
    }

    fn assert_image_insert_order(
        insert: Option<InsertOptions>,
        expected_names: &[&str],
        expected_path: &[u32],
    ) -> Result<()> {
        let slide_part = slide_part()?;
        let mut package = package_with_slide(ORDER_SLIDE_XML)?;
        let slide = minimal_slide()?;
        let operation = AddImage {
            operation_id: "op-order-image".to_owned(),
            slide_id: slide.slide_id.clone(),
            media_ref: "media-1".to_owned(),
            content_type: "image/png".to_owned(),
            bounds: fixed_bounds(),
            name: Some("Inserted".to_owned()),
            alt_text: None,
            fit: ImageFit::Stretch,
            dedupe: ImageDedupe::Never,
            insert,
        };

        let effects = operation.apply(&mut package, &slide, &media_inputs())?;

        assert_eq!(shape_tree_names(&package, &slide_part)?, expected_names);
        assert_eq!(effects.created_element_ids, vec!["slide-1:pic-11"]);
        assert_inserted_path(&package, &slide_part, expected_path)?;
        Ok(())
    }

    fn assert_inserted_path(
        package: &Package,
        slide_part: &PartName,
        expected_path: &[u32],
    ) -> Result<()> {
        let xml = element_xml_at_path(package, slide_part, expected_path)?;
        assert!(xml.contains(r#"name="Inserted""#));
        Ok(())
    }

    fn shape_tree_names(package: &Package, slide_part: &PartName) -> Result<Vec<String>> {
        let part = package.parts().get(slide_part).ok_or_else(|| {
            Error::unsupported_package(format!("Slide part {slide_part} was not found."))
        })?;
        let document = parse_document(part.bytes())?;
        let root = document
            .root_element()
            .ok_or_else(|| Error::malformed_xml("Slide XML does not contain a root element."))?;
        let sp_tree = first_descendant(root, "spTree").ok_or_else(|| {
            Error::unsupported_package("Slide fixture does not contain p:spTree.")
        })?;
        Ok(sp_tree
            .children
            .iter()
            .filter_map(XmlNode::as_element)
            .filter(|element| matches!(element.name.local_name.as_str(), "sp" | "pic"))
            .filter_map(shape_name)
            .map(ToOwned::to_owned)
            .collect())
    }

    fn shape_name(element: &XmlElement) -> Option<&str> {
        first_descendant(element, "cNvPr").and_then(|cnv_pr| {
            cnv_pr
                .attributes
                .iter()
                .find(|attribute| attribute.name.local_name == "name")
                .map(|attribute| attribute.value.as_str())
        })
    }

    fn target(kind: ElementKind) -> ResolvedElement {
        ResolvedElement {
            slide_id: "slide-1".to_owned(),
            element_id: "slide-1:target-9".to_owned(),
            kind,
            part: slide_part().expect("slide part is valid"),
            sp_tree_path: vec![1],
            group_path: Vec::new(),
            cnvpr_id: Some(9),
            text_hash: None,
            fingerprint: "fp".to_owned(),
        }
    }

    fn notes_target() -> Result<ResolvedNotesSlide> {
        Ok(ResolvedNotesSlide {
            slide_id: "slide-1".to_owned(),
            slide_part: slide_part()?,
            notes_part: notes_part()?,
            element_id: "slide-1:notes".to_owned(),
        })
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

    fn element_xml_at_path(
        package: &Package,
        slide_part: &PartName,
        path: &[u32],
    ) -> Result<String> {
        let part = package.parts().get(slide_part).ok_or_else(|| {
            Error::unsupported_package(format!("Slide part {slide_part} was not found."))
        })?;
        let document = parse_document(part.bytes())?;
        let root = document
            .root_element()
            .ok_or_else(|| Error::malformed_xml("Slide XML does not contain a root element."))?;
        let sp_tree = first_descendant(root, "spTree").ok_or_else(|| {
            Error::unsupported_package("Slide fixture does not contain p:spTree.")
        })?;
        let element = element_at_path(sp_tree, path).ok_or_else(|| {
            Error::unsupported_package("Target element path did not resolve in p:spTree.")
        })?;
        serialize_element(element)
    }

    fn part_xml(package: &Package, part_name: &PartName) -> Result<String> {
        let part = package.parts().get(part_name).ok_or_else(|| {
            Error::unsupported_package(format!("Package part {part_name} was not found."))
        })?;
        String::from_utf8(part.bytes().to_vec())
            .map_err(|source| Error::parse_error("Package part XML was not UTF-8.", source))
    }

    fn part_bytes<'a>(package: &'a Package, part_name: &PartName) -> Result<&'a [u8]> {
        package
            .parts()
            .get(part_name)
            .map(Part::bytes)
            .ok_or_else(|| {
                Error::unsupported_package(format!("Package part {part_name} was not found."))
            })
    }

    fn assert_contains_bounds(xml: &str, bounds: &Bounds) {
        assert!(xml.contains(&format!(r#"<a:off x="{}" y="{}"/>"#, bounds.x, bounds.y)));
        assert!(xml.contains(&format!(
            r#"<a:ext cx="{}" cy="{}"/>"#,
            bounds.cx, bounds.cy
        )));
    }

    fn assert_slide_namespaces(xml: &str) {
        assert!(
            xml.contains(r#"xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#),
            "constructed slide XML must declare the presentation namespace"
        );
        assert!(
            xml.contains(r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#),
            "constructed slide XML must declare the DrawingML namespace"
        );
        assert!(
            xml.contains(
                r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#
            ),
            "constructed slide XML must declare the relationships namespace"
        );
        assert_eq!(
            xml.matches("xmlns:p=").count(),
            1,
            "constructed slide XML must not duplicate the presentation namespace"
        );
        assert_eq!(
            xml.matches("xmlns:a=").count(),
            1,
            "constructed slide XML must not duplicate the DrawingML namespace"
        );
        assert_eq!(
            xml.matches("xmlns:r=").count(),
            1,
            "constructed slide XML must not duplicate the relationships namespace"
        );
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

    fn golden(expected: &str) -> &str {
        expected.strip_suffix('\n').unwrap_or(expected)
    }

    fn has_warning_code(warnings: &[serde_json::Value], code: &str) -> bool {
        warnings
            .iter()
            .any(|warning| warning.get("code").and_then(serde_json::Value::as_str) == Some(code))
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

    fn element_at_path<'a>(sp_tree: &'a XmlElement, path: &[u32]) -> Option<&'a XmlElement> {
        let mut current = sp_tree;
        for component in path {
            let index = usize::try_from(component.checked_sub(1)?).ok()?;
            current = current
                .children
                .iter()
                .filter_map(XmlNode::as_element)
                .filter(|element| is_drawable_shape_tree_child(element))
                .nth(index)?;
        }
        Some(current)
    }

    fn is_drawable_shape_tree_child(element: &XmlElement) -> bool {
        matches!(
            element.name.local_name.as_str(),
            "sp" | "pic" | "graphicFrame" | "grpSp" | "cxnSp"
        )
    }
}
