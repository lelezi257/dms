mod center;
mod home_fuse;
mod p2p_rpc;

use std::{
    collections::HashSet,
    env, fs,
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    process::Command,
    sync::{Arc, RwLock},
    thread,
    time::Duration,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    match args.as_slice() {
        [_, command, rpc, management, state] if command == "center" => {
            center::serve_durable(rpc, management, &PathBuf::from(state))?;
        }
        [_, command, name, rpc] if command == "locate" => {
            println!("{}", center::query(rpc, &format!("GET {name}"))?);
        }
        [_, command, rpc] if command == "roots" => {
            println!("{}", center::query(rpc, "ROOTS")?);
        }
        [_, command, id, rpc, nfs_endpoint, p2p_address, data_root, peers, mount, backend]
            if command == "node" =>
        {
            let token = env::var("DMS_HOME_TOKEN")
                .map_err(|_| "DMS_HOME_TOKEN must be set for node authentication")?;
            if token.len() < 16 {
                return Err("DMS_HOME_TOKEN must have at least 16 bytes".into());
            }
            let backend = home_fuse::BackendMode::parse(backend)?;
            let data_root = PathBuf::from(data_root);
            let peers = PathBuf::from(peers);
            fs::create_dir_all(&data_root)?;
            fs::create_dir_all(&peers)?;
            fs::create_dir_all(mount)?;
            // Bind before advertising an endpoint; a failed P2P listener must fail startup.
            let p2p_listener = TcpListener::bind(p2p_address)?;
            let answer = center::query(
                rpc,
                &format!("NODE {id} {nfs_endpoint} {p2p_address}"),
            )?;
            if answer != "OK" {
                return Err(format!("node registration failed: {answer}").into());
            }
            let server_root = data_root.clone();
            let server_token = token.clone();
            let private_cache = Arc::new(home_fuse::PrivateAttrCache::new());
            let mounted = Arc::new(RwLock::new(HashSet::new()));
            let watcher = MembershipWatcher {
                id: id.clone(),
                center_addr: rpc.clone(),
                nfs_endpoint: nfs_endpoint.clone(),
                p2p_address: p2p_address.clone(),
                peers_root: peers.clone(),
                mounted: mounted.clone(),
                backend,
            };
            thread::spawn(move || watch_membership(watcher));
            let home_fs = home_fuse::HomeFs::new(
                id.clone(),
                rpc.clone(),
                data_root,
                peers,
                mounted,
                backend,
                token.clone(),
            )
            .with_private_cache(private_cache.clone());
            let mut session = fuser::Session::new(
                home_fs,
                mount,
                &[
                    fuser::MountOption::FSName("dms-home".into()),
                    fuser::MountOption::DefaultPermissions,
                    fuser::MountOption::NoAtime,
                ],
            )?;
            let notifier = session.notifier();
            thread::spawn(move || {
                if let Err(error) = p2p_rpc::serve_listener_with_private_cache(
                    p2p_listener,
                    server_root,
                    server_token,
                    private_cache,
                    notifier,
                ) {
                    eprintln!("home P2P server stopped: {error}");
                }
            });
            session.run()?;
        }
        _ => return Err("usage: dms-home center RPC_ADDR MANAGEMENT_HTTP_ADDR STATE_FILE | locate ROOT RPC_ADDR | roots RPC_ADDR | node NODE_ID CENTER_RPC NFS_ENDPOINT P2P_ADDR DATA_ROOT PEER_MOUNTS FUSE_MOUNT nfs|p2p (DMS_HOME_TOKEN environment required)".into()),
    }
    Ok(())
}

struct MembershipWatcher {
    id: String,
    center_addr: String,
    nfs_endpoint: String,
    p2p_address: String,
    peers_root: PathBuf,
    mounted: Arc<RwLock<HashSet<String>>>,
    backend: home_fuse::BackendMode,
}

fn watch_membership(watcher: MembershipWatcher) {
    loop {
        // Node membership is deliberately ephemeral at the center. Re-register after restart.
        let registered = center::query(
            &watcher.center_addr,
            &format!(
                "NODE {} {} {}",
                watcher.id, watcher.nfs_endpoint, watcher.p2p_address
            ),
        );
        if !matches!(registered.as_deref(), Ok("OK")) {
            thread::sleep(Duration::from_secs(1));
            continue;
        }
        if watcher.backend != home_fuse::BackendMode::Nfs {
            thread::sleep(Duration::from_secs(1));
            continue;
        }
        match center::query(&watcher.center_addr, "NODES") {
            Ok(rows) => {
                for row in rows.lines() {
                    let fields: Vec<_> = row.split_whitespace().collect();
                    if fields.len() != 3 || fields[0] == watcher.id || !center::safe(fields[0]) {
                        continue;
                    }
                    let (peer, endpoint) = (fields[0], fields[1]);
                    if !nfs_endpoint_reachable(endpoint) {
                        watcher.mounted.write().unwrap().remove(peer);
                        continue;
                    }
                    let target = watcher.peers_root.join(peer);
                    if fs::create_dir_all(&target).is_err() {
                        continue;
                    }
                    let active = Command::new("mountpoint")
                        .arg("-q")
                        .arg(&target)
                        .status()
                        .is_ok_and(|status| status.success());
                    if !active {
                        let mut command = if unsafe { libc::geteuid() } == 0 {
                            Command::new("mount")
                        } else {
                            let mut command = Command::new("sudo");
                            command.args(["-n", "mount"]);
                            command
                        };
                        let status = command
                            .args([
                                "-t",
                                "nfs4",
                                "-o",
                                "vers=4.2,proto=tcp,hard,actimeo=0,lookupcache=none,cto,timeo=100,retrans=2",
                                endpoint,
                            ])
                            .arg(&target)
                            .status();
                        if !status.is_ok_and(|status| status.success()) {
                            watcher.mounted.write().unwrap().remove(peer);
                            continue;
                        }
                    }
                    watcher.mounted.write().unwrap().insert(peer.to_owned());
                }
            }
            Err(error) => eprintln!("node discovery: {error}"),
        }
        thread::sleep(Duration::from_secs(1));
    }
}

fn nfs_endpoint_reachable(endpoint: &str) -> bool {
    let Some(host) = endpoint.strip_suffix(":/") else {
        return false;
    };
    let Ok(address) = format!("{host}:2049").parse::<SocketAddr>() else {
        return false;
    };
    TcpStream::connect_timeout(&address, Duration::from_millis(250)).is_ok()
}
