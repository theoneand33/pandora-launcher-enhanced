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
                    1 => Some(t::instance::exit::crash1()),
                    134 => Some(t::instance::exit::abort134()),
                    139 => Some(t::instance::exit::segfault139()),
                    _ => Some(t::instance::exit::generic()),
                };
            }
            if libc::WIFSIGNALED(self.0) {
                return match libc::WTERMSIG(self.0) {
                    9 => Some(t::instance::exit::killed9()),
                    11 => Some(t::instance::exit::segfault11()),
                    6 => Some(t::instance::exit::abort6()),
                    _ => Some(t::instance::exit::terminated_signal()),
                };
            }
            Some(t::instance::exit::abnormal())
        }
        #[cfg(windows)]
        {
            match self.0 {
                0 => None,
                1 => Some(t::instance::exit::crash1()),
                0xC0000005 => Some(t::instance::exit::access_violation()),
                0xC0000409 => Some(t::instance::exit::stack_overrun()),
                _ if self.0 & 0x80000000 != 0 => Some(t::instance::exit::windows_exception()),
                _ => Some(t::instance::exit::generic()),
            }
        }
    }
}
