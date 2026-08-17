#![allow(dead_code)]
// ponytail: pkexec spawn stub — awaits wiring into launch flow
use std::{ffi::OsStr, io::Error, path::Path};

use crate::{PandoraChild, PandoraCommand, spawner::SpawnContext};

fn polkit_supports_keep_cwd() -> Option<bool> {
    let output = std::process::Command::new("pkaction").arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout);
    // Expect "pkaction version 124" or similar; parse last integer
    let ver = s.split_whitespace().last()?.parse::<u32>().ok()?;
    Some(ver >= 123)
}

pub fn spawn(mut cmd: PandoraCommand, context: &mut SpawnContext) -> std::io::Result<PandoraChild> {
    let Some(pkexec) = crate::path_cache::get_command_path_cached(OsStr::new("pkexec")) else {
        return Err(Error::new(std::io::ErrorKind::NotFound, "cannot find 'pkexec'"));
    };

    let mut executable = std::mem::replace(&mut cmd.executable, pkexec.as_os_str().to_os_string().into());

    // Resolve every non-absolute executable path before insertion into cmd.
    if !Path::new(&executable.0).is_absolute() {
        if executable.0.as_encoded_bytes().contains(&b'/') {
            // Relative path with separator: canonicalize relative to current_dir if set, else cwd
            let base = cmd.current_dir.as_deref().unwrap_or(Path::new("."));
            let candidate = base.join(Path::new(&executable.0));
            if let Ok(canonical) = candidate.canonicalize() {
                executable = canonical.into_os_string().into();
            } else if let Ok(canonical) = Path::new(&executable.0).canonicalize() {
                executable = canonical.into_os_string().into();
            } else {
                return Err(Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("cannot find '{}'", executable.0.to_string_lossy()),
                ));
            }
        } else {
            let Some(path) = crate::path_cache::get_command_path_cached(&executable.0) else {
                return Err(Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("cannot find '{}'", executable.0.to_string_lossy()),
                ));
            };
            executable = path.as_os_str().to_os_string().into();
        }
    }

    // Gate --keep-cwd on polkit 123+
    let supports_keep_cwd = polkit_supports_keep_cwd().unwrap_or(true);
    if !supports_keep_cwd {
        return Err(Error::new(
            std::io::ErrorKind::Unsupported,
            "pkexec --keep-cwd requires polkit >= 123; please update polkit",
        ));
    }

    cmd.args.insert(0, "--disable-internal-agent".into());
    cmd.args.insert(1, "--keep-cwd".into());
    cmd.args.insert(2, executable);
    if crate::unix::linux::flatpak::is_inside_flatpak() {
        return crate::unix::linux::flatpak::spawn_host_command(cmd, context);
    }
    crate::unix::unix_spawn::spawn(cmd, context)
}
