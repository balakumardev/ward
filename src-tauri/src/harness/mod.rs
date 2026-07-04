pub mod framework;
pub mod adapters;

use std::path::Path;
use crate::model::{Capabilities, Destination, HarnessItem, RestoreInfo, Scope};
use crate::error::WardError;

pub struct Ctx<'a> {
    pub home: &'a Path,
    pub cwd: Option<&'a Path>,
}

/// Optional mutation surface for a Harness adapter. Adapters that
/// support move / delete / restore / save_file implement this trait
/// and expose it via [`Harness::operations`]. The default [`Harness`]
/// methods below return "unsupported" — adapters that don't ship
/// mutations (e.g. a future read-only scanner) just don't override
/// [`Harness::operations`].
pub trait HarnessOps: Send + Sync {
    /// Valid move destinations for `item`, scoped to `scopes` already
    /// discovered by this harness.
    fn get_valid_destinations(&self, ctx: &Ctx, item: &HarnessItem, scopes: &[Scope]) -> Vec<Destination>;

    /// Move `item` to the scope identified by `dest_scope_id`. Returns
    /// an undo payload the caller stores for a future `restore`.
    fn move_item(&self, ctx: &Ctx, item: &HarnessItem, dest_scope_id: &str, scopes: &[Scope])
        -> Result<RestoreInfo, WardError>;

    /// Delete `item`. Captures enough state in the returned `RestoreInfo`
    /// that `restore` can recreate the item in its original location.
    fn delete_item(&self, ctx: &Ctx, item: &HarnessItem, scopes: &[Scope])
        -> Result<RestoreInfo, WardError>;

    /// Reverse a previous `move_item` / `delete_item` using its captured
    /// `RestoreInfo`.
    fn restore(&self, ctx: &Ctx, info: &RestoreInfo) -> Result<(), WardError>;

    /// Write `content` to `path` after validating it stays under `home`.
    fn save_file(&self, ctx: &Ctx, path: &str, content: &str) -> Result<(), WardError>;
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

    /// Optional mutation surface. `None` means this adapter is
    /// read-only; the Tauri commands fall back to an "unsupported"
    /// error in that case.
    fn operations(&self) -> Option<&dyn HarnessOps> { None }
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