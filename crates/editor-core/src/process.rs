//! Spawning helpers shared by everything the core runs (the ACP adapter, git, version checks,
//! setup commands).

use std::ffi::{OsStr, OsString};
use std::sync::OnceLock;

use tokio::process::{Child, Command};

/// A command that won't flash a console window up from the GUI app on Windows, with the
/// AppImage's environment undone (`host_env`).
pub(crate) fn command(program: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new(program);
    for (key, value) in host_env() {
        match value {
            Some(value) => cmd.env(key, value),
            None => cmd.env_remove(key),
        };
    }
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd
}

/// What to change in the environment of what Orchard starts (git, the terminal's shell, the
/// adapter), as (variable, new value or `None` to unset). Run from an AppImage, Orchard was
/// started with the image's own folders first on `LD_LIBRARY_PATH`, `PATH` and the like, for its
/// bundled libraries: the system's git would load the image's older libcurl or pcre2 and fail. So
/// the image's folders are dropped from every variable, and what its launcher set just for the
/// app is unset. Empty outside an AppImage.
pub fn host_env() -> &'static [(OsString, Option<OsString>)] {
    static FIXES: OnceLock<Vec<(OsString, Option<OsString>)>> = OnceLock::new();
    FIXES.get_or_init(
        || match (std::env::var("APPDIR"), std::env::var_os("APPIMAGE")) {
            (Ok(appdir), Some(_)) => appimage_fixes(&appdir, std::env::vars_os()),
            _ => vec![],
        },
    )
}

/// `host_env` for an AppImage mounted at `appdir`, given the current environment.
fn appimage_fixes(
    appdir: &str,
    vars: impl Iterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, Option<OsString>)> {
    let appdir = appdir.trim_end_matches('/');
    if appdir.is_empty() {
        return vec![];
    }
    // Set by the launcher for the app alone (`GTK_THEME` by the image's GTK hook).
    const APP_ONLY: &[&str] = &[
        "APPDIR",
        "APPIMAGE",
        "ARGV0",
        "OWD",
        "PYTHONDONTWRITEBYTECODE",
        "GTK_THEME",
    ];
    vars.filter_map(|(key, value)| {
        if key.to_str().is_some_and(|key| APP_ONLY.contains(&key)) {
            return Some((key, None));
        }
        let text = value.to_str()?;
        if !text.contains(appdir) {
            return None;
        }
        let kept: Vec<&str> = text
            .split(':')
            .filter(|part| !part.is_empty() && !part.starts_with(appdir))
            .collect();
        let value = (!kept.is_empty()).then(|| OsString::from(kept.join(":")));
        Some((key, value))
    })
    .collect()
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

// The job handle is only closed, once, in `drop` (nothing else touches it).
#[cfg(windows)]
unsafe impl Send for ProcessTree {}
#[cfg(windows)]
unsafe impl Sync for ProcessTree {}

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
        Self::attach_handle(child.raw_handle()?)
    }

    /// `attach` for a process known by its handle (a terminal's shell, say).
    #[cfg(windows)]
    pub(crate) fn attach_handle(process: std::os::windows::io::RawHandle) -> Option<Self> {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
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

/// Where a command's output goes as it comes (stdout and stderr alike, in whole characters).
pub(crate) type Sink<'a> = &'a (dyn Fn(&str) + Send + Sync);

/// Waits for `child` like `wait_with_output`, handing its output to `sink` as it arrives (when
/// there is one). Its stdout and stderr must be piped.
pub(crate) async fn collect(
    mut child: Child,
    sink: Option<Sink<'_>>,
) -> std::io::Result<std::process::Output> {
    use tokio::io::{AsyncRead, AsyncReadExt};
    async fn read(pipe: Option<impl AsyncRead + Unpin>, sink: Option<Sink<'_>>) -> Vec<u8> {
        let Some(mut pipe) = pipe else {
            return vec![];
        };
        let mut all = vec![];
        let mut pending = vec![];
        // (On the heap: held across the `.await`, an array would make every future awaiting a
        // command 16 KB bigger, and a command's future is first built on the 1 MB main thread.)
        let mut buffer = vec![0u8; 8192];
        while let Ok(read) = pipe.read(&mut buffer).await {
            if read == 0 {
                break;
            }
            all.extend_from_slice(&buffer[..read]);
            if let Some(sink) = sink {
                pending.extend_from_slice(&buffer[..read]);
                let text = crate::setup::take_text(&mut pending);
                if !text.is_empty() {
                    sink(&text);
                }
            }
        }
        if let (Some(sink), false) = (sink, pending.is_empty()) {
            sink(&String::from_utf8_lossy(&pending));
        }
        all
    }
    let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
    let (stdout, stderr, status) =
        tokio::join!(read(stdout, sink), read(stderr, sink), child.wait());
    Ok(std::process::Output {
        status: status?,
        stdout,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_appimages_folders_and_launcher_variables_are_dropped() {
        let vars = [
            ("APPDIR", "/tmp/.mount_X"),
            (
                "LD_LIBRARY_PATH",
                "/tmp/.mount_X/usr/lib/:/tmp/.mount_X/usr/lib32/",
            ),
            (
                "PATH",
                "/tmp/.mount_X/usr/bin/:/home/me/.local/bin:/usr/bin",
            ),
            ("GTK_DATA_PREFIX", "/tmp/.mount_X"),
            ("XDG_DATA_DIRS", "/tmp/.mount_X/usr/share/:/usr/share:"),
            ("GTK_THEME", "Adwaita:dark"),
            ("HOME", "/home/me"),
        ]
        .map(|(k, v)| (OsString::from(k), OsString::from(v)));
        let fixes = appimage_fixes("/tmp/.mount_X/", vars.into_iter());
        let fix = |key: &str| {
            fixes
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_ref().map(|v| v.to_str().unwrap().to_owned()))
        };
        assert_eq!(fix("APPDIR"), Some(None));
        assert_eq!(fix("LD_LIBRARY_PATH"), Some(None));
        assert_eq!(fix("GTK_DATA_PREFIX"), Some(None));
        assert_eq!(fix("GTK_THEME"), Some(None));
        assert_eq!(
            fix("PATH"),
            Some(Some("/home/me/.local/bin:/usr/bin".to_owned()))
        );
        assert_eq!(fix("XDG_DATA_DIRS"), Some(Some("/usr/share".to_owned())));
        assert_eq!(fix("HOME"), None, "left alone");
    }
}
