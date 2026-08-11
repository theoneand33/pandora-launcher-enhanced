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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitHint {
    Crash1,
    Abort134,
    Segfault139,
    Killed9,
    Segfault11,
    Abort6,
    AccessViolation,
    StackOverrun,
    WindowsException,
}

impl PandoraExitStatus {
    // ponytail: tiny match table, extend as new crash signatures appear.
    // Only actionable hints are emitted; benign exits like 130/143 (Ctrl+C/SIGTERM) return None.
    // Gated to known crash codes to avoid noisy Generic warnings on mod System.exit(n).
    pub fn human_hint(&self) -> Option<ExitHint> {
        #[cfg(unix)]
        {
            if libc::WIFEXITED(self.0) {
                return match libc::WEXITSTATUS(self.0) {
                    0 => None,
                    1 => Some(ExitHint::Crash1),
                    134 => Some(ExitHint::Abort134),
                    139 => Some(ExitHint::Segfault139),
                    130 | 143 => None,
                    _ => None,
                };
            }
            if libc::WIFSIGNALED(self.0) {
                return match libc::WTERMSIG(self.0) {
                    9 => Some(ExitHint::Killed9),
                    11 => Some(ExitHint::Segfault11),
                    6 => Some(ExitHint::Abort6),
                    _ => None,
                };
            }
            return None;
        }
        #[cfg(windows)]
        {
            return match self.0 {
                0 => None,
                1 => Some(ExitHint::Crash1),
                0xC0000005 => Some(ExitHint::AccessViolation),
                0xC0000409 => Some(ExitHint::StackOverrun),
                _ if (0xC0000000..=0xCFFFFFFF).contains(&self.0) => Some(ExitHint::WindowsException),
                _ => None,
            };
        }
    }
}
