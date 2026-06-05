#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamespaceBinding {
    pub prefix: Option<String>,
    pub uri: String,
}

impl NamespaceBinding {
    #[must_use]
    pub fn default(uri: impl Into<String>) -> Self {
        Self {
            prefix: None,
            uri: uri.into(),
        }
    }

    #[must_use]
    pub fn prefixed(prefix: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            prefix: Some(prefix.into()),
            uri: uri.into(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NamespaceTable {
    bindings: Vec<NamespaceBinding>,
}

impl NamespaceTable {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            bindings: Vec::new(),
        }
    }

    pub fn push(&mut self, binding: NamespaceBinding) {
        self.bindings.push(binding);
    }

    #[must_use]
    pub fn bindings(&self) -> &[NamespaceBinding] {
        &self.bindings
    }

    #[must_use]
    pub fn resolve_prefix(&self, prefix: Option<&str>) -> Option<&str> {
        self.bindings.iter().rev().find_map(|binding| {
            if binding.prefix.as_deref() == prefix {
                Some(binding.uri.as_str())
            } else {
                None
            }
        })
    }
}
