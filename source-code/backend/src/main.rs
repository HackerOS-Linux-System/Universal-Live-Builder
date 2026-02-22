use std::error::Error;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use clap::{Parser, Subcommand};
use scopeguard::defer;
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;
use toml;
use tracing::{debug, error, info, instrument, Level};
use tracing_subscriber::{self, fmt, prelude::*, EnvFilter};

#[derive(Error, Debug)]
enum UlbError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("Command failed: {0}")]
    Command(String),
    #[error("Unsupported distro: {0}")]
    UnsupportedDistro(String),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    // Add more as needed
}

#[derive(Deserialize, Debug)]
struct Config {
    distro: String,
    image_name: String,
    installer: Option<String>,
    architecture: Option<String>, // For cross-compilation
    // More fields
}

#[derive(Subcommand, Debug)]
enum Commands {
    Build {
        #[clap(long)]
        release: bool,
        #[clap(long)]
        json_output: bool,
    },
    Clean,
}

#[derive(Parser, Debug)]
#[clap(name = "ulb-backend", version = "0.2.0")]
struct Args {
    #[clap(subcommand)]
    command: Commands,
    config_path: PathBuf,
}

fn main() -> Result<(), UlbError> {
    // Setup logging
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    let mut config_file = File::open(&args.config_path)?;
    let mut config_str = String::new();
    config_file.read_to_string(&mut config_str)?;
    let config: Config = toml::from_str(&config_str)?;

    match args.command {
        Commands::Build { release, json_output } => {
            let distro = create_distro_backend(&config)?;
            distro.build_iso(release, json_output)?;
        }
        Commands::Clean => clean_cache()?,
    }

    Ok(())
}

fn clean_cache() -> Result<(), UlbError> {
    let cache_dir = Path::new("build/.cache");
    if cache_dir.exists() {
        fs::remove_dir_all(cache_dir)?;
        info!("Cache cleaned");
    }
    Ok(())
}

// Trait for Distro-specific logic
trait DistroBackend {
    fn setup_container(&self, json_output: bool) -> Result<String, UlbError>; // Returns container_name
    fn install_packages(&self, container: &str) -> Result<(), UlbError>;
    fn remove_packages(&self, container: &str) -> Result<(), UlbError>;
    fn run_scripts(&self, container: &str) -> Result<(), UlbError>;
    fn build_rootfs(&self, container: &str) -> Result<(), UlbError>;
    fn copy_files(&self) -> Result<(), UlbError>;
    fn install_installer(&self, container: &str) -> Result<(), UlbError>;
    fn install_custom_packages(&self, container: &str) -> Result<(), UlbError>;
    fn create_iso(&self, container: &str, release: bool) -> Result<(), UlbError>;
    fn emit_progress(&self, stage: &str, progress: f32, json_output: bool) -> Result<(), UlbError>;
}

fn create_distro_backend(config: &Config) -> Result<Box<dyn DistroBackend>, UlbError> {
    match config.distro.as_str() {
        "fedora" => Ok(Box::new(FedoraBackend::new(config)?)),
        "debian" => Ok(Box::new(DebianBackend::new(config)?)),
        _ => Err(UlbError::UnsupportedDistro(config.distro.clone())),
    }
}

// Example for Fedora
struct FedoraBackend {
    config: Arc<Config>,
    base_dir: PathBuf,
    cache_dir: PathBuf,
    release_dir: PathBuf,
    container_image: String,
}

impl FedoraBackend {
    fn new(config: &Config) -> Result<Self, UlbError> {
        let base_dir = Path::new(".").canonicalize()?;
        let build_dir = base_dir.join("build");
        let cache_dir = build_dir.join(".cache");
        let release_dir = build_dir.join("release");
        fs::create_dir_all(&cache_dir)?;
        fs::create_dir_all(&release_dir)?;

        // For cross-compilation: If host is Debian, use Fedora container anyway, but adjust
        let arch = config.architecture.as_deref().unwrap_or("x86_64");
        let container_image = format!("fedora:latest-{}", arch); // Simplified

        Ok(Self {
            config: Arc::new(config.clone()),
            base_dir,
            cache_dir,
            release_dir,
            container_image,
        })
    }
}

impl DistroBackend for FedoraBackend {
    #[instrument]
    fn setup_container(&self, json_output: bool) -> Result<String, UlbError> {
        self.emit_progress("setup_container", 0.0, json_output)?;

        // Pull image
        let status = Command::new("podman").arg("pull").arg(&self.container_image).status()?;
        if !status.success() {
            return Err(UlbError::Command("Podman pull failed".to_string()));
        }

        let container_name = "ulb-fedora-builder".to_string();
        let mut create_cmd = Command::new("podman");
        create_cmd
            .arg("create")
            .arg("--name")
            .arg(&container_name)
            .arg("-v")
            .arg(format!("{}:/workspace", self.base_dir.display()))
            .arg("-v")
            .arg(format!("{}:/cache", self.cache_dir.display())) // Mount cache
            .arg(&self.container_image)
            .arg("sleep")
            .arg("infinity");

        let status = create_cmd.status()?;
        if !status.success() {
            return Err(UlbError::Command("Podman create failed".to_string()));
        }

        Command::new("podman").arg("start").arg(&container_name).status()?;

        // Cleanup on drop
        defer! {
            info!("Cleaning up container");
            let _ = Command::new("podman").arg("stop").arg(&container_name).status();
            let _ = Command::new("podman").arg("rm").arg(&container_name).status();
        }

        self.emit_progress("setup_container", 1.0, json_output)?;
        Ok(container_name)
    }

    fn install_packages(&self, container: &str) -> Result<(), UlbError> {
        // Read package-lists
        let package_lists = self.base_dir.join("package-lists");
        let mut packages = String::new();
        File::open(&package_lists)?.read_to_string(&mut packages)?;

        let install_cmd = format!("dnf --cacheonly --cachedir=/cache/dnf install -y {}", packages.trim());
        podman_exec(container, &[&install_cmd])?;

        // More stages with progress
        Ok(())
    }

    // Implement other methods similarly with logging, progress, cache usage
    // For example, build_rootfs uses dnf with --installroot and cache

    fn emit_progress(&self, stage: &str, progress: f32, json_output: bool) -> Result<(), UlbError> {
        if json_output {
            let msg = json!({
                "stage": stage,
                "progress": progress,
            });
            println!("{}", msg);
        } else {
            info!("Stage: {}, Progress: {}", stage, progress);
        }
        Ok(())
    }

    // Stub for others
    fn remove_packages(&self, _container: &str) -> Result<(), UlbError> { Ok(()) }
    fn run_scripts(&self, _container: &str) -> Result<(), UlbError> { Ok(()) }
    fn build_rootfs(&self, _container: &str) -> Result<(), UlbError> { Ok(()) }
    fn copy_files(&self) -> Result<(), UlbError> { Ok(()) }
    fn install_installer(&self, _container: &str) -> Result<(), UlbError> { Ok(()) }
    fn install_custom_packages(&self, _container: &str) -> Result<(), UlbError> { Ok(()) }
    fn create_iso(&self, _container: &str, _release: bool) -> Result<(), UlbError> { Ok(()) }
}

// Similarly implement DebianBackend

impl DistroBackend for DebianBackend {
    // Similar structure
}

struct DebianBackend {
    // Fields
}

impl DebianBackend {
    fn new(config: &Config) -> Result<Self, UlbError> {
        // Similar
        Ok(Self { /* ... */ })
    }
}

fn podman_exec(container: &str, cmds: &[&str]) -> Result<(), UlbError> {
    for cmd in cmds {
        let mut exec_cmd = Command::new("podman");
        exec_cmd
            .arg("exec")
            .arg("-it")
            .arg(container)
            .arg("bash")
            .arg("-c")
            .arg(cmd);
        let output = exec_cmd.output()?;
        if !output.status.success() {
            error!("Command failed: {} - stderr: {}", cmd, String::from_utf8_lossy(&output.stderr));
            return Err(UlbError::Command(format!("Command failed: {}", cmd)));
        }
        debug!("Command output: {}", String::from_utf8_lossy(&output.stdout));
    }
    Ok(())
}

// Other helper functions like copy_dir, podman_cp remain similar but with better error handling

// In build_iso method for trait impl:
fn build_iso(&self, release: bool, json_output: bool) -> Result<(), UlbError> {
    let container = self.setup_container(json_output)?;
    self.install_packages(&container)?;
    self.remove_packages(&container)?;
    self.run_scripts(&container)?;
    self.build_rootfs(&container)?;
    self.copy_files()?;
    self.install_installer(&container)?;
    self.install_custom_packages(&container)?;
    self.create_iso(&container, release)?;
    Ok(())
}
