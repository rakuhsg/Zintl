use std::path::Path;

use deno_runtime::deno_core::ModuleSpecifier;

pub struct MainModule {
    specifier: ModuleSpecifier,
}

impl MainModule {
    pub fn from_file_path(path: impl AsRef<Path>) -> Self {
        let specifier = ModuleSpecifier::from_file_path(path)
            .expect("main module must be an absolute file path");
        Self { specifier }
    }

    pub fn specifier(&self) -> &ModuleSpecifier {
        &self.specifier
    }
}
