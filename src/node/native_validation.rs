//! Test-build-only architecture driver around the real Node bootstrap.
//! Uses real Meta/TLS/P2P and RootManager, not the in-process ContractMeta fixture.
//! Control files belong to a trusted private validation directory. This driver
//! is not compiled into afs-node and is not production native READY/ACK wiring.
use super::vfs::ownerfs::{
    OwnerFs,
    native::{
        HomeExportAuthority, LinuxMountBackend, MountPolicy, NativeMountManager, WorkspaceMount,
    },
};
use crate::{
    config::{Cli, Config, Role},
    runtime::{self, Observability},
};
use clap::Parser;
use serde::Deserialize;
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

static OWNER: OnceLock<mpsc::Sender<Arc<OwnerFs>>> = OnceLock::new();

pub(super) fn enabled() -> bool {
    OWNER.get().is_some()
}

pub(super) fn publish_owner(owner: Arc<OwnerFs>) {
    if let Some(sender) = OWNER.get() {
        let _ = sender.send(owner);
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Command {
    id: String,
    operation: String,
    name: Option<String>,
}

struct Export {
    permit: Arc<HomeExportAuthority>,
    spec: WorkspaceMount,
    manager: NativeMountManager<LinuxMountBackend>,
}

struct Driver {
    owner: Arc<OwnerFs>,
    mount: PathBuf,
    export: Option<Export>,
}

impl Driver {
    fn execute(&mut self, command: &Command) -> Result<serde_json::Value, runtime::BoxError> {
        match command.operation.as_str() {
            "prepare" => {
                if self.export.is_some() {
                    return Err(std::io::Error::other("detach the existing export first").into());
                }
                let name = command
                    .name
                    .as_deref()
                    .ok_or_else(|| std::io::Error::other("prepare requires name"))?;
                let namespace = LinuxMountBackend::current_namespace()?;
                let permit = Arc::new(
                    self.owner
                        .native_home_export(std::ffi::OsStr::new(name), namespace)?,
                );
                let backend = LinuxMountBackend::new(namespace, 4)?;
                let spec = backend.prepare_for_home(
                    &self.owner,
                    &permit,
                    File::open(&self.mount)?,
                    MountPolicy::default(),
                )?;
                let manager = NativeMountManager::new(namespace, 4, backend)?;
                manager.register(spec.clone())?;
                self.export = Some(Export {
                    permit,
                    spec,
                    manager,
                });
                self.status()
            }
            "activate" => {
                let export = self
                    .export
                    .as_ref()
                    .ok_or_else(|| std::io::Error::other("missing prepared export"))?;
                export
                    .manager
                    .activate_for_home(&self.owner, &export.permit)?;
                self.status()
            }
            "detach" => {
                let export = self
                    .export
                    .as_ref()
                    .ok_or_else(|| std::io::Error::other("missing prepared export"))?;
                export.manager.quiesce(&export.spec.identity)?;
                export.manager.detach(&export.spec.identity)?;
                let result = self.status()?;
                self.export = None;
                Ok(result)
            }
            "status" => self.status(),
            _ => Err(std::io::Error::other("unknown validation command").into()),
        }
    }

    fn status(&self) -> Result<serde_json::Value, runtime::BoxError> {
        if let Some(export) = &self.export {
            let status = export
                .manager
                .status(&export.spec.identity.root_id)?
                .ok_or_else(|| std::io::Error::other("missing manager record"))?;
            Ok(serde_json::json!({"state":format!("{:?}", status.state),
                "identity":export.spec.identity, "source":export.spec.source,
                "covered_target":export.spec.target, "observed":status.observed,
                "scope":"validation manager observation, not production READY/ACK"}))
        } else {
            Ok(serde_json::json!({"state":"Unprepared"}))
        }
    }
}

fn write_json(directory: &Path, name: &str, value: &serde_json::Value) -> std::io::Result<()> {
    let temporary = directory.join(format!(".{name}.tmp"));
    fs::write(&temporary, serde_json::to_vec(value)?)?;
    fs::rename(temporary, directory.join(name))
}

fn worker(
    directory: PathBuf,
    mount: PathBuf,
    receiver: mpsc::Receiver<Arc<OwnerFs>>,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    let owner = loop {
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(owner) => break owner,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(error) => return Err(error.to_string()),
        }
    };
    let mut driver = Driver {
        owner,
        mount,
        export: None,
    };
    write_json(&directory, "driver.json", &serde_json::json!({
        "pid":std::process::id(), "namespace":fs::read_link("/proc/self/ns/mnt").map_err(|e| e.to_string())?,
        "constructor":"native-eligible test-only; default/release Node remains ordinary",
        "scope":"test-build-only current Node/Meta/TLS/P2P driver"})).map_err(|e| e.to_string())?;
    while !stop.load(Ordering::Acquire) {
        let input = directory.join("request.json");
        if !input.exists() {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        let meta = fs::symlink_metadata(&input).map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.len() > 65536 {
            return Err("invalid trusted control input".into());
        }
        let bytes = fs::read(&input).map_err(|e| e.to_string())?;
        fs::remove_file(&input).map_err(|e| e.to_string())?;
        let command: Command = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if command.id.is_empty()
            || command.id.len() > 64
            || !command
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err("invalid command id".into());
        }
        let result = match driver.execute(&command) {
            Ok(value) => serde_json::json!({"id":command.id, "ok":true, "result":value}),
            Err(error) => {
                serde_json::json!({"id":command.id, "ok":false, "error":error.to_string()})
            }
        };
        write_json(&directory, &format!("reply-{}.json", command.id), &result)
            .map_err(|e| e.to_string())?;
    }
    if driver.export.is_some() {
        return Err("validation export was not explicitly detached before Node stop".into());
    }
    Ok(())
}

#[test]
#[ignore = "real Node architecture driver; dedicated VM namespace and explicit private control/config"]
fn privileged_native_validation_node() {
    assert_eq!(
        std::env::var("AFS_NATIVE_PRIVATE_NAMESPACE").as_deref(),
        Ok("1")
    );
    let config =
        std::env::var("AFS_NATIVE_VALIDATION_CONFIG").expect("explicit Node config required");
    let directory = PathBuf::from(
        std::env::var("AFS_NATIVE_VALIDATION_CONTROL").expect("private control directory required"),
    );
    assert!(Path::new(&config).is_absolute() && directory.is_absolute() && directory.is_dir());
    let cfg = Config::resolve(
        Role::Node,
        Cli::try_parse_from(["native-validation", "--config", &config]).unwrap(),
    )
    .unwrap();
    assert!(cfg.ownerfs && !cfg.dfs && cfg.data_mode == "grpc");
    assert!(
        cfg.tls_ca_certificate.is_some()
            && cfg.tls_identity_certificate.is_some()
            && cfg.tls_identity_private_key.is_some()
    );
    let mount = cfg
        .ownerfs_mount
        .clone()
        .expect("explicit OwnerFs mount required");
    let (sender, receiver) = mpsc::channel();
    OWNER.set(sender).expect("one validation Node per process");
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let worker = std::thread::spawn(move || worker(directory, mount, receiver, worker_stop));
    let deadline = runtime::ShutdownDeadline::for_process(Duration::from_secs(15)).unwrap();
    let trigger = deadline.trigger();
    let executor = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let obs = Observability::new().unwrap();
    let guards = runtime::initialize(&cfg, "native-validation-node", &obs).unwrap();
    let result = executor.block_on(super::run_with_shutdown(cfg, obs, trigger.clone()));
    stop.store(true, Ordering::Release);
    let worker_result = worker.join().unwrap();
    trigger.arm();
    drop(executor);
    drop(guards);
    deadline.complete();
    result.unwrap();
    worker_result.unwrap();
}
