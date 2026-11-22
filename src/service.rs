use crate::{api::{Client, Endpoint}, config};
use anyhow::{Context, Result, bail, ensure};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    net::TcpListener,
    os::{fd::{AsRawFd, FromRawFd}, unix::{fs::OpenOptionsExt, process::CommandExt}},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Deserialize, Serialize)]
pub struct Profile {
    pub endpoint: Endpoint,
    generation: String,
    source: Source,
}

#[derive(Deserialize, Serialize)]
enum Source {
    File { base: PathBuf },
    Subscription { url: String },
}

#[derive(Deserialize, Serialize)]
struct Process {
    pid: u32,
    start_time: String,
    boot_id: String,
}

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
        let file = OpenOptions::new().create(true).truncate(false).read(true).write(true).mode(0o600)
            .open(self.dir.join("operation.lock"))?;
        FileExt::lock_exclusive(&file)?;
        Ok(file)
    }

    fn profile_path(&self) -> PathBuf {
        self.dir.join("profile.json")
    }

    fn process_path(&self) -> PathBuf {
        self.dir.join("process.json")
    }

    pub fn profile(&self) -> Result<Option<Profile>> {
        optional_json(&self.profile_path())
    }

    fn config_path(&self, profile: &Profile) -> PathBuf {
        self.dir.join(&profile.generation).join("config.yaml")
    }

    fn command(&self, profile: &Profile) -> Command {
        let mut command = Command::new(&self.mihomo);
        command.arg("-d").arg(self.dir.join("core")).arg("-f").arg(self.config_path(profile));
        if let Source::File { base } = &profile.source {
            command.env("SAFE_PATHS", base);
        } else {
            command.env_remove("SAFE_PATHS");
        }
        command
    }

    pub fn import(&self, path: &Path) -> Result<()> {
        let path = path.canonicalize().with_context(|| format!("resolve {}", path.display()))?;
        let raw = config::local(&path)?;
        let base = path.parent().context("configuration has no parent directory")?.to_path_buf();
        let _lock = self.lock()?;
        self.apply(&raw, Source::File { base })
    }

    pub fn subscribe(&self, url: String) -> Result<()> {
        let _lock = self.lock()?;
        let raw = config::download(&url)?;
        self.apply(&raw, Source::Subscription { url })
    }

    pub fn update(&self) -> Result<()> {
        let _lock = self.lock()?;
        let profile = self.profile()?.context("subscribe to a configuration first")?;
        let Source::Subscription { url } = profile.source else {
            bail!("current configuration was imported from a local file");
        };
        let raw = config::download(&url)?;
        self.apply(&raw, Source::Subscription { url })
    }

    pub fn export(&self) -> Result<Vec<u8>> {
        let _lock = self.lock()?;
        let profile = self.profile()?.context("import a configuration first")?;
        config::local(&self.dir.join(profile.generation).join("source.yaml"))
    }

    fn apply(&self, raw: &[u8], source: Source) -> Result<()> {
        let previous = self.profile()?;
        let base = match &source {
            Source::File { base } => Some(base.as_path()),
            Source::Subscription { .. } => None,
        };
        let previous_endpoint = previous.as_ref().map(|profile| &profile.endpoint);
        let (endpoint, runtime) = config::runtime(raw, previous_endpoint, false, base)?;
        let staging = tempfile::Builder::new().prefix("profile-").tempdir_in(&self.dir)?;
        let generation = staging.path().file_name().unwrap().to_str().unwrap().to_owned();
        let candidate = Profile { endpoint, generation, source };
        config::write(&staging.path().join("source.yaml"), raw)?;
        config::write(&self.config_path(&candidate), &runtime)?;

        let log = append_log(&self.dir.join("validate.log"))?;
        let validated = self.command(&candidate).arg("-t")
            .stdin(Stdio::null()).stdout(log.try_clone()?).stderr(log).status()
            .context("run Mihomo configuration validation")?;
        ensure!(validated.success(), "Mihomo rejected the configuration. See {}", self.dir.join("validate.log").display());

        let running = self.running_process()?.is_some();
        if running {
            self.stop_locked()?;
        }
        let result = (|| {
            if running {
                self.start_locked(&candidate)?;
            }
            config::write_json(&self.profile_path(), &candidate)
        })();

        if let Err(error) = result {
            if running {
                let rollback = self.stop_locked().and_then(|_| {
                    self.start_locked(previous.as_ref().context("previous profile is missing")?)
                });
                if let Err(rollback) = rollback {
                    bail!("apply failed: {error:#}; restarting previous profile failed: {rollback:#}");
                }
            }
            return Err(error);
        }

        let _ = staging.keep();
        if let Some(previous) = previous {
            let _ = fs::remove_dir_all(self.dir.join(previous.generation));
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
            ensure!(Client::new(profile.endpoint)?.ready(), "Mihomo is running but its API is unavailable");
            return Ok(());
        }
        self.start_locked(&profile)
    }

    fn start_locked(&self, profile: &Profile) -> Result<()> {
        for port in [profile.endpoint.controller_port, profile.endpoint.mixed_port] {
            TcpListener::bind(("127.0.0.1", port)).with_context(|| format!("local port {port} is occupied"))?;
        }
        let log_path = self.dir.join("mihomo.log");
        let log = append_log(&log_path)?;
        let mut command = self.command(profile);
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
