use unicode_normalization::UnicodeNormalization;

use super::cpj;

/// One item in an element's text projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextSegment<'a> {
    Run(&'a str),
    SoftBreak,
}

/// Normalizes run text with optional soft breaks after each run.
///
/// `has_br[index] == true` inserts an `a:br` projection after `runs[index]`.
/// Extra flags are ignored; missing flags mean no break after that run.
#[must_use]
pub fn normalize_text(runs: &[&str], has_br: &[bool]) -> String {
    let mut segments = Vec::with_capacity(runs.len().saturating_add(has_br.len()));
    for (index, run) in runs.iter().enumerate() {
        segments.push(TextSegment::Run(run));
        if has_br.get(index).copied().unwrap_or(false) {
            segments.push(TextSegment::SoftBreak);
        }
    }

    normalize_segments(&segments)
}

/// Normalizes the exact text projection exposed as the 042 `normalized` field.
#[must_use]
pub fn normalize_segments(segments: &[TextSegment<'_>]) -> String {
    let mut normalized = String::new();
    let mut text_buffer = String::new();
    let mut pending_space = false;

    for segment in segments {
        match segment {
            TextSegment::Run(text) => text_buffer.push_str(text),
            TextSegment::SoftBreak => {
                normalize_text_buffer(&text_buffer, &mut normalized, &mut pending_space);
                text_buffer.clear();
                trim_trailing_spaces(&mut normalized);
                normalized.push('\n');
                pending_space = false;
            }
        }
    }

    normalize_text_buffer(&text_buffer, &mut normalized, &mut pending_space);
    trim_trailing_spaces(&mut normalized);
    normalized
}

/// Hashes an already-normalized text projection.
#[must_use]
pub fn text_hash(normalized: &str) -> String {
    cpj::digest_prefixed(normalized.as_bytes())
}

fn normalize_text_buffer(buffer: &str, normalized: &mut String, pending_space: &mut bool) {
    for ch in buffer.nfc() {
        if is_xml_whitespace(ch) {
            *pending_space = true;
        } else {
            if *pending_space && !normalized.is_empty() && !normalized.ends_with('\n') {
                normalized.push(' ');
            }
            normalized.push(ch);
            *pending_space = false;
        }
    }
}

fn trim_trailing_spaces(value: &mut String) {
    while value.ends_with(' ') {
        value.pop();
    }
}

const fn is_xml_whitespace(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\r')
}

#[cfg(test)]
#[test]
fn normalization() {
    assert_eq!(
        normalize_text(&["  Hello\t", "\n world "], &[]),
        "Hello world"
    );
    assert_eq!(normalize_text(&["line1", "line2"], &[true]), "line1\nline2");

    let normalized = normalize_text(&["line1", "line2"], &[true]);
    assert_eq!(
        text_hash(&normalized),
        "sha256:683376e290829b482c2655745caffa7a1dccfa10afaa62dac2b42dd6c68d0f83"
    );

    let decomposed = normalize_text(&["e\u{301}"], &[]);
    let precomposed = normalize_text(&["\u{e9}"], &[]);
    assert_eq!(decomposed, precomposed);
    assert_eq!(text_hash(&decomposed), text_hash(&precomposed));

    let split_combining_mark =
        normalize_segments(&[TextSegment::Run("e"), TextSegment::Run("\u{301}")]);
    assert_eq!(split_combining_mark, "\u{e9}");
}
