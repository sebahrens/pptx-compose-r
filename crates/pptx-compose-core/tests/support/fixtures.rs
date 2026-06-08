use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct FixtureManifest {
    pub entries: Vec<FixtureEntry>,
}

impl FixtureManifest {
    pub fn validate(&self) {
        assert!(
            !self.entries.is_empty(),
            "fixture manifest must include at least one entry"
        );

        let mut paths = BTreeSet::new();
        for entry in &self.entries {
            entry.validate();
            assert!(
                paths.insert(entry.path.as_str()),
                "fixture manifest contains duplicate path: {}",
                entry.path
            );
        }
    }

    #[allow(dead_code)]
    pub fn has_source_app(&self, source_app: SourceApp) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.source_app == source_app)
    }

    #[allow(dead_code)]
    pub fn has_path_prefix(&self, prefix: &str) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.path.starts_with(prefix))
    }

    #[allow(dead_code)]
    pub fn has_feature(&self, feature: &str) -> bool {
        self.entries.iter().any(|entry| entry.has_feature(feature))
    }

    #[allow(dead_code)]
    pub fn entries_with_feature(&self, feature: &str) -> impl Iterator<Item = &FixtureEntry> {
        self.entries
            .iter()
            .filter(move |entry| entry.has_feature(feature))
    }
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

impl FixtureEntry {
    pub fn has_feature(&self, feature: &str) -> bool {
        self.features.iter().any(|item| item == feature)
    }

    pub fn has_invariant(&self, invariant: &str) -> bool {
        self.invariants.iter().any(|item| item == invariant)
    }

    fn validate(&self) {
        assert!(
            !self.path.trim().is_empty(),
            "fixture manifest entry has an empty path"
        );
        assert!(
            is_safe_relative_path(&self.path),
            "fixture manifest entry path must be a safe relative path: {}",
            self.path
        );
        assert!(
            !self.features.is_empty(),
            "fixture manifest entry has no features: {}",
            self.path
        );
        assert!(
            !self.invariants.is_empty(),
            "fixture manifest entry has no invariants: {}",
            self.path
        );
        assert!(
            !self.consuming_test.trim().is_empty(),
            "fixture manifest entry has an empty consuming_test: {}",
            self.path
        );

        assert_non_empty_unique("features", &self.path, &self.features);
        assert_non_empty_unique("expected_warnings", &self.path, &self.expected_warnings);
        assert_non_empty_unique("invariants", &self.path, &self.invariants);
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
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
    let manifest: FixtureManifest = toml::from_str(&manifest).unwrap_or_else(|error| {
        panic!(
            "could not parse fixture manifest at {}: {error}",
            manifest_path.display()
        )
    });
    manifest.validate();
    manifest
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

fn assert_non_empty_unique(field: &str, path: &str, values: &[String]) {
    let mut seen = BTreeSet::new();
    for value in values {
        assert!(
            !value.trim().is_empty(),
            "fixture manifest entry has an empty {field} value: {path}"
        );
        assert!(
            seen.insert(value.as_str()),
            "fixture manifest entry has duplicate {field} value `{value}`: {path}"
        );
    }
}

fn is_safe_relative_path(path: &str) -> bool {
    let path = Path::new(path);
    !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}
