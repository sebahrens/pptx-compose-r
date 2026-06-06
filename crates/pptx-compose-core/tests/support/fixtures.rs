use std::path::{Path, PathBuf};

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

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures")
}
