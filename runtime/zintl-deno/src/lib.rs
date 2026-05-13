use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use deno_resolver::npm::DenoInNpmPackageChecker;
use deno_resolver::npm::NpmResolver;
use deno_runtime::BootstrapOptions;
use deno_runtime::FeatureChecker;
use deno_runtime::UNSTABLE_FEATURES;
use deno_runtime::deno_core::FsModuleLoader;
use deno_runtime::deno_core::ModuleSpecifier;
use deno_runtime::deno_core::error::CoreError;
use deno_runtime::deno_core::error::JsError;
use deno_runtime::deno_fs::RealFs;
use deno_runtime::deno_permissions::Permissions;
use deno_runtime::deno_permissions::PermissionsContainer;
use deno_runtime::deno_permissions::RuntimePermissionDescriptorParser;
use deno_runtime::tokio_util::create_and_run_current_thread;
use deno_runtime::worker::MainWorker;
use deno_runtime::worker::WorkerOptions;
use deno_runtime::worker::WorkerServiceOptions;

pub mod api;

const WEBGPU_FEATURE_NAME: &str = deno_runtime::deno_webgpu::UNSTABLE_FEATURE_NAME;

#[derive(Clone, Default)]
pub struct DenoRuntimeOptions {
    pub api: api::ZintlApi,
}

#[derive(Debug)]
pub enum DenoRuntimeError {
    Core(CoreError),
    LoadEvent(Box<JsError>),
}

impl std::fmt::Display for DenoRuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DenoRuntimeError::Core(error) => write!(f, "{error}"),
            DenoRuntimeError::LoadEvent(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for DenoRuntimeError {}

impl From<CoreError> for DenoRuntimeError {
    fn from(error: CoreError) -> Self {
        DenoRuntimeError::Core(error)
    }
}

impl From<Box<JsError>> for DenoRuntimeError {
    fn from(error: Box<JsError>) -> Self {
        DenoRuntimeError::LoadEvent(error)
    }
}

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

pub struct DenoRuntime {
    main_module: MainModule,
    worker: MainWorker,
}

impl DenoRuntime {
    pub fn new(main_module: MainModule) -> Self {
        Self::new_with_options(main_module, DenoRuntimeOptions::default())
    }

    pub fn new_with_options(main_module: MainModule, options: DenoRuntimeOptions) -> Self {
        let worker = Self::create_main_worker_with_options(&main_module, options);
        Self {
            main_module,
            worker,
        }
    }

    pub fn from_file_path(path: impl AsRef<Path>) -> Self {
        Self::new(MainModule::from_file_path(path))
    }

    pub fn from_file_path_with_options(
        path: impl AsRef<Path>,
        options: DenoRuntimeOptions,
    ) -> Self {
        Self::new_with_options(MainModule::from_file_path(path), options)
    }

    pub async fn run(&mut self) -> Result<(), DenoRuntimeError> {
        let main_module = self.main_module.specifier().clone();

        self.worker.execute_main_module(&main_module).await?;
        self.worker.dispatch_load_event()?;
        self.worker.run_event_loop(false).await?;
        Ok(())
    }

    pub fn run_current_thread(mut self) -> Result<(), DenoRuntimeError> {
        create_and_run_current_thread(async move { self.run().await })
    }

    pub fn run_file_path_current_thread(path: PathBuf) -> Result<(), DenoRuntimeError> {
        Self::run_file_path_current_thread_with_options(path, DenoRuntimeOptions::default())
    }

    pub fn run_file_path_current_thread_with_options(
        path: PathBuf,
        options: DenoRuntimeOptions,
    ) -> Result<(), DenoRuntimeError> {
        create_and_run_current_thread(async move {
            let mut runtime = Self::from_file_path_with_options(path, options);
            runtime.run().await
        })
    }

    pub fn into_worker(self) -> MainWorker {
        self.worker
    }

    pub fn create_main_worker(main_module: &MainModule) -> MainWorker {
        Self::create_main_worker_with_options(main_module, DenoRuntimeOptions::default())
    }

    pub fn create_main_worker_with_options(
        main_module: &MainModule,
        options: DenoRuntimeOptions,
    ) -> MainWorker {
        let fs = Arc::new(RealFs);
        let feature_checker = Arc::new(Self::feature_checker());
        let permissions = PermissionsContainer::new(
            Arc::new(RuntimePermissionDescriptorParser::new(
                sys_traits::impls::RealSys,
            )),
            Permissions::none_without_prompt(),
        );

        MainWorker::bootstrap_from_options::<
            DenoInNpmPackageChecker,
            NpmResolver<sys_traits::impls::RealSys>,
            sys_traits::impls::RealSys,
        >(
            main_module.specifier(),
            WorkerServiceOptions {
                module_loader: Rc::new(FsModuleLoader),
                permissions,
                fs,
                deno_rt_native_addon_loader: None,
                blob_store: Default::default(),
                broadcast_channel: Default::default(),
                feature_checker: feature_checker.clone(),
                node_services: None,
                npm_process_state_provider: None,
                root_cert_store_provider: None,
                fetch_dns_resolver: Default::default(),
                shared_array_buffer_store: None,
                compiled_wasm_module_store: None,
                v8_code_cache: None,
                bundle_provider: None,
            },
            WorkerOptions {
                bootstrap: BootstrapOptions {
                    unstable_features: Self::unstable_feature_ids(feature_checker.as_ref()),
                    ..Default::default()
                },
                extensions: vec![api::extension(options.api)],
                ..Default::default()
            },
        )
    }

    pub fn feature_checker() -> FeatureChecker {
        let mut checker = FeatureChecker::default();
        checker.enable_feature(WEBGPU_FEATURE_NAME);
        checker
    }

    fn unstable_feature_ids(feature_checker: &FeatureChecker) -> Vec<i32> {
        UNSTABLE_FEATURES
            .iter()
            .filter(|feature| feature_checker.check(feature.name))
            .map(|feature| feature.id)
            .collect()
    }
}
