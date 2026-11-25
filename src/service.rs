use crate::{api::{Client, Endpoint}, config};
use anyhow::{Context, Result, bail, ensure};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    net::TcpListener,
    os::{fd::{AsRawFd, FromRawFd}, unix::{fs::OpenOptionsExt, process::CommandExt}},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Deserialize, Serialize)]
pub struct Profile {
    pub endpoint: Endpoint,
    generation: String,
    source: Source,
    #[serde(default)]
    core: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
enum Source {
    File {
        base: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<PathBuf>,
    },
    Subscription { url: String },
}

#[derive(Default, Deserialize, Serialize)]
struct Profiles {
    active: Option<String>,
    profiles: BTreeMap<String, Profile>,
}

impl Profiles {
    fn active(&self) -> Option<&Profile> {
        self.active.as_ref().and_then(|name| self.profiles.get(name))
    }

    fn name(&self, name: Option<&str>) -> Result<String> {
        let name = name.or(self.active.as_deref()).unwrap_or("default").trim();
        ensure!(!name.is_empty() && !name.chars().any(char::is_control),
            "profile name must be nonempty and contain no control characters");
        Ok(name.to_owned())
    }

    fn get(&self, name: &str) -> Result<&Profile> {
        self.profiles.get(name).with_context(|| format!("profile {name:?} was not found"))
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StoredProfiles {
    Current(Profiles),
    Legacy(Profile),
}

#[derive(Serialize)]
pub struct ProfileInfo {
    pub name: String,
    pub source: &'static str,
    pub active: bool,
    pub mixed_port: u16,
}

pub struct DeletedProfile {
    pub active: Option<String>,
    pub active_changed: bool,
}

impl DeletedProfile {
    pub fn message(&self, name: &str) -> String {
        let mut message = format!("Deleted profile {}", name.trim());
        if self.active_changed {
            match &self.active {
                Some(active) => message.push_str(&format!(" · Active profile: {active}")),
                None => message.push_str(" · No active profile"),
            }
        }
        message
    }
}

#[derive(Deserialize, Serialize)]
struct Process {
    pid: u32,
    start_time: String,
    boot_id: String,
}

#[derive(Clone)]
pub struct Service {
    dir: PathBuf,
    mihomo: PathBuf,
}

impl Service {
    pub fn new(dir: Option<PathBuf>, mihomo: PathBuf) -> Result<Self> {
        let dir = dir.or_else(|| {
            std::env::var_os("XDG_CONFIG_HOME").filter(|path| !path.is_empty())
                .map(PathBuf::from).filter(|path| path.is_absolute())
                .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
                .map(|path| path.join("clashpt"))
        }).context("set CLASHPT_DATA_DIR or HOME")?;
        config::private_dir(&dir)?;
        config::private_dir(&dir.join("core"))?;
        Ok(Self { dir: dir.canonicalize()?, mihomo })
    }

    fn lock(&self) -> Result<File> {
        // The lock inode also coordinates already-running clients during upgrades.
        let file = OpenOptions::new().create(true).truncate(false).read(true).write(true)
            .mode(0o600).custom_flags(libc::O_NOFOLLOW).open(self.dir.join("operation.lock"))?;
        FileExt::lock_exclusive(&file)?;
        Ok(file)
    }

    pub fn uninstall_runtime(&self) -> Result<()> {
        let _lock = self.lock()?;
        let profiles = self.profiles()?;
        self.stop_locked()?;
        for profile in profiles.profiles.values() {
            for (parent, name, files) in [
                (self.dir.clone(), Some(&profile.generation), &["source.yaml", "config.yaml"][..]),
                (self.dir.join("core"), profile.core.as_ref(), &["cache.db", "geoip.metadb", "geoip.dat", "geosite.dat", "ASN.mmdb", "Country.mmdb"][..]),
            ] {
                let Some(name) = name else { continue };
                let directory = parent.join(name);
                if !name.starts_with("profile-") || name.contains('/') ||
                    !fs::symlink_metadata(&parent).is_ok_and(|m| m.is_dir()) ||
                    !fs::symlink_metadata(&directory).is_ok_and(|m| m.is_dir()) { continue; }
                for file in files {
                    let path = directory.join(file);
                    if fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) { fs::remove_file(path)?; }
                }
                let _ = fs::remove_dir(directory);
            }
        }
        for name in ["profile.json", "validate.log", "mihomo.log", ".clashpt-runtime"] {
            match fs::remove_file(self.dir.join(name)) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        Ok(())
    }

    fn profile_path(&self) -> PathBuf {
        self.dir.join("profile.json")
    }

    fn process_path(&self) -> PathBuf {
        self.dir.join("process.json")
    }

    fn profiles(&self) -> Result<Profiles> {
        let mut profiles = match optional_json(&self.profile_path())? {
            Some(StoredProfiles::Current(profiles)) => profiles,
            Some(StoredProfiles::Legacy(profile)) => {
                let name = String::from("default");
                Profiles { active: Some(name.clone()), profiles: BTreeMap::from([(name, profile)]) }
            }
            None => Profiles::default(),
        };
        if !profiles.profiles.is_empty() {
            let socket = self.control_socket()?;
            for profile in profiles.profiles.values_mut() { profile.endpoint.socket = socket.clone(); }
        }
        Ok(profiles)
    }

    pub fn profile(&self) -> Result<Option<Profile>> {
        Ok(self.profiles()?.active().cloned())
    }

    pub fn list_profiles(&self) -> Result<Vec<ProfileInfo>> {
        let Profiles { active, profiles } = self.profiles()?;
        Ok(profiles.into_iter().map(|(name, profile)| {
            let selected = active.as_ref() == Some(&name);
            let source = match profile.source {
                Source::File { .. } => "file",
                Source::Subscription { .. } => "subscription",
            };
            ProfileInfo { name, source, active: selected, mixed_port: profile.endpoint.mixed_port }
        }).collect())
    }

    fn config_path(&self, profile: &Profile) -> PathBuf {
        self.dir.join(&profile.generation).join("config.yaml")
    }

    fn command(&self, profile: &Profile) -> Result<Command> {
        let mut command = Command::new(&self.mihomo);
        let mut core_dir = self.dir.join("core");
        if let Some(core) = &profile.core {
            core_dir.push(core);
        }
        let exe = std::env::current_exe()?;
        if let Some(prefix) = exe.parent().and_then(Path::parent) {
            let seed = prefix.join("share/clashpt/geodata");
            if seed.is_dir() {
                for name in ["geoip.metadb", "geoip.dat", "geosite.dat", "ASN.mmdb"] {
                    let dest = core_dir.join(name);
                    if dest.try_exists()? { continue; }
                    if name == "geoip.metadb" && (core_dir.join("Country.mmdb").try_exists()? ||
                        core_dir.join("geoip.db").try_exists()?) { continue; }
                    let mut file = tempfile::NamedTempFile::new_in(&core_dir)?;
                    io::copy(&mut File::open(seed.join(name))?, file.as_file_mut())?;
                    file.persist_noclobber(dest)?;
                }
            }
        }
        command.arg("-d").arg(core_dir).arg("-f").arg(self.config_path(profile));
        if let Source::File { base, .. } = &profile.source {
            command.env("SAFE_PATHS", base);
        } else {
            command.env_remove("SAFE_PATHS");
        }
        Ok(command)
    }

    pub fn import(&self, path: &Path, name: Option<&str>) -> Result<String> {
        let path = path.canonicalize().with_context(|| format!("resolve {}", path.display()))?;
        let base = path.parent().context("configuration has no parent directory")?.to_path_buf();
        let _lock = self.lock()?;
        let raw = config::local(&path)?;
        self.save(self.profiles()?, name, &raw, Source::File { base, path: Some(path) })
    }

    pub fn subscribe(&self, url: String, name: Option<&str>) -> Result<String> {
        let _lock = self.lock()?;
        let profiles = self.profiles()?;
        let url = url.trim().to_owned();
        let raw = config::download(&url)?;
        self.save(profiles, name, &raw, Source::Subscription { url })
    }

    pub fn update(&self, name: Option<&str>) -> Result<String> {
        let _lock = self.lock()?;
        let profiles = self.profiles()?;
        let name = profiles.name(name)?;
        let Source::Subscription { url } = &profiles.get(&name)?.source else {
            bail!("profile {name:?} was imported from a local file");
        };
        let url = url.clone();
        let raw = config::download(&url)?;
        self.save(profiles, Some(&name), &raw, Source::Subscription { url })
    }

    pub fn export(&self, name: Option<&str>) -> Result<Vec<u8>> {
        let _lock = self.lock()?;
        let profiles = self.profiles()?;
        let profile = profiles.get(&profiles.name(name)?)?;
        config::local(&self.dir.join(&profile.generation).join("source.yaml"))
    }

    pub fn activate(&self, name: &str) -> Result<bool> {
        let _lock = self.lock()?;
        let mut profiles = self.profiles()?;
        let name = profiles.name(Some(name))?;
        profiles.get(&name)?;
        if profiles.active.as_ref() == Some(&name) {
            return Ok(false);
        }
        let previous = profiles.active().cloned();
        profiles.active = Some(name);
        self.commit(previous.as_ref(), &profiles)?;
        Ok(true)
    }

    pub fn delete_profile(&self, name: &str) -> Result<DeletedProfile> {
        let _lock = self.lock()?;
        let mut profiles = self.profiles()?;
        let name = profiles.name(Some(name))?;
        let previous = profiles.active().cloned();
        let removed = profiles.profiles.remove(&name)
            .with_context(|| format!("profile {name:?} was not found"))?;
        let active_changed = profiles.active.as_ref() == Some(&name);
        if active_changed {
            profiles.active = profiles.profiles.keys().next().cloned();
        }
        self.commit(previous.as_ref(), &profiles)?;

        remove_profile_dir(&self.dir, &removed.generation);
        if let Some(core) = removed.core {
            remove_profile_dir(&self.dir.join("core"), &core);
        }
        Ok(DeletedProfile { active: profiles.active, active_changed })
    }

    fn save_name(&self, profiles: &Profiles, name: Option<&str>, source: &Source, raw: &[u8]) -> Result<String> {
        if let Some(name) = name {
            return profiles.name(Some(name));
        }
        if profiles.profiles.is_empty() {
            return Ok("default".into());
        }

        let same_source = |profile: &Profile| match (&profile.source, source) {
            (Source::Subscription { url: old }, Source::Subscription { url: new }) => old == new,
            (Source::File { path: Some(old), .. }, Source::File { path: Some(new), .. }) => old == new,
            (Source::File { base: old, path: None }, Source::File { base: new, .. }) if old == new => {
                config::local(&self.dir.join(&profile.generation).join("source.yaml"))
                    .is_ok_and(|saved| saved == raw)
            }
            _ => false,
        };
        if let Some(active) = &profiles.active && profiles.active().is_some_and(same_source) {
            return Ok(active.clone());
        }
        if let Some((existing, _)) = profiles.profiles.iter().find(|(_, profile)| same_source(profile)) {
            return Ok(existing.clone());
        }

        let base = match source {
            Source::File { path, .. } => path.as_deref().and_then(Path::file_stem)
                .and_then(|name| name.to_str()).map(str::trim)
                .filter(|name| !name.is_empty() && !name.chars().any(char::is_control))
                .unwrap_or("file"),
            Source::Subscription { .. } => "subscription",
        };
        if !profiles.profiles.contains_key(base) {
            return Ok(base.to_owned());
        }
        (2..).map(|suffix| format!("{base}-{suffix}"))
            .find(|name| !profiles.profiles.contains_key(name))
            .context("could not choose a profile name")
    }

    fn control_socket(&self) -> Result<PathBuf> {
        use std::{hash::{Hash, Hasher, DefaultHasher}, os::unix::fs::{MetadataExt, DirBuilderExt}};
        let uid = unsafe { libc::geteuid() };
        let local = PathBuf::from(format!("/run/user/{uid}"));
        let directory = if crate::api::private_socket_directory(&local).is_ok() {
            local.join("clashpt")
        } else {
            if let Some(base) = ["/tmp", "/dev/shm"].into_iter().find(|base| {
                fs::symlink_metadata(base).is_ok_and(|m| m.is_dir() && m.uid() == 0 && m.mode() & 0o1000 != 0)
            }) {
                Path::new(base).join(format!("clashpt-control-{uid}"))
            } else {
                let home = std::env::var_os("HOME").context("set HOME for private control sockets")?;
                PathBuf::from(home).canonicalize()?.join(".clashpt-control")
            }
        };
        match fs::DirBuilder::new().mode(0o700).create(&directory) {
            Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e.into()),
            _ => {}
        }
        crate::api::private_socket_directory(&directory)?;
        let mut hash = DefaultHasher::new();
        self.dir.hash(&mut hash);
        fs::read("/etc/machine-id")?.hash(&mut hash);
        Ok(directory.join(format!("{:016x}.sock", hash.finish())))
    }

    fn save(&self, mut profiles: Profiles, name: Option<&str>, raw: &[u8], source: Source) -> Result<String> {
        let name = self.save_name(&profiles, name, &source, raw)?;
        let existing = profiles.profiles.get(&name);
        let previous = profiles.active().cloned();
        let previous_endpoint = existing.or(previous.as_ref()).map(|profile| &profile.endpoint);
        let base = match &source {
            Source::File { base, .. } => Some(base.as_path()),
            Source::Subscription { .. } => None,
        };
        let (endpoint, runtime) = config::runtime(raw, previous_endpoint, false, base, &self.control_socket()?)?;
        let staging = tempfile::Builder::new().prefix("profile-").tempdir_in(&self.dir)?;
        let generation = directory_name(staging.path())?;
        let new_core = if existing.is_none() {
            Some(tempfile::Builder::new().prefix("profile-").tempdir_in(self.dir.join("core"))?)
        } else {
            None
        };
        let core = match &new_core {
            Some(dir) => Some(directory_name(dir.path())?),
            None => existing.and_then(|profile| profile.core.clone()),
        };
        let candidate = Profile { endpoint, generation, source, core };
        config::write(&staging.path().join("source.yaml"), raw)?;
        config::write(&self.config_path(&candidate), &runtime)?;

        let log = append_log(&self.dir.join("validate.log"))?;
        let validated = self.command(&candidate)?.arg("-t")
            .stdin(Stdio::null()).stdout(log.try_clone()?).stderr(log).status()
            .context("run Mihomo configuration validation")?;
        ensure!(validated.success(), "Mihomo rejected the configuration. See {}", self.dir.join("validate.log").display());

        let replaced = profiles.profiles.insert(name.clone(), candidate);
        if profiles.active.is_none() {
            profiles.active = Some(name.clone());
        }
        self.commit(previous.as_ref(), &profiles)?;
        let _ = staging.keep();
        if let Some(core) = new_core {
            let _ = core.keep();
        }
        if let Some(replaced) = replaced {
            remove_profile_dir(&self.dir, &replaced.generation);
        }
        Ok(name)
    }

    fn commit(&self, previous: Option<&Profile>, profiles: &Profiles) -> Result<()> {
        let next = profiles.active();
        ensure!(next.is_some() || profiles.profiles.is_empty(), "active profile is missing");
        let changed = previous.map(|profile| &profile.generation) != next.map(|profile| &profile.generation);
        let running = changed && self.running_process()?.is_some();
        if running {
            self.stop_locked()?;
        }
        let result = (|| {
            if running && let Some(next) = next {
                self.start_locked(next)?;
            }
            config::write_json(&self.profile_path(), profiles)
        })();

        if let Err(error) = result {
            if running {
                let rollback = self.stop_locked().and_then(|_| {
                    self.start_locked(previous.context("previous profile is missing")?)
                });
                if let Err(rollback) = rollback {
                    bail!("apply failed: {error:#}; restarting previous profile failed: {rollback:#}");
                }
            }
            return Err(error);
        }
        Ok(())
    }

    fn running_process(&self) -> Result<Option<File>> {
        let Some(process): Option<Process> = optional_json(&self.process_path())? else {
            return Ok(None);
        };
        process.open()
    }

    pub fn status(&self) -> Result<(Option<Endpoint>, bool)> {
        let _lock = self.lock()?;
        let endpoint = self.profile()?.map(|profile| profile.endpoint);
        let running = self.running_process()?.is_some() && endpoint.as_ref()
            .is_some_and(|e| Client::new(e.clone()).is_ok_and(|client| client.ready()));
        Ok((endpoint, running))
    }

    pub fn start(&self) -> Result<()> {
        let _lock = self.lock()?;
        let profile = self.profile()?.context("import a configuration first")?;
        if self.running_process()?.is_some() {
            if Client::new(profile.endpoint.clone())?.ready() && !self.refresh_runtime(&profile)? {
                return Ok(());
            }
            self.stop_locked()?;

        }
        self.start_locked(&profile)
    }

    fn refresh_runtime(&self, profile: &Profile) -> Result<bool> {
        // The copied runtime YAML can still contain the previous host's controller.
        let path = self.config_path(profile);
        let mut yaml: serde_yaml::Mapping = serde_yaml::from_slice(&config::local(&path)?)?;
        let socket = serde_yaml::Value::String(profile.endpoint.socket.to_str().context("invalid socket path")?.into());
        let mut changed = false;
        for key in ["external-controller", "secret", "authentication", "skip-auth-prefixes"] {
            changed |= yaml.remove(key).is_some();
        }
        if yaml.get("external-controller-unix") != Some(&socket) {
            yaml.insert("external-controller-unix".into(), socket);
            changed = true;
        }
        if changed { config::write(&path, serde_yaml::to_string(&yaml)?.as_bytes())?; }
        Ok(changed)
    }

    fn start_locked(&self, profile: &Profile) -> Result<()> {
        self.refresh_runtime(profile)?;
        let port = profile.endpoint.mixed_port;
        TcpListener::bind(("127.0.0.1", port)).with_context(|| format!("local port {port} is occupied"))?;
        let log_path = self.dir.join("mihomo.log");
        let log = append_log(&log_path)?;
        let mut command = self.command(profile)?;
        // Detach from the invoking terminal, including when launched over SSH.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 { Err(io::Error::last_os_error()) } else { Ok(()) }
            });
        }
        let mut child = command.stdin(Stdio::null()).stdout(log.try_clone()?).stderr(log)
            .spawn().context("start Mihomo")?;

        let result = (|| {
            let pid = child.id();
            let start_time = process_start(pid)?.context("Mihomo exited during startup")?;
            config::write_json(&self.process_path(), &Process { pid, start_time, boot_id: boot_id()? })?;

            let client = Client::new(profile.endpoint.clone())?;
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if let Some(status) = child.try_wait()? {
                    bail!("Mihomo exited with {status}");
                }
                if client.ready() {
                    return Ok(());
                }
                thread::sleep(Duration::from_millis(100));
            }
            bail!("timed out waiting for the Mihomo API");
        })();

        if result.is_err() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = fs::remove_file(self.process_path());
        }
        result.with_context(|| format!("Mihomo startup failed. See {}", log_path.display()))
    }

    pub fn stop(&self) -> Result<()> {
        let _lock = self.lock()?;
        self.stop_locked()
    }

    fn stop_locked(&self) -> Result<()> {
        if let Some(process) = self.running_process()? {
            signal(&process, libc::SIGTERM)?;
            if !wait(&process, 3000)? {
                signal(&process, libc::SIGKILL)?;
                ensure!(wait(&process, 3000)?, "Mihomo did not exit");
            }
        }
        match fs::remove_file(self.process_path()) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error.into()),
            _ => Ok(()),
        }
    }
}

fn remove_profile_dir(parent: &Path, name: &str) {
    if name.starts_with("profile-") && !name.contains('/') {
        let _ = fs::remove_dir_all(parent.join(name));
    }
}

fn directory_name(path: &Path) -> Result<String> {
    Ok(path.file_name().and_then(|name| name.to_str()).context("profile directory name")?.to_owned())
}

fn optional_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match File::open(path) {
        Ok(file) => Ok(Some(serde_json::from_reader(file).with_context(|| format!("decode {}", path.display()))?)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn append_log(path: &Path) -> Result<File> {
    let mut file = OpenOptions::new().create(true).append(true).mode(0o600).open(path)?;
    writeln!(file)?;
    Ok(file)
}

fn process_start(pid: u32) -> Result<Option<String>> {
    let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let (_, fields) = stat.rsplit_once(')').context("invalid process stat")?;
    let mut fields = fields.split_whitespace();
    if matches!(fields.next(), Some("Z" | "X")) {
        return Ok(None);
    }
    Ok(Some(fields.nth(18).context("process start time missing")?.to_owned()))
}

fn boot_id() -> Result<String> {
    Ok(fs::read_to_string("/proc/sys/kernel/random/boot_id")?)
}

impl Process {
    fn open(&self) -> Result<Option<File>> {
        if boot_id()? != self.boot_id {
            return Ok(None);
        }
        if process_start(self.pid)?.as_ref() != Some(&self.start_time) {
            return Ok(None);
        }
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, self.pid, 0) };
        if fd < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                return Ok(None);
            }
            return Err(error).context("open Mihomo process handle (requires Linux 5.3 or later)");
        }
        let file = unsafe { File::from_raw_fd(fd as _) };
        Ok((process_start(self.pid)?.as_ref() == Some(&self.start_time)).then_some(file))
    }
}

fn signal(process: &File, sig: i32) -> Result<()> {
    let result = unsafe {
        libc::syscall(libc::SYS_pidfd_send_signal, process.as_raw_fd(), sig, std::ptr::null::<libc::siginfo_t>(), 0)
    };
    if result < 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error).context("signal Mihomo");
        }
    }
    Ok(())
}

fn wait(process: &File, timeout_ms: i32) -> Result<bool> {
    let mut fd = libc::pollfd { fd: process.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    let result = unsafe { libc::poll(&mut fd, 1, timeout_ms) };
    if result < 0 {
        return Err(io::Error::last_os_error()).context("wait for Mihomo");
    }
    Ok(result > 0)
}
