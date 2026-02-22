// main.rs
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use clap::{Parser, Subcommand};
use scopeguard::defer;
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;
use toml;
use tracing::{debug, error, info, instrument};
use tracing_subscriber::{self, fmt, prelude::*, EnvFilter};

#[derive(Error, Debug)]
enum UlbError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("Command failed in {stage}: {message}")]
    Command { stage: String, message: String },
    #[error("Unsupported distro: {0}")]
    UnsupportedDistro(String),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Validation error: {0}")]
    Validation(String),
    // Add more as needed
}

#[derive(Deserialize, Debug, Clone)]
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
    Status,
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
    let config_dir = args.config_path.parent().unwrap_or(Path::new("."));
    let mut config_file = File::open(&args.config_path)?;
    let mut config_str = String::new();
    config_file.read_to_string(&mut config_str)?;
    let config: Config = toml::from_str(&config_str)?;
    validate_config(&config, config_dir)?;
    match args.command {
        Commands::Build { release, json_output } => {
            let distro = create_distro_backend(&config)?;
            distro.build_iso(release, json_output)?;
        }
        Commands::Clean => clean_cache()?,
        Commands::Status => status(&config, &args.config_path)?,
    }
    Ok(())
}

fn validate_config(config: &Config, config_dir: &Path) -> Result<(), UlbError> {
    if !["fedora", "debian"].contains(&config.distro.as_str()) {
        return Err(UlbError::Validation(format!("Unsupported distro: {}", config.distro)));
    }
    if config.image_name.is_empty() {
        return Err(UlbError::Validation("image_name cannot be empty".to_string()));
    }
    let package_list_path = config_dir.join("package-lists");
    if !package_list_path.exists() || package_list_path.metadata()?.len() == 0 {
        return Err(UlbError::Validation("package-lists file is missing or empty".to_string()));
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

fn status(config: &Config, config_path: &PathBuf) -> Result<(), UlbError> {
    println!("ULB Backend Version: 0.2.0");
    println!("Config Path: {}", config_path.display());
    println!("Distro: {}", config.distro);
    println!("Image Name: {}", config.image_name);
    if let Some(installer) = &config.installer {
        println!("Installer: {}", installer);
    }
    if let Some(arch) = &config.architecture {
        println!("Architecture: {}", arch);
    }
    let podman_status = Command::new("podman").arg("--version").status();
    match podman_status {
        Ok(status) if status.success() => println!("Podman is available."),
        _ => println!("Warning: Podman is not available or not in PATH."),
    }
    Ok(())
}

// Trait for Distro-specific logic
trait DistroBackend {
    fn base(&self) -> &BaseBackend;
    fn install_packages(&self, container: &str, json_output: bool) -> Result<(), UlbError>;
    fn remove_packages(&self, container: &str, json_output: bool) -> Result<(), UlbError>;
    fn build_rootfs(&self, container: &str, json_output: bool) -> Result<(), UlbError>;
    fn install_installer(&self, container: &str, json_output: bool) -> Result<(), UlbError>;
    fn install_custom_packages(&self, container: &str, json_output: bool) -> Result<(), UlbError>;
    fn create_iso(&self, container: &str, release: bool, json_output: bool) -> Result<(), UlbError>;
}

// Base struct for common fields and methods
#[derive(Debug)]
struct BaseBackend {
    config: Arc<Config>,
    base_dir: PathBuf,
    cache_dir: PathBuf,
    release_dir: PathBuf,
    container_image: String,
    container_name: String,
}

impl BaseBackend {
    fn new(config: &Config, distro: &str, default_arch: &str, image_prefix: &str) -> Result<Self, UlbError> {
        let base_dir = Path::new(".").canonicalize()?;
        let build_dir = base_dir.join("build");
        let cache_dir = build_dir.join(".cache");
        let release_dir = build_dir.join("release");
        fs::create_dir_all(&cache_dir)?;
        fs::create_dir_all(&release_dir)?;
        let arch = config.architecture.as_deref().unwrap_or(default_arch);
        let container_image = format!("{}:latest-{}", image_prefix, arch);
        let container_name = format!("ulb-{}-builder", distro);
        Ok(Self {
            config: Arc::new(config.clone()),
            base_dir,
            cache_dir,
            release_dir,
            container_image,
            container_name,
        })
    }

    #[instrument]
    fn setup_container(&self, json_output: bool) -> Result<String, UlbError> {
        self.emit_progress("setup_container", 0.0, json_output)?;
        let status = Command::new("podman").arg("pull").arg(&self.container_image).status()?;
        if !status.success() {
            return Err(UlbError::Command { stage: "setup_container".to_string(), message: "Podman pull failed".to_string() });
        }
        let mut create_cmd = Command::new("podman");
        create_cmd
            .arg("create")
            .arg("--name")
            .arg(&self.container_name)
            .arg("-v")
            .arg(format!("{}:/workspace", self.base_dir.display()))
            .arg("-v")
            .arg(format!("{}:/cache", self.cache_dir.display()))
            .arg(&self.container_image)
            .arg("sleep")
            .arg("infinity");
        let status = create_cmd.status()?;
        if !status.success() {
            return Err(UlbError::Command { stage: "setup_container".to_string(), message: "Podman create failed".to_string() });
        }
        Command::new("podman").arg("start").arg(&self.container_name).status()?;
        self.emit_progress("setup_container", 1.0, json_output)?;
        Ok(self.container_name.clone())
    }

    fn run_scripts(&self, container: &str, json_output: bool) -> Result<(), UlbError> {
        self.emit_progress("run_scripts", 0.0, json_output)?;
        let scripts_dir = self.base_dir.join("scripts");
        if scripts_dir.exists() {
            let mut entries: Vec<_> = fs::read_dir(&scripts_dir)?.collect::<Result<_, _>>()?;
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                if entry.path().extension().map_or(false, |e| e == "sh") {
                    let script_path = entry.path();
                    let script_name = script_path.file_name().unwrap().to_str().unwrap();
                    podman_cp(&script_path, container, &format!("/tmp/{}", script_name))?;
                    let run_cmd = format!("bash /tmp/{} && rm /tmp/{}", script_name, script_name);
                    podman_exec(container, &[&run_cmd], "run_scripts")?;
                }
            }
        }
        self.emit_progress("run_scripts", 1.0, json_output)?;
        Ok(())
    }

    fn copy_files(&self, container: &str, json_output: bool) -> Result<(), UlbError> {
        self.emit_progress("copy_files", 0.0, json_output)?;
        let files_dir = self.base_dir.join("files");
        if files_dir.exists() {
            let dest = "/workspace/build/rootfs";
            let copy_cmd = format!("cp -r /workspace/files/* {}", dest);
            podman_exec(container, &[&copy_cmd], "copy_files")?;
        }
        let install_files_dir = self.base_dir.join("install-files");
        if install_files_dir.exists() {
            let install_dest = "/workspace/build/rootfs/opt/install-files"; // Example dest
            podman_exec(container, &[&format!("mkdir -p {}", install_dest)], "copy_files")?;
            let copy_install_cmd = format!("cp -r /workspace/install-files/* {}", install_dest);
            podman_exec(container, &[&copy_install_cmd], "copy_files")?;
        }
        self.emit_progress("copy_files", 1.0, json_output)?;
        Ok(())
    }

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

    fn cleanup_container(&self, container: &str) -> Result<(), UlbError> {
        info!("Cleaning up container");
        let _ = Command::new("podman").arg("stop").arg(container).status();
        let _ = Command::new("podman").arg("rm").arg(container).status();
        Ok(())
    }

    fn build_iso_pipeline(&self, backend: &dyn DistroBackend, release: bool, json_output: bool) -> Result<(), UlbError> {
        let container = self.setup_container(json_output)?;
        defer! {
            let _ = self.cleanup_container(&container);
        }
        backend.install_packages(&container, json_output)?;
        backend.remove_packages(&container, json_output)?;
        self.run_scripts(&container, json_output)?;
        backend.build_rootfs(&container, json_output)?;
        self.copy_files(&container, json_output)?;
        backend.install_installer(&container, json_output)?;
        backend.install_custom_packages(&container, json_output)?;
        backend.create_iso(&container, release, json_output)?;
        Ok(())
    }
}

// Fedora
struct FedoraBackend {
    base: BaseBackend,
}

impl FedoraBackend {
    fn new(config: &Config) -> Result<Self, UlbError> {
        let base = BaseBackend::new(config, "fedora", "x86_64", "fedora")?;
        Ok(Self { base })
    }
}

impl DistroBackend for FedoraBackend {
    fn base(&self) -> &BaseBackend {
        &self.base
    }

    fn install_packages(&self, container: &str, json_output: bool) -> Result<(), UlbError> {
        self.base.emit_progress("install_packages", 0.0, json_output)?;
        let make_cache_cmd = "dnf makecache --cachedir=/cache/dnf";
        podman_exec(container, &[make_cache_cmd], "install_packages")?;
        let package_list_path = self.base.base_dir.join("package-lists");
        let mut packages = String::new();
        File::open(&package_list_path)?.read_to_string(&mut packages)?;
        let packages = packages.lines().collect::<Vec<_>>().join(" ");
        let install_cmd = format!("dnf --cachedir=/cache/dnf install -y {}", packages.trim());
        podman_exec(container, &[&install_cmd], "install_packages")?;
        self.base.emit_progress("install_packages", 1.0, json_output)?;
        Ok(())
    }

    fn remove_packages(&self, container: &str, json_output: bool) -> Result<(), UlbError> {
        self.base.emit_progress("remove_packages", 0.0, json_output)?;
        let remove_list_path = self.base.base_dir.join("packages-lists-remove");
        if remove_list_path.exists() {
            let mut packages = String::new();
            File::open(&remove_list_path)?.read_to_string(&mut packages)?;
            let packages = packages.lines().collect::<Vec<_>>().join(" ");
            let remove_cmd = format!("dnf remove -y {}", packages.trim());
            podman_exec(container, &[&remove_cmd], "remove_packages")?;
        }
        self.base.emit_progress("remove_packages", 1.0, json_output)?;
        Ok(())
    }

    fn build_rootfs(&self, container: &str, json_output: bool) -> Result<(), UlbError> {
        self.base.emit_progress("build_rootfs", 0.0, json_output)?;
        let rootfs_dir = "/workspace/build/rootfs";
        fs::create_dir_all(self.base.base_dir.join("build/rootfs"))?;
        let build_cmd = format!("dnf install --installroot {} --releasever=latest -y @core", rootfs_dir); // Example
        podman_exec(container, &[&build_cmd], "build_rootfs")?;
        self.base.emit_progress("build_rootfs", 1.0, json_output)?;
        Ok(())
    }

    fn install_installer(&self, container: &str, json_output: bool) -> Result<(), UlbError> {
        self.base.emit_progress("install_installer", 0.0, json_output)?;
        if let Some(installer) = &self.base.config.installer {
            let install_cmd = format!("dnf install -y {}", installer);
            podman_exec(container, &[&install_cmd], "install_installer")?;
        }
        self.base.emit_progress("install_installer", 1.0, json_output)?;
        Ok(())
    }

    fn install_custom_packages(&self, container: &str, json_output: bool) -> Result<(), UlbError> {
        self.base.emit_progress("install_custom_packages", 0.0, json_output)?;
        let repos_dir = self.base.base_dir.join("repos");
        if repos_dir.exists() {
            let copy_cmd = "cp /workspace/repos/* /etc/yum.repos.d/";
            podman_exec(container, &[copy_cmd], "install_custom_packages")?;
            let update_cmd = "dnf update -y";
            podman_exec(container, &[update_cmd], "install_custom_packages")?;
        }
        self.base.emit_progress("install_custom_packages", 1.0, json_output)?;
        Ok(())
    }

    fn create_iso(&self, container: &str, release: bool, json_output: bool) -> Result<(), UlbError> {
        self.base.emit_progress("create_iso", 0.0, json_output)?;
        // Use lorax for Fedora live ISO
        let iso_name = if release { "release.iso" } else { "debug.iso" };
        let lorax_cmd = format!("lorax -p {} -v latest -r latest --rootfs-size=3 --buildarch={} -s http://download.fedoraproject.org/pub/fedora/linux/releases/latest/Everything/{}/os/ --isfinal={} /workspace/build/release/{}", self.base.config.image_name, self.base.config.architecture.as_deref().unwrap_or("x86_64"), self.base.config.architecture.as_deref().unwrap_or("x86_64"), release, iso_name);
        podman_exec(container, &[&lorax_cmd], "create_iso")?;
        self.base.emit_progress("create_iso", 1.0, json_output)?;
        Ok(())
    }
}

// Debian
struct DebianBackend {
    base: BaseBackend,
}

impl DebianBackend {
    fn new(config: &Config) -> Result<Self, UlbError> {
        let base = BaseBackend::new(config, "debian", "amd64", "debian")?;
        Ok(Self { base })
    }
}

impl DistroBackend for DebianBackend {
    fn base(&self) -> &BaseBackend {
        &self.base
    }

    fn install_packages(&self, container: &str, json_output: bool) -> Result<(), UlbError> {
        self.base.emit_progress("install_packages", 0.0, json_output)?;
        let package_list_path = self.base.base_dir.join("package-lists");
        let mut packages = String::new();
        File::open(&package_list_path)?.read_to_string(&mut packages)?;
        let packages = packages.lines().collect::<Vec<_>>().join(" ");
        let update_cmd = "apt update";
        let install_cmd = format!("DEBIAN_FRONTEND=noninteractive apt install -y {}", packages.trim());
        podman_exec(container, &[update_cmd, &install_cmd], "install_packages")?;
        self.base.emit_progress("install_packages", 1.0, json_output)?;
        Ok(())
    }

    fn remove_packages(&self, container: &str, json_output: bool) -> Result<(), UlbError> {
        self.base.emit_progress("remove_packages", 0.0, json_output)?;
        let remove_list_path = self.base.base_dir.join("packages-lists-remove");
        if remove_list_path.exists() {
            let mut packages = String::new();
            File::open(&remove_list_path)?.read_to_string(&mut packages)?;
            let packages = packages.lines().collect::<Vec<_>>().join(" ");
            let remove_cmd = format!("DEBIAN_FRONTEND=noninteractive apt remove -y {}", packages.trim());
            podman_exec(container, &[&remove_cmd], "remove_packages")?;
        }
        self.base.emit_progress("remove_packages", 1.0, json_output)?;
        Ok(())
    }

    fn build_rootfs(&self, container: &str, json_output: bool) -> Result<(), UlbError> {
        self.base.emit_progress("build_rootfs", 0.0, json_output)?;
        let rootfs_dir = "/workspace/build/rootfs";
        fs::create_dir_all(self.base.base_dir.join("build/rootfs"))?;
        let arch = self.base.config.architecture.as_deref().unwrap_or("amd64");
        let build_cmd = format!("debootstrap --arch={} stable {} http://deb.debian.org/debian", arch, rootfs_dir);
        podman_exec(container, &[&build_cmd], "build_rootfs")?;
        self.base.emit_progress("build_rootfs", 1.0, json_output)?;
        Ok(())
    }

    fn install_installer(&self, container: &str, json_output: bool) -> Result<(), UlbError> {
        self.base.emit_progress("install_installer", 0.0, json_output)?;
        if let Some(installer) = &self.base.config.installer {
            let install_cmd = format!("DEBIAN_FRONTEND=noninteractive apt install -y {}", installer);
            podman_exec(container, &[&install_cmd], "install_installer")?;
        }
        self.base.emit_progress("install_installer", 1.0, json_output)?;
        Ok(())
    }

    fn install_custom_packages(&self, container: &str, json_output: bool) -> Result<(), UlbError> {
        self.base.emit_progress("install_custom_packages", 0.0, json_output)?;
        let repos_dir = self.base.base_dir.join("repos");
        if repos_dir.exists() {
            let copy_cmd = "cp /workspace/repos/* /etc/apt/sources.list.d/";
            podman_exec(container, &[copy_cmd], "install_custom_packages")?;
            let update_cmd = "apt update";
            podman_exec(container, &[update_cmd], "install_custom_packages")?;
        }
        self.base.emit_progress("install_custom_packages", 1.0, json_output)?;
        Ok(())
    }

    fn create_iso(&self, container: &str, release: bool, json_output: bool) -> Result<(), UlbError> {
        self.base.emit_progress("create_iso", 0.0, json_output)?;
        let iso_name = if release { "release.iso" } else { "debug.iso" };
        let create_cmd = format!("xorriso -as mkisofs -o /workspace/build/release/{} /workspace/build/rootfs", iso_name);
        podman_exec(container, &[&create_cmd], "create_iso")?;
        self.base.emit_progress("create_iso", 1.0, json_output)?;
        Ok(())
    }
}

fn create_distro_backend(config: &Config) -> Result<Box<dyn DistroBackend>, UlbError> {
    match config.distro.as_str() {
        "fedora" => Ok(Box::new(FedoraBackend::new(config)?)),
        "debian" => Ok(Box::new(DebianBackend::new(config)?)),
        _ => Err(UlbError::UnsupportedDistro(config.distro.clone())),
    }
}

impl dyn DistroBackend {
    fn build_iso(&self, release: bool, json_output: bool) -> Result<(), UlbError> {
        self.base().build_iso_pipeline(self, release, json_output)
    }
}

fn podman_exec(container: &str, cmds: &[&str], stage: &str) -> Result<(), UlbError> {
    for cmd in cmds {
        let mut exec_cmd = Command::new("podman");
        exec_cmd
            .arg("exec")
            .arg(container)
            .arg("bash")
            .arg("-c")
            .arg(cmd);
        let output = exec_cmd.output()?;
        if !output.status.success() {
            error!("Command failed in {}: {} - stderr: {}", stage, cmd, String::from_utf8_lossy(&output.stderr));
            return Err(UlbError::Command { stage: stage.to_string(), message: format!("Command failed: {}", cmd) });
        }
        debug!("Command output in {}: {}", stage, String::from_utf8_lossy(&output.stdout));
    }
    Ok(())
}

fn podman_cp(src: &Path, container: &str, dest: &str) -> Result<(), UlbError> {
    let src_str = src.to_str().unwrap();
    let cp_cmd = Command::new("podman")
        .arg("cp")
        .arg(src_str)
        .arg(format!("{}:{}", container, dest))
        .status()?;
    if !cp_cmd.success() {
        return Err(UlbError::Command { stage: "podman_cp".to_string(), message: "Podman cp failed".to_string() });
    }
    Ok(())
}

// Tests
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_parse() {
        let config_str = r#"
distro = "fedora"
image_name = "test"
"#;
        let config: Config = toml::from_str(config_str).unwrap();
        assert_eq!(config.distro, "fedora");
    }

    #[test]
    fn test_validate_config() {
        let config = Config {
            distro: "invalid".to_string(),
            image_name: "test".to_string(),
            installer: None,
            architecture: None,
        };
        assert!(validate_config(&config, Path::new(".")).is_err());
    }

    // More tests...
}
