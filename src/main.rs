#[cfg(target_os = "macos")]
use std::os::unix::process::CommandExt;
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;

use aegis::api::server;
use aegis::host::LoadedAegisClient;
use aegis::{
    BrowserConfig, BrowserMode, Command, NativeConfiguration, SessionState, build_xcode,
    configure_xcode, native, replay_trace,
};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "aegis")]
#[command(about = "AEGIS agent-native browser runtime")]
struct Cli {
    #[arg(long, global = true)]
    host_lib: Option<PathBuf>,
    #[arg(long, global = true, default_value = "headless")]
    mode: BrowserModeArg,
    #[arg(long, global = true)]
    start_url: Option<String>,
    #[arg(long, global = true)]
    user_data_dir: Option<String>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Clone, clap::ValueEnum)]
enum BrowserModeArg {
    Headless,
    Headful,
}

#[derive(Subcommand)]
enum Commands {
    Serve {
        #[arg(long, default_value = "127.0.0.1:7878")]
        addr: SocketAddr,
    },
    Navigate {
        url: String,
    },
    Execute {
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long)]
        json: Option<String>,
    },
    SnapshotDom,
    Session {
        #[command(subcommand)]
        command: SessionCommands,
    },
    Trace {
        #[command(subcommand)]
        command: TraceCommands,
    },
    Native {
        #[command(subcommand)]
        command: NativeCommands,
    },
    Events {
        #[arg(long, default_value_t = 0)]
        since: u64,
    },
}

#[derive(Subcommand)]
enum SessionCommands {
    Inject { file: PathBuf },
    Snapshot,
}

#[derive(Subcommand)]
enum TraceCommands {
    Enable { path: PathBuf },
    Replay { path: PathBuf },
}

#[derive(Subcommand)]
enum NativeCommands {
    Status,
    Configure,
    Build {
        #[arg(long, value_enum, default_value = "debug")]
        configuration: NativeConfigurationArg,
        #[arg(long)]
        scheme: Option<String>,
    },
    Paths,
}

#[derive(Clone, clap::ValueEnum)]
enum NativeConfigurationArg {
    Debug,
    Release,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    maybe_reexec_runtime_command_from_bundle(&cli, Path::new("."))?;
    let host_lib = cli.host_lib.clone();
    let browser_config = BrowserConfig {
        mode: match cli.mode {
            BrowserModeArg::Headless => BrowserMode::Headless,
            BrowserModeArg::Headful => BrowserMode::Headful,
        },
        start_url: cli.start_url.clone(),
        user_data_dir: cli.user_data_dir.clone(),
    };

    match cli.command {
        Commands::Trace {
            command: TraceCommands::Replay { path },
        } => {
            let state = replay_trace(path)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "session": state.session,
                    "final_snapshot": state.final_snapshot,
                    "latest_event_sequence": state.events.latest_sequence()
                }))?
            );
            return Ok(());
        }
        Commands::Native { command } => {
            handle_native_command(command, Path::new("."))?;
            return Ok(());
        }
        _ => {}
    }

    match cli.command {
        Commands::Serve { addr } => {
            let host_lib =
                host_lib.ok_or("`--host-lib` is required for runtime-backed commands")?;
            server::serve(addr, host_lib, browser_config).await?;
        }
        Commands::Navigate { url } => {
            let host_lib =
                host_lib.ok_or("`--host-lib` is required for runtime-backed commands")?;
            let mut client = LoadedAegisClient::connect(host_lib, browser_config)?;
            let events = client.navigate(url)?;
            println!("{}", serde_json::to_string_pretty(&events)?);
        }
        Commands::Execute { file, json } => {
            let host_lib =
                host_lib.ok_or("`--host-lib` is required for runtime-backed commands")?;
            let mut client = LoadedAegisClient::connect(host_lib, browser_config)?;
            let commands = load_commands(file, json)?;
            let report = client.execute(&commands)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Commands::SnapshotDom => {
            let host_lib =
                host_lib.ok_or("`--host-lib` is required for runtime-backed commands")?;
            let mut client = LoadedAegisClient::connect(host_lib, browser_config)?;
            println!("{}", serde_json::to_string_pretty(&client.snapshot_dom())?);
        }
        Commands::Session { command } => match command {
            SessionCommands::Inject { file } => {
                let host_lib =
                    host_lib.ok_or("`--host-lib` is required for runtime-backed commands")?;
                let mut client = LoadedAegisClient::connect(host_lib, browser_config)?;
                let session: SessionState = serde_json::from_slice(&fs::read(file)?)?;
                client.inject_session(session)?;
            }
            SessionCommands::Snapshot => {
                let host_lib =
                    host_lib.ok_or("`--host-lib` is required for runtime-backed commands")?;
                let mut client = LoadedAegisClient::connect(host_lib, browser_config)?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&client.snapshot_session()?)?
                );
            }
        },
        Commands::Trace { command } => match command {
            TraceCommands::Enable { path } => {
                let host_lib =
                    host_lib.ok_or("`--host-lib` is required for runtime-backed commands")?;
                let mut client = LoadedAegisClient::connect(host_lib, browser_config)?;
                client.enable_trace_recording(path);
            }
            TraceCommands::Replay { .. } => unreachable!("handled before host init"),
        },
        Commands::Events { since } => {
            let host_lib =
                host_lib.ok_or("`--host-lib` is required for runtime-backed commands")?;
            let client = LoadedAegisClient::connect(host_lib, browser_config)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&client.events_since(since))?
            );
        }
        Commands::Native { .. } => unreachable!("handled before runtime startup"),
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn maybe_reexec_runtime_command_from_bundle(
    cli: &Cli,
    workspace_root: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os("AEGIS_BUNDLED_CLI").is_some() || !command_requires_runtime(&cli.command) {
        return Ok(());
    }

    let current_exe = std::env::current_exe()?;
    if native::is_bundle_executable(&current_exe) {
        return Ok(());
    }

    let bundled_cli = native::prepare_bundled_cli(workspace_root, &current_exe)?;
    let mut command = ProcessCommand::new(&bundled_cli);
    command.current_dir(std::env::current_dir()?);
    command.env("AEGIS_BUNDLED_CLI", "1");
    command.args(std::env::args_os().skip(1));
    let error = command.exec();
    Err(Box::new(error))
}

#[cfg(not(target_os = "macos"))]
fn maybe_reexec_runtime_command_from_bundle(
    _cli: &Cli,
    _workspace_root: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

fn command_requires_runtime(command: &Commands) -> bool {
    match command {
        Commands::Serve { .. }
        | Commands::Navigate { .. }
        | Commands::Execute { .. }
        | Commands::SnapshotDom
        | Commands::Events { .. } => true,
        Commands::Session { .. } => true,
        Commands::Trace { command } => matches!(command, TraceCommands::Enable { .. }),
        Commands::Native { .. } => false,
    }
}

fn load_commands(
    file: Option<PathBuf>,
    json: Option<String>,
) -> Result<Vec<Command>, Box<dyn std::error::Error>> {
    match (file, json) {
        (Some(path), None) => Ok(serde_json::from_slice(&fs::read(path)?)?),
        (None, Some(json)) => Ok(serde_json::from_str(&json)?),
        _ => Err("provide exactly one of --file or --json".into()),
    }
}

fn handle_native_command(
    command: NativeCommands,
    workspace_root: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        NativeCommands::Status => {
            println!(
                "{}",
                serde_json::to_string_pretty(&native::status(workspace_root))?
            );
        }
        NativeCommands::Configure => {
            let project = configure_xcode(workspace_root)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "xcode_project": project,
                }))?
            );
        }
        NativeCommands::Build {
            configuration,
            scheme,
        } => {
            let configuration = match configuration {
                NativeConfigurationArg::Debug => NativeConfiguration::Debug,
                NativeConfigurationArg::Release => NativeConfiguration::Release,
            };
            let artifact = build_xcode(workspace_root, configuration, scheme.as_deref())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "configuration": configuration.as_str(),
                    "scheme": scheme.unwrap_or_else(|| "aegis_native".to_string()),
                    "artifact": artifact,
                }))?
            );
        }
        NativeCommands::Paths => {
            let status = native::status(workspace_root);
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "cef_sdk_root": status.cef_sdk_root,
                    "xcode_project": status.xcode_project,
                    "default_app_bundle": status.default_app_bundle,
                    "default_app_executable": status.default_app_executable,
                    "default_host_library": status.default_host_library,
                }))?
            );
        }
    }

    Ok(())
}
