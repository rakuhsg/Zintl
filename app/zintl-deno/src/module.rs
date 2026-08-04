use std::path::Path;

use deno_ast::MediaType;
use deno_ast::ParseParams;
use deno_ast::SourceMapOption;
use deno_core::ModuleLoadOptions;
use deno_core::ModuleLoadReferrer;
use deno_core::ModuleLoadResponse;
use deno_core::ModuleLoader;
use deno_core::ModuleSource;
use deno_core::ModuleSourceCode;
use deno_core::ModuleSpecifier;
use deno_core::ModuleType;
use deno_core::RequestedModuleType;
use deno_core::ResolutionKind;
use deno_core::error::ModuleLoaderError;
use deno_core::resolve_import;
use deno_error::JsErrorBox;

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

#[derive(Default)]
pub struct ZintlModuleLoader {
    emitter: SimpleEmitter,
}

impl ZintlModuleLoader {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ModuleLoader for ZintlModuleLoader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        _kind: ResolutionKind,
    ) -> Result<ModuleSpecifier, ModuleLoaderError> {
        resolve_import(specifier, referrer).map_err(JsErrorBox::from_err)
    }

    fn load(
        &self,
        module_specifier: &ModuleSpecifier,
        _maybe_referrer: Option<&ModuleLoadReferrer>,
        options: ModuleLoadOptions,
    ) -> ModuleLoadResponse {
        ModuleLoadResponse::Sync(self.load_module(module_specifier, options))
    }
}

impl ZintlModuleLoader {
    fn load_module(
        &self,
        module_specifier: &ModuleSpecifier,
        options: ModuleLoadOptions,
    ) -> Result<ModuleSource, ModuleLoaderError> {
        let path = module_specifier
            .to_file_path()
            .map_err(|_| JsErrorBox::generic("Zintl only supports file:// module loading"))?;

        if matches!(
            options.requested_module_type,
            RequestedModuleType::Text | RequestedModuleType::Bytes
        ) {
            let code = read_module_bytes(module_specifier, &path)?;
            let module_type = match options.requested_module_type {
                RequestedModuleType::Text => ModuleType::Text,
                RequestedModuleType::Bytes => ModuleType::Bytes,
                _ => unreachable!(),
            };
            return Ok(ModuleSource::new(
                module_type,
                ModuleSourceCode::Bytes(code.into_boxed_slice().into()),
                module_specifier,
                None,
            ));
        }

        let media_type = MediaType::from_path(&path);
        match media_type {
            MediaType::Json => {
                if options.requested_module_type != RequestedModuleType::Json {
                    return Err(JsErrorBox::generic(
                        "Attempted to load JSON module without specifying \"type\": \"json\" attribute in the import statement.",
                    ));
                }
                let code = read_module_bytes(module_specifier, &path)?;
                Ok(ModuleSource::new(
                    ModuleType::Json,
                    ModuleSourceCode::Bytes(code.into_boxed_slice().into()),
                    module_specifier,
                    None,
                ))
            }
            MediaType::Wasm => {
                let code = read_module_bytes(module_specifier, &path)?;
                Ok(ModuleSource::new(
                    ModuleType::Wasm,
                    ModuleSourceCode::Bytes(code.into_boxed_slice().into()),
                    module_specifier,
                    None,
                ))
            }
            media_type if is_emittable(media_type) => {
                let source = read_module_string(module_specifier, &path)?;
                let code = self.emitter.emit(module_specifier, media_type, source)?;
                Ok(ModuleSource::new(
                    ModuleType::JavaScript,
                    ModuleSourceCode::String(code.into()),
                    module_specifier,
                    None,
                ))
            }
            _ => {
                let code = read_module_bytes(module_specifier, &path)?;
                let module_type = match options.requested_module_type {
                    RequestedModuleType::Other(ty) => ModuleType::Other(ty),
                    _ => ModuleType::JavaScript,
                };
                Ok(ModuleSource::new(
                    module_type,
                    ModuleSourceCode::Bytes(code.into_boxed_slice().into()),
                    module_specifier,
                    None,
                ))
            }
        }
    }
}

fn read_module_bytes(
    specifier: &ModuleSpecifier,
    path: &Path,
) -> Result<Vec<u8>, ModuleLoaderError> {
    std::fs::read(path).map_err(|source| {
        JsErrorBox::generic(format!("Failed to load module \"{specifier}\": {source}"))
    })
}

fn read_module_string(
    specifier: &ModuleSpecifier,
    path: &Path,
) -> Result<String, ModuleLoaderError> {
    std::fs::read_to_string(path).map_err(|source| {
        JsErrorBox::generic(format!("Failed to load module \"{specifier}\": {source}"))
    })
}

#[derive(Default)]
struct SimpleEmitter;

impl SimpleEmitter {
    fn emit(
        &self,
        specifier: &ModuleSpecifier,
        media_type: MediaType,
        source: String,
    ) -> Result<String, ModuleLoaderError> {
        let parsed = deno_ast::parse_module(ParseParams {
            specifier: specifier.clone(),
            text: source.into(),
            media_type,
            capture_tokens: false,
            scope_analysis: false,
            maybe_syntax: None,
        })
        .map_err(JsErrorBox::from_err)?;

        let transpiled = parsed
            .transpile(
                &deno_ast::TranspileOptions {
                    imports_not_used_as_values: deno_ast::ImportsNotUsedAsValues::Remove,
                    decorators: deno_ast::DecoratorsTranspileOption::Ecma,
                    ..Default::default()
                },
                &deno_ast::TranspileModuleOptions { module_kind: None },
                &deno_ast::EmitOptions {
                    source_map: SourceMapOption::None,
                    ..Default::default()
                },
            )
            .map_err(JsErrorBox::from_err)?
            .into_source();

        Ok(transpiled.text)
    }
}

fn is_emittable(media_type: MediaType) -> bool {
    matches!(
        media_type,
        MediaType::TypeScript | MediaType::Mts | MediaType::Cts | MediaType::Jsx | MediaType::Tsx
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use deno_core::{JsRuntime, RuntimeOptions};
    use deno_runtime::tokio_util::create_and_run_current_thread;

    use super::*;

    #[test]
    fn runs_typescript_module_graph() {
        let dir = TestDir::new("zintl-deno-ts");
        dir.write(
            "dep.ts",
            r#"
export interface Item {
  label: string;
}

export const item: Item = { label: "zintl" };
"#,
        );
        let main = dir.write(
            "main.ts",
            r#"
import { item } from "./dep.ts";
globalThis.__zintlTestValue = item.label;
"#,
        );

        run_module(main).expect("typescript module graph should run");
    }

    #[test]
    fn requires_json_import_attribute() {
        let dir = TestDir::new("zintl-deno-json-missing-attribute");
        dir.write("data.json", r#"{"name":"zintl"}"#);
        let main = dir.write(
            "main.js",
            r#"
import data from "./data.json";
globalThis.__zintlTestValue = data.name;
"#,
        );

        let error = run_module(main).expect_err("json import without attribute should fail");
        assert!(error.to_string().contains("JSON module"));
    }

    #[test]
    fn loads_json_with_import_attribute() {
        let dir = TestDir::new("zintl-deno-json-with-attribute");
        dir.write("data.json", r#"{"name":"zintl"}"#);
        let main = dir.write(
            "main.js",
            r#"
import data from "./data.json" with { type: "json" };
globalThis.__zintlTestValue = data.name;
"#,
        );

        run_module(main).expect("json import with attribute should run");
    }

    #[test]
    fn transpiles_tsx_modules() {
        let dir = TestDir::new("zintl-deno-tsx");
        let path = dir.write(
            "view.tsx",
            r#"
export const view = <div data-zintl="true" />;
"#,
        );
        let specifier =
            ModuleSpecifier::from_file_path(path).expect("test path should be absolute");
        let loader = ZintlModuleLoader::new();

        let response = loader.load(
            &specifier,
            None,
            ModuleLoadOptions {
                is_dynamic_import: false,
                is_synchronous: false,
                requested_module_type: RequestedModuleType::None,
            },
        );

        let ModuleLoadResponse::Sync(result) = response else {
            panic!("zintl module loader should load local files synchronously");
        };
        let module = result.expect("tsx module should transpile");
        assert_eq!(module.module_type, ModuleType::JavaScript);
    }

    fn run_module(path: PathBuf) -> Result<(), deno_core::error::CoreError> {
        create_and_run_current_thread(async move {
            let specifier =
                ModuleSpecifier::from_file_path(path).expect("test path should be absolute");
            let mut runtime = JsRuntime::new(RuntimeOptions {
                module_loader: Some(Rc::new(ZintlModuleLoader::new())),
                ..Default::default()
            });
            let module_id = runtime.load_main_es_module(&specifier).await?;
            let evaluation = runtime.mod_evaluate(module_id);
            runtime.run_event_loop(Default::default()).await?;
            evaluation.await
        })
    }

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("{name}-{nonce}"));
            std::fs::create_dir_all(&path).expect("test dir should be created");
            Self { path }
        }

        fn write(&self, relative_path: &str, contents: &str) -> PathBuf {
            let path = self.path.join(relative_path);
            std::fs::write(&path, contents).expect("test fixture should be written");
            path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
