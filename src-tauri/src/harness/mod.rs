pub mod framework;
pub mod adapters;

use std::path::Path;
use crate::model::{Capabilities, HarnessItem, Scope};
use crate::error::WardError;

pub struct Ctx<'a> {
    pub home: &'a Path,
    pub cwd: Option<&'a Path>,
}

pub trait Harness: Send + Sync {
    fn id(&self) -> &str;
    fn display_name(&self) -> &str;
    fn short_name(&self) -> &str;
    fn icon(&self) -> &str;
    fn executable(&self) -> &str;
    fn category_ids(&self) -> Vec<&'static str>;
    fn capabilities(&self) -> Capabilities;
    fn discover_scopes(&self, ctx: &Ctx) -> Result<Vec<Scope>, WardError>;
    fn scan_category(&self, ctx: &Ctx, category: &str, scope: &Scope)
        -> Result<Vec<HarnessItem>, WardError>;
}

pub struct Registry {
    adapters: Vec<Box<dyn Harness>>,
}

impl Registry {
    pub fn new() -> Self {
        Registry { adapters: Vec::new() }
    }
    pub fn register(&mut self, adapter: Box<dyn Harness>) {
        self.adapters.push(adapter);
    }
    pub fn get(&self, id: &str) -> Option<&dyn Harness> {
        self.adapters.iter().map(|b| b.as_ref()).find(|a| a.id() == id)
    }
    pub fn list(&self) -> Vec<&dyn Harness> {
        self.adapters.iter().map(|b| b.as_ref()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake;
    impl Harness for Fake {
        fn id(&self) -> &str { "fake" }
        fn display_name(&self) -> &str { "Fake" }
        fn short_name(&self) -> &str { "Fk" }
        fn icon(&self) -> &str { "◆" }
        fn executable(&self) -> &str { "fake" }
        fn category_ids(&self) -> Vec<&'static str> { vec!["skill"] }
        fn capabilities(&self) -> Capabilities {
            Capabilities { context_budget: false, mcp_controls: false, mcp_policy: false,
                mcp_security: false, sessions: false, effective: false, backup: false }
        }
        fn discover_scopes(&self, _ctx: &Ctx) -> Result<Vec<Scope>, WardError> { Ok(vec![]) }
        fn scan_category(&self, _c: &Ctx, _cat: &str, _s: &Scope) -> Result<Vec<HarnessItem>, WardError> { Ok(vec![]) }
    }

    #[test]
    fn registry_registers_and_finds() {
        let mut r = Registry::new();
        r.register(Box::new(Fake));
        assert!(r.get("fake").is_some());
        assert!(r.get("nope").is_none());
        assert_eq!(r.list().len(), 1);
    }
}