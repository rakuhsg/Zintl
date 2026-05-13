use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use deno_resolver::npm::DenoInNpmPackageChecker;
use deno_resolver::npm::NpmResolver;
use deno_runtime::deno_core::FsModuleLoader;
use deno_runtime::deno_core::ModuleSpecifier;
use deno_runtime::deno_fs::RealFs;
use deno_runtime::deno_permissions::Permissions;
use deno_runtime::deno_permissions::PermissionsContainer;
use deno_runtime::deno_permissions::RuntimePermissionDescriptorParser;
use deno_runtime::worker::MainWorker;
use deno_runtime::worker::WorkerOptions;
use deno_runtime::worker::WorkerServiceOptions;

pub fn main_worker(main_module: impl AsRef<Path>) -> MainWorker {
    let main_module = ModuleSpecifier::from_file_path(main_module)
        .expect("main module must be an absolute file path");
    let fs = Arc::new(RealFs);
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
        &main_module,
        WorkerServiceOptions {
            module_loader: Rc::new(FsModuleLoader),
            permissions,
            fs,
            deno_rt_native_addon_loader: None,
            blob_store: Default::default(),
            broadcast_channel: Default::default(),
            feature_checker: Default::default(),
            node_services: None,
            npm_process_state_provider: None,
            root_cert_store_provider: None,
            fetch_dns_resolver: Default::default(),
            shared_array_buffer_store: None,
            compiled_wasm_module_store: None,
            v8_code_cache: None,
            bundle_provider: None,
        },
        WorkerOptions::default(),
    )
}
