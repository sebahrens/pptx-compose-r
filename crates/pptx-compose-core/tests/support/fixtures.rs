use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct FixtureManifest {
    pub entries: Vec<FixtureEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct FixtureEntry {
    pub path: String,
    pub source_app: SourceApp,
    pub features: Vec<String>,
    pub expected_warnings: Vec<String>,
    pub invariants: Vec<String>,
    pub consuming_test: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "kebab-case")]
pub enum SourceApp {
    #[serde(rename = "powerpoint")]
    PowerPoint,
    #[serde(rename = "libreoffice")]
    LibreOffice,
    GoogleSlides,
    Legacy,
    Synthetic,
}

pub fn load_manifest() -> FixtureManifest {
    let manifest_path = fixture_path("manifest.toml");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap_or_else(|error| {
        panic!(
            "could not read fixture manifest at {}: {error}",
            manifest_path.display()
        )
    });
    toml::from_str(&manifest).unwrap_or_else(|error| {
        panic!(
            "could not parse fixture manifest at {}: {error}",
            manifest_path.display()
        )
    })
}

pub fn fixture_path(rel: &str) -> PathBuf {
    fixtures_root().join(rel)
}

pub fn assert_expected_warnings<I, S>(
    fixture: &str,
    expected_warnings: &[String],
    actual_warning_codes: I,
) where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let expected = expected_warnings
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let actual = actual_warning_codes
        .into_iter()
        .map(|code| code.as_ref().to_owned())
        .collect::<BTreeSet<_>>();

    assert_eq!(
        actual,
        expected
            .iter()
            .map(|warning| (*warning).to_owned())
            .collect::<BTreeSet<_>>(),
        "{fixture}: validation warnings did not match fixture manifest"
    );
}

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures")
}
