//! Drop-in content packs: a library folder of pack directories and ZIPs,
//! each holding one JSON manifest. Art and audio packs share this layout.
mod library;
mod source;
pub use library::{Library, PackEntry, library_directory, resolve_selection, selection_value};
pub use source::{Member, Source, read_bounded};

/// What distinguishes one pack type's files and library folder.
#[derive(Debug)]
pub struct Kind {
    /// Manifest file name inside a pack folder or ZIP.
    pub manifest: &'static str,
    /// Library folder beside the executable or in the checkout.
    pub folder: &'static str,
    /// Environment variable that overrides the library folder.
    pub env: &'static str,
    /// Largest accepted ZIP.
    pub max_bytes: usize,
    /// The manifest's display name, after validating its file references.
    pub name: fn(&mut Source) -> Result<String, String>,
}
