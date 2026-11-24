use crate::{api::{Client, Endpoint}, config, gateway};
use anyhow::{Context, Result, bail, ensure};
use std::{
    fs::{self, File, Permissions},
    io::{self, Write},
    os::{fd::AsRawFd, unix::fs::{MetadataExt, PermissionsExt}},
    num::NonZeroUsize,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub const GATEWAY_SOCKET: &str = "/run/clashpt-api/control.sock";
pub const CORE_SOCKET: &str = "/run/clashpt-core/control.sock";
const DIRECTORY: &str = "/etc/clashpt";
const UNIT: &str = "/etc/systemd/system/clashpt.service";
const API_UNIT: &str = "/etc/systemd/system/clashpt-api.service";
const SERVICE: &str = "clashpt.service";

const UNIT_CONTENT: &str = r#"[Unit]
Description=ClashPT Mihomo TUN service
Wants=network-online.target clashpt-api.service
After=network-online.target

[Service]
Type=exec
DynamicUser=yes
User=clashpt
RuntimeDirectory=clashpt-core
RuntimeDirectoryMode=0700
StateDirectory=clashpt
StateDirectoryMode=0700
LoadCredential=config.yaml:/etc/clashpt/config.yaml
ExecStartPre=/etc/clashpt/mihomo -t -d /var/lib/clashpt -f ${CREDENTIALS_DIRECTORY}/config.yaml
ExecStart=/etc/clashpt/mihomo -d /var/lib/clashpt -f ${CREDENTIALS_DIRECTORY}/config.yaml
ExecStartPost=/usr/bin/timeout 10 /bin/sh -c 'until test -e /sys/class/net/clashpt-tun; do sleep 0.1; done'
AmbientCapabilities=CAP_NET_ADMIN CAP_NET_RAW CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_ADMIN CAP_NET_RAW CAP_NET_BIND_SERVICE
NoNewPrivileges=yes
UMask=0077
TimeoutStartSec=45

[Install]
WantedBy=multi-user.target
"#;

const API_CONTENT: &str = r#"[Unit]
Description=ClashPT local node selection API
PartOf=clashpt.service
BindsTo=clashpt.service
After=clashpt.service

[Service]
Type=exec
DynamicUser=yes
User=clashpt
RuntimeDirectory=clashpt-api
RuntimeDirectoryMode=0755
LoadCredential=server.json:/etc/clashpt/server.json
ExecStart=/etc/clashpt/clashpt system serve-api ${CREDENTIALS_DIRECTORY}/server.json /run/clashpt-core/control.sock
NoNewPrivileges=yes
UMask=0077
"#;

pub(crate) fn client_settings() -> Result<gateway::Settings> {
    let file = if unsafe { libc::geteuid() } == 0 { "server.json" } else { "client.json" };
    config::read_json(&Path::new(DIRECTORY).join(file))
        .context("read system API credentials")
}

pub fn endpoint() -> Result<Endpoint> {
    Ok(client_settings()?.endpoint)
}

pub fn control(action: &str) -> Result<()> {
    if unsafe { libc::geteuid() } == 0 {
        let _lock = deployment_lock()?;
        ensure!(check_receipt()?, "no registered system deployment");
        return root_systemctl(&[action, SERVICE]);
    }
    let manager = "/etc/clashpt/clashpt";
    executable(Path::new(manager))?;
    let status = Command::new("/usr/bin/sudo").args(["--", manager, "--system", action])
        .status().context("manage system service")?;
    ensure!(status.success(), "system service {action} failed");
    Ok(())
}

fn root_systemctl(args: &[&str]) -> Result<()> {
    let status = Command::new("/usr/bin/systemctl").args(args).status().context("run systemctl")?;
    ensure!(status.success(), "systemctl {} failed", args.join(" "));
    Ok(())
}

fn executable(path: &Path) -> Result<File> {
    ensure!(path.is_absolute(), "system deployment requires an absolute administrator-owned binary path");
    ensure!(path.canonicalize()? == path, "system executable path must be canonical and free of symlinks");
    for ancestor in path.ancestors() {
        let meta = fs::metadata(ancestor)?;
        ensure!(meta.uid() == 0 && meta.mode() & 0o022 == 0,
            "untrusted system executable path: {}", ancestor.display());
    }
    let file = File::open(path)?;
    let meta = file.metadata()?;
    ensure!(meta.is_file() && meta.mode() & 0o111 != 0, "expected an executable file");
    Ok(file)
}

fn credentials(path: &Path, settings: &gateway::Settings, uid: u32) -> Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    temp.write_all(&serde_json::to_vec(settings)?)?;
    temp.as_file().set_permissions(Permissions::from_mode(0o400))?;
    let result = unsafe { libc::fchown(temp.as_file().as_raw_fd(), uid, 0) };
    if result != 0 {
        return Err(io::Error::last_os_error()).context("grant API credential access");
    }
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    Ok(())
}

fn install_executable(mut input: File, target: &Path) -> Result<()> {
    let mut output = tempfile::NamedTempFile::new_in(target.parent().unwrap())?;
    io::copy(&mut input, &mut output)?;
    output.as_file().set_permissions(Permissions::from_mode(0o755))?;
    output.as_file().sync_all()?;
    output.persist(target)?;
    Ok(())
}

const RECEIPT: &str = "/etc/clashpt/install.json";
const MANAGED: &[&str] = &[
    UNIT, API_UNIT, "/etc/clashpt/clashpt", "/etc/clashpt/mihomo",
    "/etc/clashpt/server.json", "/etc/clashpt/client.json", "/etc/clashpt/config.yaml",
];

fn deployment_identity(path: &Path) -> Result<String> {
    // The designated user owns the client copy. Its contents must never gate an
    // administrator's ability to stop, repair, or remove the deployment.
    if path == Path::new("/etc/clashpt/client.json") {
        return Ok(format!("client-uid:{}", fs::symlink_metadata(path)?.uid()));
    }
    let output = Command::new("/usr/bin/sha256sum").stdin(File::open(path)?).output()?;
    ensure!(output.status.success(), "could not hash {}", path.display());
    Ok(std::str::from_utf8(&output.stdout)?.split_whitespace().next().context("missing digest")?.into())
}

fn deployment_lock() -> Result<File> {
    // Keep the directory inode stable across removal and subsequent deployments.
    let directory = Path::new(DIRECTORY);
    match fs::create_dir(directory) {
        Ok(()) => fs::set_permissions(directory, Permissions::from_mode(0o755))?,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {},
        Err(e) => return Err(e.into()),
    }
    let meta = fs::symlink_metadata(directory)?;
    ensure!(meta.is_dir() && meta.uid() == 0 && meta.mode() & 0o022 == 0,
        "untrusted system deployment directory");
    let lock = File::open(directory)?;
    fs2::FileExt::lock_exclusive(&lock)?;
    Ok(lock)
}

fn check_receipt() -> Result<bool> {
    let receipt = Path::new(RECEIPT);
    let exists = receipt.try_exists()?;
    let sums: std::collections::BTreeMap<String,String> = if exists {
        config::read_json(receipt)?
    } else { Default::default() };
    for name in MANAGED {
        let path = Path::new(name);
        match fs::symlink_metadata(path) {
            Ok(meta) => ensure!(meta.is_file() && sums.get(*name) == Some(&deployment_identity(path)?),
                "unregistered or changed deployment file: {name}"),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {},
            Err(e) => return Err(e.into()),
        }
    }
    Ok(exists)
}

fn save_receipt() -> Result<()> {
    let mut sums = std::collections::BTreeMap::new();
    for name in MANAGED {
        sums.insert(*name, deployment_identity(Path::new(name))?);
    }
    config::write_json(Path::new(RECEIPT), &sums)
}

pub fn remove(keep_mihomo: bool) -> Result<()> {
    ensure!(unsafe { libc::geteuid() } == 0, "administrator privileges required");
    let _lock = deployment_lock()?;
    ensure!(check_receipt()?, "no system installation receipt");
    if Path::new(UNIT).exists() { root_systemctl(&["disable", "--now", SERVICE])?; }
    if Path::new(API_UNIT).exists() { root_systemctl(&["stop", "clashpt-api.service"])?; }
    for name in MANAGED {
        if keep_mihomo && *name == "/etc/clashpt/mihomo" { continue; }
        let path = Path::new(name);
        if path.exists() { fs::remove_file(path)?; }
    }
    root_systemctl(&["daemon-reload"])?;
    if keep_mihomo && Path::new("/etc/clashpt/mihomo").exists() {
        let retained = std::collections::BTreeMap::from([("/etc/clashpt/mihomo", deployment_identity(Path::new("/etc/clashpt/mihomo"))?)]);
        config::write_json(Path::new(RECEIPT), &retained)?;
    } else { fs::remove_file(RECEIPT)?; }
    Ok(())
}

pub fn apply(source: &Path, mihomo: &Path, uid: u32, workers: Option<NonZeroUsize>) -> Result<()> {
    ensure!(unsafe { libc::geteuid() } == 0, "run system apply using sudo");
    ensure!(Path::new("/run/systemd/system").is_dir(), "a running systemd system manager is required");
    ensure!(Path::new("/dev/net/tun").exists(), "the system must provide /dev/net/tun");
    ensure!(uid != u32::MAX, "invalid UID");
    let manager = executable(&std::env::current_exe()?)?;
    let running = fs::metadata("/proc/self/exe")?;
    let installed = manager.metadata()?;
    ensure!(installed.ino() == running.ino() && installed.dev() == running.dev(), "manager binary changed");
    let raw = config::local(source)?;
    let directory = Path::new(DIRECTORY);
    let _lock = deployment_lock()?;
    check_receipt()?;

    let server = directory.join("server.json");
    let client_path = directory.join("client.json");
    let previous_settings: Option<gateway::Settings> = if server.exists() {
        Some(config::read_json(&server)?)
    } else {
        None
    };
    let same_user = fs::metadata(&client_path).is_ok_and(|meta| meta.uid() == uid);
    let previous_endpoint = previous_settings.as_ref().filter(|_| same_user).map(|settings| &settings.endpoint);
    let (endpoint, runtime) = config::runtime(&raw, previous_endpoint, true, None, Path::new(GATEWAY_SOCKET))?;
    let workers = workers
        .or_else(|| previous_settings.as_ref().map(|settings| settings.workers))
        .unwrap_or(gateway::DEFAULT_WORKERS);
    let gateway_settings = gateway::Settings { endpoint, workers };

    let binary = directory.join("mihomo");
    let source_binary = if mihomo == Path::new("mihomo") {
        let default = Path::new("/usr/local/bin/mihomo");
        executable(if default.exists() { default } else { &binary })?
    } else { executable(mihomo)? };
    let active = Command::new("/usr/bin/systemctl").args(["is-active", "--quiet", SERVICE])
        .stdout(Stdio::null()).stderr(Stdio::null()).status()?.success();
    let target = directory.join("config.yaml");
    let manager_path = directory.join("clashpt");
    let paths = [binary.as_path(), manager_path.as_path(), Path::new(UNIT), Path::new(API_UNIT), &server, &target, &client_path];
    let backup = tempfile::Builder::new().prefix(".rollback-")
        .permissions(Permissions::from_mode(0o700)).tempdir_in(directory)?;
    for (i, path) in paths.iter().enumerate() {
        if path.exists() { fs::hard_link(path, backup.path().join(i.to_string())).or_else(|_| fs::copy(path, backup.path().join(i.to_string())).map(|_| ()))?; }
    }
    let result = (|| {
        install_executable(source_binary, &binary)?;
        install_executable(manager, &manager_path)?;
        for (path, content) in [(UNIT, UNIT_CONTENT), (API_UNIT, API_CONTENT)] {
            config::write(Path::new(path), content.as_bytes())?;
            fs::set_permissions(path, Permissions::from_mode(0o644))?;
        }
        root_systemctl(&["daemon-reload"])?;
        config::write_json(&server, &gateway_settings)?;
        config::write(&target, &runtime)?;
        root_systemctl(&["restart", SERVICE])?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while !Client::new(gateway_settings.endpoint.clone()).is_ok_and(|client| client.ready()) {
            ensure!(Instant::now() < deadline, "Mihomo API did not become ready");
            thread::sleep(Duration::from_millis(100));
        }
        credentials(&client_path, &gateway_settings, uid)?;
        save_receipt()
    })();

    if let Err(error) = result {
        let rollback = (|| {
            let _ = root_systemctl(&["stop", SERVICE]);
            for (i, path) in paths.iter().enumerate() {
                let saved = backup.path().join(i.to_string());
                if saved.exists() {
                    let mut restored = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
                    io::copy(&mut File::open(&saved)?, restored.as_file_mut())?;
                    restored.as_file().set_permissions(fs::metadata(&saved)?.permissions())?;
                    let meta = fs::metadata(&saved)?;
                    ensure!(unsafe { libc::fchown(restored.as_file().as_raw_fd(), meta.uid(), meta.gid()) } == 0,
                        "could not restore deployment ownership");
                    restored.persist(path)?;
                } else if path.exists() { fs::remove_file(path)?; }
            }
            root_systemctl(&["daemon-reload"])?;
            if active {
                root_systemctl(&["start", SERVICE])?;
            }
            Ok::<_, anyhow::Error>(())
        })();
        if let Err(rollback) = rollback {
            let retained = backup.keep();
            bail!("system apply failed: {error:#}; rollback failed: {rollback:#}. Backup: {}", retained.display());
        }
        return Err(error).context("system apply failed. See journalctl -u clashpt.service");
    }
    Ok(())
}
