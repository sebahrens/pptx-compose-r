#![deny(warnings)]

pub mod diffs;
pub mod journal;
pub mod media_inputs;
pub mod operations;
pub mod patch;
pub mod reports;
pub mod selectors;

#[cfg(test)]
mod tests {
    #[test]
    fn edit_crate_compiles() {
        let crate_name = env!("CARGO_PKG_NAME");
        assert_eq!(crate_name, "pptx-compose-edit");
    }
}
