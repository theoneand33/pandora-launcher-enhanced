use std::{ffi::OsStr, io::Error};

use crate::{PandoraChild, PandoraCommand, spawner::SpawnContext};

pub fn is_inside_flatpak() -> bool {
    // Flatpak sandbox always contains /.flatpak-info and sets FLATPAK_ID
    if std::path::Path::new("/.flatpak-info").exists() {
        return true;
    }
    if std::env::var_os("FLATPAK_ID").is_some() {
        return true;
    }
    false
}

pub fn spawn(mut cmd: PandoraCommand, context: &mut SpawnContext) -> std::io::Result<PandoraChild> {
    if !is_inside_flatpak() {
        return crate::unix::unix_spawn::spawn(cmd, context);
    }

    let Some(flatpak_spawn) = crate::path_cache::get_command_path_cached(OsStr::new("flatpak-spawn")) else {
        // Fallback to direct spawn if helper is missing
        return crate::unix::unix_spawn::spawn(cmd, context);
    };

    let executable = std::mem::replace(&mut cmd.executable, flatpak_spawn.as_os_str().to_os_string().into());
    // flatpak-spawn --host <cmd> <args>
    cmd.args.insert(0, executable);
    cmd.args.insert(0, "--host".into());

    crate::unix::unix_spawn::spawn(cmd, context)
}

pub fn spawn_host_command(mut cmd: PandoraCommand, context: &mut SpawnContext) -> std::io::Result<PandoraChild> {
    // Explicit host execution via flatpak-spawn, used by elevated paths like pkexec
    if !is_inside_flatpak() {
        return crate::unix::unix_spawn::spawn(cmd, context);
    }
    let Some(flatpak_spawn) = crate::path_cache::get_command_path_cached(OsStr::new("flatpak-spawn")) else {
        return Err(Error::new(std::io::ErrorKind::NotFound, "cannot find 'flatpak-spawn'"));
    };
    let executable = std::mem::replace(&mut cmd.executable, flatpak_spawn.as_os_str().to_os_string().into());
    cmd.args.insert(0, executable);
    cmd.args.insert(0, "--host".into());
    crate::unix::unix_spawn::spawn(cmd, context)
}
