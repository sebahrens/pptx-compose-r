#![deny(warnings)]

pub mod permissions;
pub mod prompts;
pub mod resources;
pub mod sessions;
pub mod tools;

#[cfg(test)]
mod tests {
    #[test]
    fn mcp_crate_compiles() {
        let crate_name = env!("CARGO_PKG_NAME");
        assert_eq!(crate_name, "pptx-compose-mcp");
    }
}
