use crate::{api::{Client, Endpoint}, config};
use anyhow::{Context, Result, bail, ensure};
use std::{
    fs::{self, File, OpenOptions, Permissions},
    io::{self, Write},
    os::{fd::AsRawFd, unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt}},
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

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
LoadCredential=server.json:/etc/clashpt/server.json
ExecStart=/etc/clashpt/clashpt system serve-api ${CREDENTIALS_DIRECTORY}/server.json /run/clashpt-core/control.sock
NoNewPrivileges=yes
UMask=0077
"#;

pub fn endpoint() -> Result<Endpoint> {
    config::read_json(&Path::new(DIRECTORY).join("client.json"))
        .context("read system API credentials")
}

pub fn control(action: &str) -> Result<()> {
    let mut command = if unsafe { libc::geteuid() } == 0 {
        Command::new("/usr/bin/systemctl")
    } else {
        let mut command = Command::new("sudo");
        command.arg("/usr/bin/systemctl");
        command
    };
    let status = command.args([action, SERVICE]).status().context("run systemctl")?;
    ensure!(status.success(), "systemctl {action} {SERVICE} failed");
    Ok(())
}

fn root_systemctl(args: &[&str]) -> Result<()> {
    let status = Command::new("/usr/bin/systemctl").args(args).status().context("run systemctl")?;
    ensure!(status.success(), "systemctl {} failed", args.join(" "));
    Ok(())
}

fn executable(path: &Path) -> Result<File> {
    let candidates = if path.components().count() > 1 || path.is_absolute() {
        vec![path.to_path_buf()]
    } else {
        std::env::var_os("PATH").into_iter()
            .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
            .map(|dir| dir.join(path)).collect()
    };
    for path in candidates {
        if let Ok(file) = File::open(&path) {
            let metadata = file.metadata()?;
            if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
                return Ok(file);
            }
        }
    }
    bail!("Mihomo executable not found. Pass --mihomo /absolute/path/to/mihomo");
}

fn credentials(path: &Path, endpoint: &Endpoint, uid: u32) -> Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    temp.write_all(&serde_json::to_vec(endpoint)?)?;
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

pub fn apply(source: &Path, mihomo: &Path, uid: u32) -> Result<()> {
    ensure!(unsafe { libc::geteuid() } == 0, "run system apply using sudo");
    ensure!(Path::new("/run/systemd/system").is_dir(), "a running systemd system manager is required");
    ensure!(Path::new("/dev/net/tun").exists(), "the system must provide /dev/net/tun");
    ensure!(uid != u32::MAX, "invalid UID");
    let raw = config::local(source)?;
    let directory = Path::new(DIRECTORY);
    fs::create_dir_all(directory)?;
    fs::set_permissions(directory, Permissions::from_mode(0o755))?;
    let _lock = OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600)
        .open(directory.join("apply.lock"))?;
    fs2::FileExt::lock_exclusive(&_lock)?;

    let server = directory.join("server.json");
    let client_path = directory.join("client.json");
    let previous_endpoint: Option<Endpoint> = if server.exists() {
        Some(config::read_json(&server)?)
    } else {
        None
    };
    let (mut endpoint, runtime) = config::runtime(&raw, previous_endpoint.as_ref(), true, None)?;
    if fs::metadata(&client_path).is_ok_and(|metadata| metadata.uid() != uid) {
        endpoint.secret = config::secret()?;
    }
    let binary = directory.join("mihomo");
    if !binary.exists() {
        install_executable(executable(mihomo)?, &binary)?;
    }
    install_executable(File::open("/proc/self/exe")?, &directory.join("clashpt"))?;

    let active = Command::new("/usr/bin/systemctl").args(["is-active", "--quiet", SERVICE])
        .stdout(Stdio::null()).stderr(Stdio::null()).status()?.success();
    let target = directory.join("config.yaml");
    let previous = match fs::read(&target) {
        Ok(value) => Some(value),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    config::write(Path::new(UNIT), UNIT_CONTENT.as_bytes())?;
    fs::set_permissions(UNIT, Permissions::from_mode(0o644))?;
    config::write(Path::new(API_UNIT), API_CONTENT.as_bytes())?;
    fs::set_permissions(API_UNIT, Permissions::from_mode(0o644))?;
    root_systemctl(&["daemon-reload"])?;
    let result = (|| {
        config::write_json(&server, &endpoint)?;
        config::write(&target, &runtime)?;
        root_systemctl(&["restart", SERVICE])?;
        let client = Client::new(endpoint.clone())?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while !client.ready() {
            ensure!(Instant::now() < deadline, "Mihomo API did not become ready");
            thread::sleep(Duration::from_millis(100));
        }
        credentials(&client_path, &endpoint, uid)
    })();

    if let Err(error) = result {
        let rollback = (|| {
            root_systemctl(&["stop", SERVICE])?;
            match previous {
                Some(previous) => config::write(&target, &previous)?,
                None if target.exists() => fs::remove_file(&target)?,
                None => {}
            }
            match previous_endpoint {
                Some(previous) => config::write_json(&server, &previous)?,
                None if server.exists() => fs::remove_file(&server)?,
                None => {}
            }
            if active {
                root_systemctl(&["start", SERVICE])?;
            }
            Ok::<_, anyhow::Error>(())
        })();
        if let Err(rollback) = rollback {
            bail!("system apply failed: {error:#}; rollback failed: {rollback:#}");
        }
        return Err(error).context("system apply failed. See journalctl -u clashpt.service");
    }
    Ok(())
}
