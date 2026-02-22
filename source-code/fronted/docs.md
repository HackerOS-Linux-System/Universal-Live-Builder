# Universal Live Builder Documentation

## Introduction
ULB is a tool for building custom live ISOs for various distributions like Fedora and Debian. It leverages Podman for containerized builds to ensure reproducibility and isolation.

## Project Structure
- **Config.toml**: Configuration file for distro, image name, etc.
- **package-lists**: File listing packages to install (one per line).
- **packages-lists-remove**: File listing packages to remove (one per line).
- **scripts/**: Directory for custom shell scripts to run during build (sorted by name).
- **files/**: Files to copy into the rootfs.
- **install-files/**: Files to copy into a special install directory in rootfs.
- **repos/**: Custom repository files.
- **build/.cache**: Cache directory for downloads.
- **build/release**: Output directory for ISO.

## Usage
- `ulb init`: Initialize project with directories and example files.
- `ulb build --release`: Build release ISO (use --json-output for progress if needed internally).
- `ulb clean`: Clean cache.
- `ulb docs`: View this documentation in TUI.
- `ulb update`: Update backend and tool.
- `ulb status`: Show configuration and backend status.

## Configuration
Edit Config.toml to set distro (fedora/debian), image_name, optional installer and architecture.

## Extending
Add scripts in scripts/ for custom configuration. Scripts are executed in alphabetical order.

## Troubleshooting
- Ensure Podman is installed and running.
- Check logs for errors during build.
- Use status command to verify setup.

For more details, see the source code or contribute on GitHub.
