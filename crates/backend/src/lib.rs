#![deny(unused_must_use)]

use std::ffi::{OsStr, OsString};

mod backend;
pub use backend::*;

mod account;
mod arcfactory;
mod backend_filesystem;
mod backend_handler;
mod directories;
mod duplicate;
mod export;
mod fs;
mod id_slab;
mod install_content;
mod instance;
mod java_manifest;
mod launch;
mod launch_wrapper;
mod launcher_import;
mod lockfile;
mod log_reader;
mod metadata;
mod mod_metadata;
mod p2p_sync;
mod persistent;
mod server_list_pinger;
mod shortcut;
mod skin_manager;
mod skin_server;
mod syncing;
mod update;

pub const KNOWN_SHADER_MODS: &[&'static str] = &["iris", "oculus", "optifine"];

pub fn join_windows_shell(args: &[&str]) -> String {
    let os_args: Vec<&OsStr> = args.iter().map(|s| OsStr::new(s)).collect();
    let os_string = join_windows_shell_os(&os_args);
    // SAFETY: os_string contains original UTF-8 argument bytes plus ASCII quoting bytes.
    os_string.into_string().expect("join_windows_shell produced non-UTF-8 output")
}

pub fn join_windows_shell_os(args: &[&OsStr]) -> OsString {
    let mut string = Vec::new();

    let mut first = true;
    for arg in args {
        let mut backslashes = 0;

        if first {
            first = false;
        } else {
            string.push(b' ');
        }

        if arg.is_empty() {
            string.extend(b"\"\"");
            continue;
        }

        let arg_raw = arg.as_encoded_bytes();
        let quoted = arg_raw.contains(&b' ') || arg_raw.contains(&b'\t');
        if quoted {
            string.push(b'"');
        }

        for byte in arg_raw {
            if *byte == b'\\' {
                backslashes += 1;
            } else if *byte == b'"' {
                for _ in 0..backslashes * 2 {
                    string.push(b'\\');
                }
                string.push(b'\\');
                string.push(b'"');
                backslashes = 0;
            } else {
                for _ in 0..backslashes {
                    string.push(b'\\');
                }
                backslashes = 0;
                string.push(*byte);
            }
        }

        if quoted {
            for _ in 0..backslashes * 2 {
                string.push(b'\\');
            }
        } else {
            for _ in 0..backslashes {
                string.push(b'\\');
            }
        }

        if quoted {
            string.push(b'"');
        }
    }

    // SAFETY: string contains arguments' encoded bytes in order and inserted ASCII bytes (' ', '"', '\\') cannot split multibyte sequences.
    unsafe { OsString::from_encoded_bytes_unchecked(string) }
}
