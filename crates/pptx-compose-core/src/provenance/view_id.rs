use std::collections::BTreeMap;

use crate::provenance::cpj::{self, Cpj};

const VIEW_ID_SCHEMA: &str = "pptx-compose.view_id.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViewMode {
    DeckSummary,
    SlideDetail,
    ElementDetail,
}

#[must_use]
pub fn view_id(document_id: &str, revision: u64, mode: ViewMode, scope: &Cpj) -> String {
    let mut preimage = BTreeMap::new();
    preimage.insert("document_id".to_owned(), Cpj::Str(document_id.to_owned()));
    preimage.insert("mode".to_owned(), Cpj::Str(mode.as_token().to_owned()));
    preimage.insert("revision".to_owned(), Cpj::Uint(revision));
    preimage.insert("schema".to_owned(), Cpj::Str(VIEW_ID_SCHEMA.to_owned()));
    preimage.insert("scope".to_owned(), scope.clone());

    cpj::digest_cpj(&Cpj::Object(preimage))
}

impl ViewMode {
    #[must_use]
    pub const fn as_token(self) -> &'static str {
        match self {
            Self::DeckSummary => "deck_summary",
            Self::SlideDetail => "slide_detail",
            Self::ElementDetail => "element_detail",
        }
    }
}

#[cfg(test)]
#[test]
fn scope_sensitivity() {
    let document_id = "sha256:2610efa71c965b45569609b83e454e27219cad81cee0dc39a6669bde50f07dc8";
    let scope = slide_scope("slide-1", "cursor-a", 20);

    let base = view_id(document_id, 1, ViewMode::SlideDetail, &scope);
    assert!(base.starts_with("sha256:"));
    assert_eq!(base.len(), "sha256:".len() + 64);
    assert_eq!(base, view_id(document_id, 1, ViewMode::SlideDetail, &scope));

    let changed_cursor = slide_scope("slide-1", "cursor-b", 20);
    assert_ne!(
        base,
        view_id(document_id, 1, ViewMode::SlideDetail, &changed_cursor)
    );

    assert_ne!(base, view_id(document_id, 2, ViewMode::SlideDetail, &scope));
}

#[cfg(test)]
fn slide_scope(slide_id: &str, cursor: &str, limit: u64) -> Cpj {
    let mut pagination = BTreeMap::new();
    pagination.insert("cursor".to_owned(), Cpj::Str(cursor.to_owned()));
    pagination.insert("limit".to_owned(), Cpj::Uint(limit));

    let mut scope = BTreeMap::new();
    scope.insert("pagination".to_owned(), Cpj::Object(pagination));
    scope.insert(
        "slide_ids".to_owned(),
        Cpj::Array(vec![Cpj::Str(slide_id.to_owned())]),
    );

    Cpj::Object(scope)
}
