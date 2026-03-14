use anyhow::{Context, Result, bail};
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use tar::Archive;
use zip::ZipArchive;

use crate::acp::install::state::{ManagedAgentState, ManagedInstalledVersion};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryInstallSpec {
    pub version: String,
    pub sha256: String,
    pub executable_path: PathBuf,
    pub archive_kind: Option<String>,
}

pub fn download_binary_artifact(url: &str) -> Result<Vec<u8>> {
    let response = ureq::get(url)
        .call()
        .map_err(|err| anyhow::anyhow!("failed to download binary artifact: {err}"))?;
    let mut reader = response.into_reader();
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .context("failed to read binary artifact response")?;
    Ok(bytes)
}

pub fn install_binary_from_file(
    artifact_path: &Path,
    installs_root: &Path,
    spec: &BinaryInstallSpec,
    state: &mut ManagedAgentState,
) -> Result<PathBuf> {
    validate_binary_spec(spec)?;

    let artifact_bytes = fs::read(artifact_path)
        .with_context(|| format!("failed to read binary artifact {}", artifact_path.display()))?;
    verify_sha256(&artifact_bytes, &spec.sha256)?;

    let version_root = installs_root.join(&spec.version);
    fs::create_dir_all(&version_root)
        .with_context(|| format!("failed to create install root {}", version_root.display()))?;

    match spec.archive_kind.as_deref() {
        None => {
            let target = version_root.join(&spec.executable_path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!(
                        "failed to create binary parent directory {}",
                        parent.display()
                    )
                })?;
            }
            fs::write(&target, &artifact_bytes)
                .with_context(|| format!("failed to write binary payload {}", target.display()))?;
        }
        Some("zip") => extract_zip(&artifact_bytes, &version_root)?,
        Some("tar") => extract_tar(Cursor::new(&artifact_bytes), &version_root)?,
        Some("tar.gz") => extract_tar_gz(&artifact_bytes, &version_root)?,
        Some(other) => bail!("unsupported archive kind '{other}'"),
    }

    let executable_path = version_root.join(&spec.executable_path);
    if !executable_path.exists() {
        bail!(
            "expected executable '{}' after install",
            executable_path.display()
        );
    }

    let resolved_command = executable_path.to_string_lossy().to_string();
    state.record_installed_version(ManagedInstalledVersion {
        version: spec.version.clone(),
        install_root: version_root.clone(),
        resolved_command: resolved_command.clone(),
        resolved_args: Vec::new(),
    });
    state.set_active_version(&spec.version);
    state.install_root = Some(version_root);
    state.resolved_command = Some(resolved_command);
    state.resolved_args.clear();

    Ok(executable_path)
}

fn validate_binary_spec(spec: &BinaryInstallSpec) -> Result<()> {
    if spec.sha256.trim().is_empty() {
        bail!("binary install metadata requires sha256");
    }
    if spec.executable_path.as_os_str().is_empty() {
        bail!("binary install metadata requires executable_path");
    }
    if let Some(kind) = spec.archive_kind.as_deref() {
        match kind {
            "zip" | "tar" | "tar.gz" => {}
            other => bail!("unsupported archive kind '{other}'"),
        }
    }
    Ok(())
}

fn verify_sha256(bytes: &[u8], expected_sha256: &str) -> Result<()> {
    let actual = format!("{:x}", Sha256::digest(bytes));
    if !actual.eq_ignore_ascii_case(expected_sha256) {
        bail!("binary checksum mismatch");
    }
    Ok(())
}

fn extract_zip(bytes: &[u8], output_dir: &Path) -> Result<()> {
    let cursor = Cursor::new(bytes);
    let mut archive = ZipArchive::new(cursor).context("failed to open zip archive")?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .with_context(|| format!("failed to open zip entry {index}"))?;
        let Some(path) = entry.enclosed_name().map(|path| output_dir.join(path)) else {
            continue;
        };
        if entry.name().ends_with('/') {
            fs::create_dir_all(&path)
                .with_context(|| format!("failed to create zip directory {}", path.display()))?;
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create extracted parent directory {}",
                    parent.display()
                )
            })?;
        }
        let mut output = fs::File::create(&path)
            .with_context(|| format!("failed to create extracted file {}", path.display()))?;
        std::io::copy(&mut entry, &mut output)
            .with_context(|| format!("failed to extract zip entry {}", path.display()))?;
    }
    Ok(())
}

fn extract_tar<R: Read>(reader: R, output_dir: &Path) -> Result<()> {
    let mut archive = Archive::new(reader);
    archive
        .unpack(output_dir)
        .with_context(|| format!("failed to unpack tar archive into {}", output_dir.display()))
}

fn extract_tar_gz(bytes: &[u8], output_dir: &Path) -> Result<()> {
    let decoder = GzDecoder::new(Cursor::new(bytes));
    extract_tar(decoder, output_dir)
}
