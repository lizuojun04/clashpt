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
        /// Read ClashPT settings from this YAML file (default: ~/.config/clashpt.yaml)
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
        /// Read status indicator settings from this YAML file (default: ~/.config/clashpt.yaml)
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
    /// Import a complete Mihomo configuration file
    Import { path: PathBuf },
    /// Import a URL returning a complete Mihomo configuration
    Subscribe {
        /// Omit the URL to read it from standard input
        url: Option<String>,
    },
    /// Fetch and apply the current subscription again
    Update,
    /// Write the original configuration to standard output
    Export,
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

fn run() -> Result<()> {
    let cli = Cli::parse();
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
            let (endpoint, workers) = if cli.system {
                let settings = system::client_settings()?;
                (settings.endpoint, Some(settings.workers))
            } else {
                (endpoint()?, None)
            };
            tui::run(Client::new(endpoint)?, config.as_deref(), workers)?;
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
        Command::Proxy(Proxy::On(output)) => proxy::print(Some(endpoint()?.mixed_port), &output)?,
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
                Profile::Import { path } => service.import(&path)?,
                Profile::Subscribe { url } => {
                    let url = match url {
                        Some(url) => url,
                        None => {
                            let mut url = String::new();
                            io::stdin().read_line(&mut url)?;
                            url.trim().to_owned()
                        }
                    };
                    ensure!(!url.is_empty(), "provide a subscription URL");
                    service.subscribe(url)?;
                }
                Profile::Update => service.update()?,
                Profile::Export => io::stdout().lock().write_all(&service.export()?)?,
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
                Nodes::Select { group, node } => client.select(&group, &node)?,
            }
        }
        Command::System(_) => unreachable!(),
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("clashpt: {error:#}");
        std::process::exit(1);
    }
}
