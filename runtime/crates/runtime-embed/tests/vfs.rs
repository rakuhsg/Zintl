use runtime_embed::{
    Authority, AuthorizationRequest, AuthorizationResult, RuntimeBuilder, RuntimeError, Source,
    VfsConfig,
};
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Default)]
struct RecordingAuthority {
    paths: Mutex<Vec<String>>,
}

impl Authority for RecordingAuthority {
    fn authorization_requested(&self, request: &AuthorizationRequest<'_>) -> AuthorizationResult {
        self.paths
            .lock()
            .expect("authority paths")
            .push(request.path.to_owned());
        AuthorizationResult::Allow
    }
}

fn temporary_directory(label: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "zintl-vfs-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir(&path).expect("temporary directory");
    path.canonicalize().expect("canonical temporary directory")
}

#[test]
// Verifies VFS reads hide the host root and consult application authority for every access.
fn vfs_reads_are_relative_and_application_authorized() {
    let root = temporary_directory("read");
    fs::write(root.join("hello.txt"), b"hello").expect("fixture");
    let authority = Arc::new(RecordingAuthority::default());
    let runtime = RuntimeBuilder::new()
        .add_vfs(VfsConfig {
            name: "project".into(),
            source: Source::LoadDir { path: root.clone() },
            authority: Some(authority.clone()),
        })
        .expect("register VFS")
        .build()
        .expect("runtime");
    assert!(runtime.vfs_descriptor("project").is_some());
    runtime.start().expect("start");

    for _ in 0..2 {
        assert_eq!(
            runtime
                .read_file("project://hello.txt", 16)
                .expect("submit read")
                .wait()
                .expect("read"),
            b"hello"
        );
    }
    assert!(matches!(
        runtime
            .read_file("/etc/passwd", 16)
            .expect("submit invalid URL")
            .wait(),
        Err(RuntimeError::InvalidArgument)
    ));
    assert!(matches!(
        runtime
            .read_file("project://../hello.txt", 16)
            .expect("submit invalid path")
            .wait(),
        Err(RuntimeError::InvalidArgument)
    ));
    assert_eq!(
        *authority.paths.lock().expect("authority paths"),
        ["hello.txt", "hello.txt"]
    );

    runtime.shutdown().expect("shutdown");
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
// Verifies invalid or duplicate VFS schemes are rejected before opening host paths.
fn vfs_names_are_unique_url_schemes() {
    let root = temporary_directory("names");
    let builder = RuntimeBuilder::new()
        .add_vfs(VfsConfig {
            name: "fs".into(),
            source: Source::LoadDir { path: root.clone() },
            authority: None,
        })
        .expect("first VFS");
    assert!(matches!(
        builder.add_vfs(VfsConfig {
            name: "fs".into(),
            source: Source::LoadDir { path: root.clone() },
            authority: None,
        }),
        Err(RuntimeError::InvalidConfiguration)
    ));
    assert!(matches!(
        RuntimeBuilder::new().add_vfs(VfsConfig {
            name: "Not A Scheme".into(),
            source: Source::LoadDir { path: root.clone() },
            authority: None,
        }),
        Err(RuntimeError::InvalidConfiguration)
    ));
    fs::remove_dir_all(root).expect("cleanup");
}
