#![deny(warnings)]

pub mod agent_view;
pub mod binary_encoding;
pub mod legacy_path_map;
pub mod schema_versions;
pub mod schemas;

#[cfg(test)]
mod tests {
    #[test]
    fn json_crate_compiles() {
        let crate_name = env!("CARGO_PKG_NAME");
        assert_eq!(crate_name, "pptx-compose-json");
    }
}
