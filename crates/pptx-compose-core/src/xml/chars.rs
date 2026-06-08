use crate::error::{Error, Result};

pub fn validate_xml_chars(value: &str, context: &str) -> Result<()> {
    if let Some(character) = value.chars().find(|character| !is_xml_char(*character)) {
        return Err(Error::malformed_xml(format!(
            "{context} contains XML 1.0 illegal character U+{:04X}.",
            u32::from(character)
        )));
    }
    Ok(())
}

pub const fn is_xml_char(character: char) -> bool {
    matches!(
        character as u32,
        0x09 | 0x0A | 0x0D | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF
    )
}

#[cfg(test)]
mod tests {
    use crate::error::ErrorCode;

    use super::validate_xml_chars;

    #[test]
    fn accepts_xml_line_break_characters() {
        validate_xml_chars("tab\tlf\ncr\rtext", "XML text").expect("XML line breaks are valid");
    }

    #[test]
    fn rejects_vertical_tab() {
        let error = validate_xml_chars("bad\u{000B}text", "XML text")
            .expect_err("vertical tab is not legal XML 1.0 text");

        assert_eq!(error.code(), ErrorCode::MalformedXml);
        assert!(error.message().contains("U+000B"));
    }
}
