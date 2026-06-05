#![deny(warnings)]

pub use pptx_compose_core as core;
pub use pptx_compose_edit as edit;
pub use pptx_compose_json as json;

#[cfg(test)]
mod tests {
    #[test]
    fn facade_crate_compiles() {
        let crate_name = env!("CARGO_PKG_NAME");
        assert_eq!(crate_name, "pptx-compose");
    }
}
