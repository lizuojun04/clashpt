mod api;
mod config;
mod gateway;
mod proxy;
mod service;
mod settings;
mod status;
mod system;
mod tui;

use anyhow::{Context, Result, bail, ensure};
use api::Client;
use clap::{Parser, Subcommand};
use service::Service;
use std::{io::{self, Write}, path::PathBuf};

#[derive(Parser)]
#[command(version, about = "Manage Mihomo from the terminal")]
struct Cli {
    #[arg(long, global = true, env = "CLASHPT_DATA_DIR")]
    data_dir: Option<PathBuf>,
    #[arg(long, global = true, env = "CLASHPT_MIHOMO", default_value = "mihomo")]
    mihomo: PathBuf,
    #[arg(long, global = true, help = "Use the system service with TUN")]
    system: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Open the interactive Mihomo dashboard
    Tui {
        /// Read ClashPT settings from this YAML file (default: ~/.config/clashpt/clashpt.yaml)
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Start Mihomo
    Start,
    /// Stop Mihomo
    Stop,
    /// Configure the current shell proxy with the zsh integration, or print shell commands
    #[command(subcommand)]
    Proxy(Proxy),
    /// Show service status and the local proxy address
    Status {
        #[arg(long)]
        json: bool,
        /// Read status indicator settings from this YAML file (default: ~/.config/clashpt/clashpt.yaml)
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Manage a local configuration or subscription
    #[command(subcommand)]
    Profile(Profile),
    /// List and switch proxy group selections
    #[command(subcommand)]
    Nodes(Nodes),
    /// Install or update the system service as an administrator
    #[command(subcommand)]
    System(System),
    #[command(hide = true)]
    InstallSettings,
    #[command(hide = true)]
    UninstallRuntime,
}

#[derive(Subcommand)]
enum Proxy {
    /// Export proxy variables for the configured mixed port
    On(proxy::Output),
    /// Unset proxy variables
    Off(proxy::Output),
}

#[derive(Subcommand)]
enum Profile {
    /// Save a complete Mihomo configuration file
    Import {
        path: PathBuf,
        /// Profile name (default: reuse the source or pick a new name)
        #[arg(long)]
        name: Option<String>,
    },
    /// Save a URL returning a complete Mihomo configuration
    Subscribe {
        /// Omit the URL to read it from standard input
        url: Option<String>,
        /// Profile name (default: reuse the source or pick a new name)
        #[arg(long)]
        name: Option<String>,
    },
    /// List saved profiles and show which one is active
    List {
        #[arg(long)]
        json: bool,
    },
    /// Activate a saved profile
    Use { name: String },
    /// Delete a saved profile
    #[command(visible_alias = "remove")]
    Delete { name: String },
    /// Download a saved subscription again (default: the active profile)
    Update { name: Option<String> },
    /// Write a profile's original YAML to standard output (default: the active profile)
    Export { name: Option<String> },
}

#[derive(Subcommand)]
enum Nodes {
    /// List groups, their current selection, and available nodes
    List {
        #[arg(long)]
        json: bool,
    },
    /// Select a node or nested group
    Select { group: String, node: String },
}

#[derive(Subcommand)]
enum System {
    /// Remove files owned by the system deployment
    Remove { #[arg(long)] keep_mihomo: bool },
    /// Install/update the service, start TUN and grant API access to a user
    Apply {
        config: PathBuf,
        /// Numeric UID allowed to control the service (default: SUDO_UID)
        #[arg(long)]
        uid: Option<u32>,
        /// Read system.gateway_workers from this ClashPT settings file
        #[arg(long)]
        settings: Option<PathBuf>,
    },
    #[command(hide = true)]
    ServeApi { credentials: PathBuf, socket: PathBuf },
}

fn run(cli: Cli) -> Result<()> {
    if matches!(cli.command, Command::InstallSettings) {
        return settings::Settings::install_defaults();
    }

    if let Command::System(System::Remove { keep_mihomo }) = cli.command { return system::remove(keep_mihomo); }

    if let Command::System(System::Apply { config, uid, settings }) = cli.command {
        let uid = uid.or_else(|| std::env::var("SUDO_UID").ok()?.parse().ok())
            .context("pass --uid with the UID of the user who will control the service")?;
        let workers = match settings {
            Some(path) => settings::Settings::load(Some(&path))?.system.gateway_workers,
            None => None,
        };
        return system::apply(&config, &cli.mihomo, uid, workers);
    }

    if let Command::System(System::ServeApi { credentials, socket }) = cli.command {
        return gateway::run(&credentials, &socket);
    }

    if let Command::Proxy(Proxy::Off(ref output)) = cli.command {
        return proxy::print(None, output);
    }

    let service = if cli.system { None } else { Some(Service::new(cli.data_dir, cli.mihomo)?) };
    let endpoint = || {
        match &service {
            Some(service) => Ok(service.profile()?.context("import a configuration first")?.endpoint),
            None => system::endpoint(),
        }
    };

    match cli.command {
        Command::Tui { config } => {
            let (backend, workers) = if let Some(service) = &service {
                (tui::Backend::user(service.clone()), None)
            } else {
                let settings = system::client_settings()?;
                (tui::Backend::system(Client::new(settings.endpoint)?), Some(settings.workers))
            };
            tui::run(backend, config.as_deref(), workers)?;
        }
        Command::Start => {
            match &service {
                Some(service) => service.start()?,
                None => system::control("start")?,
            }
            status::print_state(true)?;
        }
        Command::Stop => {
            match &service {
                Some(service) => service.stop()?,
                None => system::control("stop")?,
            }
            status::print_state(false)?;
        }
        Command::Proxy(Proxy::On(output)) => proxy::print(Some(endpoint()?), &output)?,
        Command::Proxy(Proxy::Off(_)) => unreachable!(),
        Command::Status { json, config } => match &service {
            Some(service) => {
                let (endpoint, running) = service.status()?;
                status::print(endpoint, running, json, config.as_deref())?;
            }
            None => {
                let endpoint = system::endpoint()?;
                let running = Client::new(endpoint.clone())?.ready();
                status::print(Some(endpoint), running, json, config.as_deref())?;
            }
        },
        Command::Profile(profile) => {
            let Some(service) = service else {
                bail!("use profile commands in user mode, then run sudo clashpt system apply <file>");
            };
            match profile {
                Profile::Import { path, name } => {
                    let name = service.import(&path, name.as_deref())?;
                    status::print_success(&format!("Saved profile {name}"))?;
                }
                Profile::Subscribe { url, name } => {
                    let url = match url {
                        Some(url) => url,
                        None => {
                            let mut url = String::new();
                            io::stdin().read_line(&mut url)?;
                            url.trim().to_owned()
                        }
                    };
                    ensure!(!url.is_empty(), "provide a subscription URL");
                    let name = service.subscribe(url, name.as_deref())?;
                    status::print_success(&format!("Saved profile {name}"))?;
                }
                Profile::List { json } => {
                    let profiles = service.list_profiles()?;
                    if json {
                        println!("{}", serde_json::to_string(&profiles)?);
                    } else {
                        status::print_profiles(&profiles)?;
                    }
                }
                Profile::Use { name } => {
                    service.activate(&name)?;
                    status::print_success(&format!("Active profile: {}", name.trim()))?;
                }
                Profile::Delete { name } => {
                    let deleted = service.delete_profile(&name)?;
                    status::print_success(&deleted.message(&name))?;
                }
                Profile::Update { name } => {
                    let name = service.update(name.as_deref())?;
                    status::print_success(&format!("Updated profile {name}"))?;
                }
                Profile::Export { name } => io::stdout().lock().write_all(&service.export(name.as_deref())?)?,
            }
        }
        Command::Nodes(command) => {
            let client = Client::new(endpoint()?)?;
            match command {
                Nodes::List { json: true } => {
                    println!("{}", serde_json::to_string(&client.groups()?)?);
                }
                Nodes::List { json: false } => {
                    for group in client.groups()? {
                        println!("{} ({})", group.name, group.kind);
                        for node in group.all {
                            println!("  {} {node}", if node == group.now { '*' } else { ' ' });
                        }
                    }
                }
                Nodes::Select { group, node } => {
                    client.select(&group, &node)?;
                    status::print_success("Node switched")?;
                }
            }
        }
        Command::UninstallRuntime => service.context("expected user mode")?.uninstall_runtime()?,
        Command::System(_) | Command::InstallSettings => unreachable!(),
    }
    Ok(())
}

fn main() {
    let cli = Cli::parse();
    let profile_operation = matches!(&cli.command,
        Command::Profile(Profile::Import { .. } | Profile::Subscribe { .. } | Profile::Delete { .. }));
    if let Err(error) = run(cli) {
        let message = format!("clashpt: {error:#}");
        if !profile_operation || status::print_failure(&message).is_err() {
            eprintln!("{message}");
        }
        std::process::exit(1);
    }
}
