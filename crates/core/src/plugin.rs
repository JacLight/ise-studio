use crate::registry::Registry;

/// A capability (sketch, solid, BIM, CAM, an importer...). Plugins register
/// components, commands and derived computations into the one shared space;
/// they read each other's components freely and own no private project state.
pub trait Plugin {
    fn name(&self) -> &'static str;

    fn register(&self, registry: &mut Registry);
}
