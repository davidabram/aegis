#[cfg(target_os = "macos")]
use std::os::unix::process::CommandExt;
use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;

use aegis::api::server;
use aegis::{
    BrowserConfig, BrowserMode, NativeConfiguration, build_xcode, configure_xcode, native,
    replay_trace,
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
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Clone, clap::ValueEnum)]
enum BrowserModeArg {
    Headless,
    Headful,
}

#[derive(Clone, Subcommand)]
enum Commands {
    Serve {
        #[arg(long, default_value = "127.0.0.1:7878")]
        addr: SocketAddr,
    },
    Trace {
        #[command(subcommand)]
        command: TraceCommands,
    },
    Native {
        #[command(subcommand)]
        command: NativeCommands,
    },
}

#[derive(Clone, Subcommand)]
enum TraceCommands {
    Replay { path: PathBuf },
}

#[derive(Clone, Subcommand)]
enum NativeCommands {
    Status,
    Configure,
    Build {
        #[arg(long, value_enum, default_value = "release")]
        configuration: NativeConfigurationArg,
        #[arg(long)]
        scheme: Option<String>,
    },
    Install,
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
    let workspace_root = resolve_workspace_root()?;
    maybe_reexec_runtime_command_from_bundle(&cli, &workspace_root)?;
    let command = resolved_command(&cli);
    let browser_config = BrowserConfig {
        mode: match effective_mode(&cli) {
            BrowserModeArg::Headless => BrowserMode::Headless,
            BrowserModeArg::Headful => BrowserMode::Headful,
        },
        start_url: cli.start_url.clone(),
    };

    match &command {
        Commands::Trace {
            command: TraceCommands::Replay { path },
        } => {
            let state = replay_trace(path.clone())?;
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
            handle_native_command(command.clone(), &workspace_root)?;
            return Ok(());
        }
        _ => {}
    }

    match command {
        Commands::Serve { addr } => {
            let host_lib = cli
                .host_lib
                .clone()
                .unwrap_or_else(|| native::status(&workspace_root).default_host_library);
            if !host_lib.exists() {
                return Err(format!(
                    "host library not found at {}. Run `aegis native build --configuration release --scheme aegis_host` first or pass --host-lib.",
                    host_lib.display()
                )
                .into());
            }
            server::serve(addr, host_lib, browser_config).await?;
        }
        Commands::Trace { command } => match command {
            TraceCommands::Replay { .. } => unreachable!("handled before host init"),
        },
        Commands::Native { .. } => unreachable!("handled before runtime startup"),
    }

    Ok(())
}

fn resolve_workspace_root() -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(root) = std::env::var_os("AEGIS_WORKSPACE_ROOT") {
        return Ok(PathBuf::from(root));
    }
    Ok(std::env::current_dir()?)
}

fn resolved_command(cli: &Cli) -> Commands {
    cli.command.clone().unwrap_or(Commands::Serve {
        addr: SocketAddr::from(([127, 0, 0, 1], 7878)),
    })
}

fn effective_mode(cli: &Cli) -> BrowserModeArg {
    if cli.command.is_none() && !mode_flag_was_set() {
        return BrowserModeArg::Headful;
    }
    cli.mode.clone()
}

fn mode_flag_was_set() -> bool {
    std::env::args_os().any(|arg| arg == "--mode")
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

fn command_requires_runtime(command: &Option<Commands>) -> bool {
    match command {
        None => true,
        Some(Commands::Serve { .. }) => true,
        Some(_) => false,
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
        NativeCommands::Install => {
            #[cfg(target_os = "macos")]
            {
                let current_exe = std::env::current_exe()?;
                let bundle = native::install_local_release(workspace_root, &current_exe)?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "installed_app_bundle": bundle,
                        "installed_app_executable": native::bundle_executable(&bundle),
                    }))?
                );
            }
            #[cfg(not(target_os = "macos"))]
            {
                return Err("`aegis native install` is only supported on macOS".into());
            }
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
