use anyhow::{Context, Result, anyhow};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchCommand {
    pub command: String,
    pub args: Vec<String>,
}

pub fn build_npx_launch(package: &str, version: &str) -> LaunchCommand {
    choose_installed_npx_launch(&format!("{package}@{version}"), &[])
        .unwrap_or_else(|_| npx_launch_candidates(&format!("{package}@{version}"), &[])[0].clone())
}

pub fn build_npx_package_launch(package_spec: &str, extra_args: &[String]) -> LaunchCommand {
    choose_installed_npx_launch(package_spec, extra_args)
        .unwrap_or_else(|_| npx_launch_candidates(package_spec, extra_args)[0].clone())
}

pub fn choose_installed_npx_launch(
    package_spec: &str,
    extra_args: &[String],
) -> Result<LaunchCommand> {
    choose_npx_launch(package_spec, extra_args, |candidate| {
        Ok(launch_command_exists(&candidate.command))
    })
}

pub fn choose_npx_launch<F>(
    package_spec: &str,
    extra_args: &[String],
    mut verify: F,
) -> Result<LaunchCommand>
where
    F: FnMut(&LaunchCommand) -> Result<bool>,
{
    let candidates = npx_launch_candidates(package_spec, extra_args);
    let mut last_err = None;
    for candidate in candidates {
        match verify(&candidate) {
            Ok(true) => return Ok(candidate),
            Ok(false) => {}
            Err(err) => last_err = Some(err),
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow!("no working npx launcher candidate found")))
}

pub fn npx_launch_candidates(package_spec: &str, extra_args: &[String]) -> Vec<LaunchCommand> {
    if cfg!(windows) {
        let executable = infer_package_executable(package_spec);
        let npm_exec_args = {
            let mut args = vec![
                "exec".to_string(),
                "--yes".to_string(),
                "--package".to_string(),
                package_spec.to_string(),
                executable,
            ];
            if !extra_args.is_empty() {
                args.push("--".to_string());
                args.extend(extra_args.iter().cloned());
            }
            args
        };

        vec![
            LaunchCommand {
                command: "npm.cmd".into(),
                args: npm_exec_args,
            },
            LaunchCommand {
                command: "npx.cmd".into(),
                args: std::iter::once("-y".to_string())
                    .chain(std::iter::once(package_spec.to_string()))
                    .chain(extra_args.iter().cloned())
                    .collect(),
            },
            LaunchCommand {
                command: "npx".into(),
                args: std::iter::once("-y".to_string())
                    .chain(std::iter::once(package_spec.to_string()))
                    .chain(extra_args.iter().cloned())
                    .collect(),
            },
        ]
    } else {
        vec![LaunchCommand {
            command: "npx".into(),
            args: std::iter::once("-y".to_string())
                .chain(std::iter::once(package_spec.to_string()))
                .chain(extra_args.iter().cloned())
                .collect(),
        }]
    }
}

pub fn build_uvx_launch(package: &str, version: &str) -> LaunchCommand {
    build_uvx_package_launch(&format!("{package}=={version}"), &[])
}

pub fn build_uvx_package_launch(package_spec: &str, extra_args: &[String]) -> LaunchCommand {
    LaunchCommand {
        command: if cfg!(windows) {
            "uvx.exe".into()
        } else {
            "uvx".into()
        },
        args: std::iter::once(package_spec.to_string())
            .chain(extra_args.iter().cloned())
            .collect(),
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

fn infer_package_executable(package_spec: &str) -> String {
    let without_version = if package_spec.starts_with('@') {
        if package_spec.matches('@').count() >= 2 {
            package_spec
                .rsplit_once('@')
                .map(|(name, _)| name)
                .unwrap_or(package_spec)
                .to_string()
        } else {
            package_spec.to_string()
        }
    } else {
        package_spec
            .rsplit_once('@')
            .map(|(name, _)| name.to_string())
            .unwrap_or_else(|| package_spec.to_string())
    };

    without_version
        .rsplit('/')
        .next()
        .unwrap_or(&without_version)
        .to_string()
}

pub fn launch_command_exists(command: &str) -> bool {
    let path = Path::new(command);
    if path.is_absolute() || command.contains(['\\', '/']) {
        return path.is_file();
    }

    let probe = if cfg!(windows) { "where" } else { "which" };
    Command::new(probe)
        .arg(command)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::build_npx_package_launch;

    #[test]
    fn npx_windows_fallback_uses_npm_exec() {
        let launch = build_npx_package_launch("@zed-industries/codex-acp@0.10.0", &[]);
        if cfg!(windows) {
            assert_eq!(launch.command, "npm.cmd");
            assert_eq!(
                launch.args,
                vec![
                    "exec",
                    "--yes",
                    "--package",
                    "@zed-industries/codex-acp@0.10.0",
                    "codex-acp"
                ]
            );
        } else {
            assert_eq!(launch.command, "npx");
            assert_eq!(launch.args, vec!["-y", "@zed-industries/codex-acp@0.10.0"]);
        }
    }
}
