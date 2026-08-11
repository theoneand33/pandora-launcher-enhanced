use std::fmt::{Debug, Display};

#[cfg(windows)]
#[derive(Clone, Copy, Debug)]
pub struct PandoraExitStatus(pub(crate) u32);

#[cfg(windows)]
impl Display for PandoraExitStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0 & 0x80000000 != 0 {
            f.write_fmt(format_args!("exitcode=0x{:#x}", self.0))
        } else {
            f.write_fmt(format_args!("exitcode={}", self.0))
        }
    }
}

#[cfg(unix)]
#[derive(Clone, Copy)]
pub struct PandoraExitStatus(pub(crate) libc::c_int);

#[cfg(unix)]
impl Display for PandoraExitStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if libc::WIFEXITED(self.0) {
            f.write_fmt(format_args!("exitcode={}", libc::WEXITSTATUS(self.0)))
        } else if libc::WIFSIGNALED(self.0) {
            f.write_fmt(format_args!("signal={}", libc::WTERMSIG(self.0)))
        } else {
            f.write_fmt(format_args!("unknownwait=0x{:#x}", self.0))
        }
    }
}

#[cfg(unix)]
impl Debug for PandoraExitStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("PandoraExitStatus");
        debug.field("raw", &self.0);
        if libc::WIFEXITED(self.0) {
            debug.field("exitcode", &libc::WEXITSTATUS(self.0));
        }
        if libc::WIFSIGNALED(self.0) {
            debug.field("signal", &libc::WTERMSIG(self.0));
        }
        debug.finish()
    }
}

impl PandoraExitStatus {
    // ponytail: tiny match table, extend as new crash signatures appear.
    pub fn human_hint(&self) -> Option<&'static str> {
        #[cfg(unix)]
        {
            if libc::WIFEXITED(self.0) {
                return match libc::WEXITSTATUS(self.0) {
                    0 => None,
                    1 => Some("Minecraft crashed (exit 1) — check Logs / Game Output for the stacktrace"),
                    134 => Some("Aborted (exit 134) — native library crash, try updating GLFW/OpenAL or Java"),
                    139 => Some("Segfault (exit 139) — native crash, check graphics drivers"),
                    _ => Some("Minecraft exited with an error — check Logs for details"),
                };
            }
            if libc::WIFSIGNALED(self.0) {
                return match libc::WTERMSIG(self.0) {
                    9 => Some("Killed (signal 9) — likely out of memory, try lower -Xmx or closing apps"),
                    11 => Some("Segmentation fault (signal 11) — faulty native library or driver"),
                    6 => Some("Aborted (signal 6) — native library aborted"),
                    _ => Some("Minecraft was terminated by a signal — check Logs"),
                };
            }
            Some("Minecraft ended abnormally — check Logs")
        }
        #[cfg(windows)]
        {
            match self.0 {
                0 => None,
                1 => Some("Minecraft crashed (exit 1) — check Logs / Game Output for the stacktrace"),
                0xC0000005 => {
                    Some("Access violation (0xC0000005) — faulty native library (GLFW/OpenAL) or wrong Java version")
                },
                0xC0000409 => Some("Stack buffer overrun (0xC0000409) — native crash, update drivers/Java"),
                _ if self.0 & 0x80000000 != 0 => Some("Minecraft crashed with a Windows exception — check Logs"),
                _ => Some("Minecraft exited with an error — check Logs for details"),
            }
        }
    }
}
