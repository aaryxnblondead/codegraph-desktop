//! Language extractor registry.

use std::path::Path;

use super::facts::FileFacts;

#[cfg(feature = "lang-python")]
pub mod python;
#[cfg(feature = "lang-rust")]
pub mod rust_lang;
#[cfg(feature = "lang-typescript")]
pub mod typescript;

/// A language-specific source extractor.
pub trait LanguageExtractor: Send + Sync {
    fn language_id(&self) -> &'static str;
    fn extensions(&self) -> &'static [&'static str];
    /// Extract symbols/imports/refs/metrics. Must never panic.
    fn extract(&self, path: &Path, src: &str) -> FileFacts;
}

/// Return the extractor for a file, or `None` if unsupported.
pub fn extractor_for(path: &Path) -> Option<&'static dyn LanguageExtractor> {
    static EXTRACTORS: &[&dyn LanguageExtractor] = &[
        #[cfg(feature = "lang-typescript")]
        &typescript::TypeScriptExtractor,
        #[cfg(feature = "lang-python")]
        &python::PythonExtractor,
        #[cfg(feature = "lang-rust")]
        &rust_lang::RustExtractor,
    ];
    let ext = path.extension().and_then(|e| e.to_str())?.to_lowercase();
    EXTRACTORS
        .iter()
        .find(|e| e.extensions().iter().any(|x| *x == ext))
        .copied()
}
