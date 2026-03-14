use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchCommand {
    pub command: String,
    pub args: Vec<String>,
}

pub fn build_npx_launch(package: &str, version: &str) -> LaunchCommand {
    LaunchCommand {
        command: if cfg!(windows) {
            "npx.cmd".into()
        } else {
            "npx".into()
        },
        args: vec!["-y".into(), format!("{package}@{version}")],
    }
}

pub fn build_uvx_launch(package: &str, version: &str) -> LaunchCommand {
    LaunchCommand {
        command: if cfg!(windows) {
            "uvx.exe".into()
        } else {
            "uvx".into()
        },
        args: vec![format!("{package}=={version}")],
    }
}

pub fn write_launch_wrapper(
    install_root: &Path,
    name: &str,
    launch: &LaunchCommand,
) -> Result<PathBuf> {
    fs::create_dir_all(install_root).with_context(|| {
        format!(
            "failed to create install root for managed wrapper {}",
            install_root.display()
        )
    })?;
    let wrapper_path = install_root.join(format!("{name}.{}", wrapper_extension()));
    fs::write(&wrapper_path, render_wrapper(launch))
        .with_context(|| format!("failed to write launch wrapper {}", wrapper_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(&wrapper_path)
            .with_context(|| format!("failed to stat wrapper {}", wrapper_path.display()))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&wrapper_path, permissions).with_context(|| {
            format!(
                "failed to update launch wrapper permissions {}",
                wrapper_path.display()
            )
        })?;
    }

    Ok(wrapper_path)
}

pub fn remove_launch_wrapper(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_file(path)
            .with_context(|| format!("failed to remove launch wrapper {}", path.display()))?;
    }
    Ok(())
}

fn wrapper_extension() -> &'static str {
    if cfg!(windows) { "cmd" } else { "sh" }
}

fn render_wrapper(launch: &LaunchCommand) -> String {
    let rendered_args = launch
        .args
        .iter()
        .map(|arg| quote_arg(arg))
        .collect::<Vec<_>>()
        .join(" ");

    if cfg!(windows) {
        if rendered_args.is_empty() {
            format!("@echo off\r\n\"{}\" %*\r\n", launch.command)
        } else {
            format!(
                "@echo off\r\n\"{}\" {} %*\r\n",
                launch.command, rendered_args
            )
        }
    } else if rendered_args.is_empty() {
        format!("#!/usr/bin/env sh\nexec \"{}\" \"$@\"\n", launch.command)
    } else {
        format!(
            "#!/usr/bin/env sh\nexec \"{}\" {} \"$@\"\n",
            launch.command, rendered_args
        )
    }
}

fn quote_arg(arg: &str) -> String {
    if arg.contains([' ', '\t', '"']) {
        format!("\"{}\"", arg.replace('"', "\\\""))
    } else {
        arg.to_string()
    }
}
