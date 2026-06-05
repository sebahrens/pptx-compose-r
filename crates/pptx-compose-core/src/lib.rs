#![deny(warnings)]

pub mod error;
pub mod opc;
pub mod pptx;
pub mod provenance;
pub mod validation;
pub mod xml;
pub mod zip;

#[cfg(test)]
mod tests {
    #[test]
    fn core_crate_compiles() {
        let crate_name = env!("CARGO_PKG_NAME");
        assert_eq!(crate_name, "pptx-compose-core");
    }
}
