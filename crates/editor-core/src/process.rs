//! Spawning helpers shared by everything the core runs (the ACP adapter, git, version checks,
//! setup commands).

use std::ffi::OsStr;

use tokio::process::{Child, Command};

/// A command that won't flash a console window up from the GUI app on Windows.
pub(crate) fn command(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd
}

/// Kills a process and everything it started when dropped: killing just a shell would leave the
/// `node` or `cargo` it ran behind, still holding the Worktree's folder open. Windows puts the
/// process in a Job object; Linux in its own process group (call `own_group` before spawning).
pub(crate) struct ProcessTree {
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
    #[cfg(unix)]
    group: i32,
}

// The job handle is only closed, once, in `drop`.
#[cfg(windows)]
unsafe impl Send for ProcessTree {}

impl ProcessTree {
    /// Makes the command start its own process group, so `attach` can stop the whole tree.
    pub(crate) fn own_group(cmd: &mut Command) {
        #[cfg(unix)]
        cmd.process_group(0);
        #[cfg(not(unix))]
        let _ = cmd;
    }

    /// Ties the spawned `child` (and whatever it starts from now on) to the returned guard. On Windows
    /// the child joins the job just after it starts, so something it starts in that instant could
    /// escape; a shell running a setup command hasn't started anything yet.
    #[cfg(windows)]
    pub(crate) fn attach(child: &Child) -> Option<Self> {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        let process = child.raw_handle()?;
        // SAFETY: plain Win32 calls on handles we own; `info` lives across the call that reads it.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let limited = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) != 0;
            if !limited || AssignProcessToJobObject(job, process as _) == 0 {
                CloseHandle(job);
                return None;
            }
            Some(Self { job })
        }
    }

    #[cfg(unix)]
    pub(crate) fn attach(child: &Child) -> Option<Self> {
        Some(Self {
            group: i32::try_from(child.id()?).ok()?,
        })
    }
}

impl Drop for ProcessTree {
    fn drop(&mut self) {
        // SAFETY: the handle / group id came from `attach`; this is the only place it's released.
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::JobObjects::TerminateJobObject;
            TerminateJobObject(self.job, 1);
            CloseHandle(self.job);
        }
        // On Linux the group's leader may already be reaped, so in principle its id could have been
        // reused for a new group; the window is tiny and accepted.
        #[cfg(unix)]
        unsafe {
            libc::killpg(self.group, libc::SIGKILL);
        }
    }
}
