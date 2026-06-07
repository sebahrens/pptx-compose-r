use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const INTERNAL_CRATES: &[&str] = &[
    "pptx-compose",
    "pptx-compose-core",
    "pptx-compose-json",
    "pptx-compose-edit",
    "pptx-compose-cli",
    "pptx-compose-mcp",
];

#[test]
fn architecture_boundaries_match_spec_060() {
    let workspace = workspace_root();
    let manifests = load_manifests(&workspace);

    assert_internal_deps(
        &manifests,
        "pptx-compose-core",
        &[],
        "pptx-compose-core must not depend on json/edit/facade/cli/mcp crates",
    );
    assert_internal_deps(
        &manifests,
        "pptx-compose-json",
        &["pptx-compose-core"],
        "pptx-compose-json may depend on core but not edit/cli/mcp",
    );
    assert_internal_deps(
        &manifests,
        "pptx-compose-edit",
        &["pptx-compose-core", "pptx-compose-json"],
        "pptx-compose-edit may only depend on core/json among internal crates",
    );
    assert_internal_deps(
        &manifests,
        "pptx-compose",
        &[
            "pptx-compose-core",
            "pptx-compose-json",
            "pptx-compose-edit",
        ],
        "pptx-compose facade must depend on core/json/edit among internal crates",
    );

    assert_has_internal_dep(
        &manifests,
        "pptx-compose-cli",
        "pptx-compose",
        "pptx-compose-cli must depend on the public facade crate",
    );
    assert_has_internal_dep(
        &manifests,
        "pptx-compose-mcp",
        "pptx-compose",
        "pptx-compose-mcp must depend on the public facade crate",
    );

    assert_no_forbidden_core_internal_imports(&workspace.join("crates/pptx-compose-cli/src"));
    assert_no_forbidden_core_internal_imports(&workspace.join("crates/pptx-compose-mcp/src"));
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("core crate must live under crates/pptx-compose-core")
        .to_path_buf()
}

fn load_manifests(workspace: &Path) -> BTreeMap<String, BTreeSet<String>> {
    let mut manifests = BTreeMap::new();

    for crate_name in INTERNAL_CRATES {
        let manifest_path = workspace.join("crates").join(crate_name).join("Cargo.toml");
        let manifest = fs::read_to_string(&manifest_path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", manifest_path.display()));
        let manifest = toml::from_str::<toml::Value>(&manifest).unwrap_or_else(|error| {
            panic!(
                "could not parse {} as TOML: {error}",
                manifest_path.display()
            )
        });

        manifests.insert(
            (*crate_name).to_string(),
            internal_dependencies(&manifest, &manifest_path),
        );
    }

    manifests
}

fn internal_dependencies(manifest: &toml::Value, manifest_path: &Path) -> BTreeSet<String> {
    let dependencies = manifest
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .unwrap_or_else(|| panic!("{} has no [dependencies] table", manifest_path.display()));

    dependencies
        .keys()
        .filter(|dependency| INTERNAL_CRATES.contains(&dependency.as_str()))
        .cloned()
        .collect()
}

fn assert_internal_deps(
    manifests: &BTreeMap<String, BTreeSet<String>>,
    crate_name: &str,
    expected: &[&str],
    message: &str,
) {
    let actual = manifests
        .get(crate_name)
        .unwrap_or_else(|| panic!("missing manifest for {crate_name}"));
    let expected = expected
        .iter()
        .map(|dependency| (*dependency).to_string())
        .collect::<BTreeSet<_>>();

    assert_eq!(actual, &expected, "{message}");
}

fn assert_has_internal_dep(
    manifests: &BTreeMap<String, BTreeSet<String>>,
    crate_name: &str,
    expected_dependency: &str,
    message: &str,
) {
    let actual = manifests
        .get(crate_name)
        .unwrap_or_else(|| panic!("missing manifest for {crate_name}"));

    assert!(
        actual.contains(expected_dependency),
        "{message}; actual internal dependencies: {actual:?}"
    );
}

fn assert_no_forbidden_core_internal_imports(src_root: &Path) {
    for path in rust_files(src_root) {
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
        let compact_source = source
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();

        for forbidden_module in ["opc", "xml", "zip", "validation"] {
            let direct_import = format!("pptx_compose_core::{forbidden_module}");
            assert!(
                !compact_source.contains(&direct_import),
                "{} imports forbidden core internal module `{direct_import}`; specs/060 requires CLI and MCP surfaces to use facade/json/edit APIs",
                path.display()
            );

            let facade_bypass = format!("pptx_compose::core::{forbidden_module}");
            assert!(
                !compact_source.contains(&facade_bypass),
                "{} imports forbidden facade core internal module `{facade_bypass}`; specs/060 requires CLI and MCP surfaces to use safe facade/json/edit APIs",
                path.display()
            );

            let grouped_facade_bypass = format!("core::{{...{forbidden_module}");
            assert!(
                !contains_grouped_core_import(&compact_source, forbidden_module),
                "{} imports forbidden facade core internal module via grouped import `{grouped_facade_bypass}`; specs/060 requires CLI and MCP surfaces to use safe facade/json/edit APIs",
                path.display()
            );
        }
    }
}

fn contains_grouped_core_import(source: &str, forbidden_module: &str) -> bool {
    let mut remaining = source;
    while let Some(start) = remaining.find("core::{") {
        let group = &remaining[start + "core::{".len()..];
        let Some(end) = group.find('}') else {
            return false;
        };
        if group[..end].split(',').any(|import| {
            import == forbidden_module || import.starts_with(&format!("{forbidden_module}::"))
        }) {
            return true;
        }
        remaining = &group[end + 1..];
    }
    false
}

fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_rust_files(root, &mut files);
    files.sort();
    files
}

fn collect_rust_files(path: &Path, files: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(path).unwrap_or_else(|error| {
        panic!(
            "could not read source directory {}: {error}",
            path.display()
        )
    });

    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!(
                "could not read source directory entry under {}: {error}",
                path.display()
            )
        });
        let path = entry.path();
        let file_type = entry.file_type().unwrap_or_else(|error| {
            panic!("could not read file type for {}: {error}", path.display())
        });

        if file_type.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}
