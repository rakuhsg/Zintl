use std::path::Path;
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

const WEBGPU_FEATURE_NAME: &str = deno_runtime::deno_webgpu::UNSTABLE_FEATURE_NAME;

#[derive(Debug)]
pub enum RunMainWorkerError {
    Core(CoreError),
    LoadEvent(Box<JsError>),
}

impl std::fmt::Display for RunMainWorkerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunMainWorkerError::Core(error) => write!(f, "{error}"),
            RunMainWorkerError::LoadEvent(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for RunMainWorkerError {}

impl From<CoreError> for RunMainWorkerError {
    fn from(error: CoreError) -> Self {
        RunMainWorkerError::Core(error)
    }
}

impl From<Box<JsError>> for RunMainWorkerError {
    fn from(error: Box<JsError>) -> Self {
        RunMainWorkerError::LoadEvent(error)
    }
}

pub fn main_worker(main_module: impl AsRef<Path>) -> MainWorker {
    let main_module = ModuleSpecifier::from_file_path(main_module)
        .expect("main module must be an absolute file path");
    main_worker_from_specifier(&main_module)
}

pub fn run_main_worker(main_module: impl AsRef<Path>) -> Result<(), RunMainWorkerError> {
    let main_module = ModuleSpecifier::from_file_path(main_module)
        .expect("main module must be an absolute file path");

    create_and_run_current_thread(async move {
        let mut worker = main_worker_from_specifier(&main_module);
        worker.execute_main_module(&main_module).await?;
        worker.dispatch_load_event()?;
        worker.run_event_loop(false).await?;
        Ok(())
    })
}

fn main_worker_from_specifier(main_module: &ModuleSpecifier) -> MainWorker {
    let fs = Arc::new(RealFs);
    let feature_checker = Arc::new(webgpu_feature_checker());
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
        main_module,
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
                unstable_features: unstable_feature_ids(feature_checker.as_ref()),
                ..Default::default()
            },
            ..Default::default()
        },
    )
}

fn webgpu_feature_checker() -> FeatureChecker {
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
