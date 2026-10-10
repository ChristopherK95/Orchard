//! Worktree setup (ticket 08): the repo's setup commands, run one at a time in a new Worktree before
//! its first Agent session. `bash` on Linux; on Windows `pwsh` (else Windows PowerShell), or Git
//! Bash if the repo's settings ask for it. Output (stdout and stderr together) streams as it comes.
//! On Linux the commands get the environment of the user's login shell (`shell_env`), as a command
//! typed in the Terminal panel would.

#[cfg(unix)]
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::mpsc;

use crate::process::ProcessTree;
use crate::session::SessionId;
use crate::settings::WindowsShell;

/// A Worktree's setup, as the frontend shows it in place of the Tab it's waiting to start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupInfo {
    pub worktree: PathBuf,
    pub commands: Vec<String>,
    pub status: SetupStatus,
    /// Everything the commands printed (the latest `OUTPUT_LIMIT` bytes of it), each command
    /// introduced by a `> command` line.
    pub output: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SetupStatus {
    /// Running `commands[step]`.
    Running { step: usize },
    /// Every command succeeded (or the user chose Start anyway); the first session is starting.
    StartingSession,
    /// `commands[step]` failed.
    Failed { step: usize, message: String },
    /// The commands ran, but the Agent session didn't start.
    SessionFailed { message: String },
    #[serde(rename_all = "camelCase")]
    Done { session_id: SessionId },
}

/// How much setup output is kept for a view that opens late (the stream itself isn't capped).
pub(crate) const OUTPUT_LIMIT: usize = 256 * 1024;

/// Appends to the kept output, dropping the oldest text past `OUTPUT_LIMIT`.
pub(crate) fn keep_output(output: &mut String, text: &str) {
    output.push_str(text);
    if output.len() > OUTPUT_LIMIT {
        let mut cut = output.len() - OUTPUT_LIMIT;
        while !output.is_char_boundary(cut) {
            cut += 1;
        }
        output.drain(..cut);
    }
}

/// After the command exits, how long to wait for output still in its pipes. A process it started in
/// the background can hold them open indefinitely.
const LINGER: Duration = Duration::from_millis(500);

/// Output is passed on at most this often, so a noisy install doesn't flood the event channel.
const OUTPUT_BATCH: Duration = Duration::from_millis(50);

/// Runs one setup command in `cwd`, passing its output to `output` in batches as it arrives. `Err`
/// says why it failed (couldn't start, or a non-zero exit). Anything the command leaves running is
/// stopped with it, as is everything it started if this is dropped mid-run.
pub(crate) async fn run(
    command: &str,
    cwd: &Path,
    shell: WindowsShell,
    output: impl Fn(String),
) -> Result<(), String> {
    let (program, args) = shell_command(shell, command).await?;
    let mut cmd = crate::process::command(&program);
    #[cfg(unix)]
    cmd.envs(shell_env().await.iter().map(|(key, value)| (key, value)));
    cmd.args(&args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    ProcessTree::own_group(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("couldn't start {}: {e}", program.display()))?;
    let _tree = ProcessTree::attach(&child);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let readers = [
        tokio::spawn(forward(child.stdout.take().expect("piped"), tx.clone())),
        tokio::spawn(forward(child.stderr.take().expect("piped"), tx)),
    ];

    let mut pending = String::new();
    let mut flush = tokio::time::interval(OUTPUT_BATCH);
    flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let status = {
        let mut exited = pin!(child.wait());
        let mut status = None;
        let mut deadline = None;
        loop {
            tokio::select! {
                text = rx.recv() => match text {
                    Some(text) => pending.push_str(&text),
                    None => break, // both pipes closed
                },
                _ = flush.tick() => if !pending.is_empty() {
                    output(std::mem::take(&mut pending));
                },
                result = &mut exited, if status.is_none() => {
                    status = Some(result);
                    deadline = Some(tokio::time::Instant::now() + LINGER);
                }
                _ = tokio::time::sleep_until(deadline.unwrap_or_else(tokio::time::Instant::now)),
                    if deadline.is_some() => break,
            }
        }
        match status {
            Some(status) => status,
            None => exited.await,
        }
    };
    // Something the command left running may still hold the pipes; stop listening to it.
    for reader in readers {
        reader.abort();
    }
    if !pending.is_empty() {
        output(pending);
    }
    let status = status.map_err(|e| format!("couldn't wait for the command: {e}"))?;
    match status.code() {
        Some(0) => Ok(()),
        Some(code) => Err(format!("`{command}` exited with code {code}")),
        None => Err(format!("`{command}` was stopped before it finished")),
    }
}
/// Sends what `pipe` produces as text, never splitting a UTF-8 character across two sends.
async fn forward(mut pipe: impl AsyncRead + Unpin, tx: mpsc::UnboundedSender<String>) {
    let mut buffer = vec![0u8; 8192];
    let mut pending: Vec<u8> = vec![];
    while let Ok(read) = pipe.read(&mut buffer).await {
        if read == 0 {
            break;
        }
        pending.extend_from_slice(&buffer[..read]);
        let text = take_text(&mut pending);
        if !text.is_empty() && tx.send(text).is_err() {
            return;
        }
    }
    if !pending.is_empty() {
        let _ = tx.send(String::from_utf8_lossy(&pending).into_owned());
    }
}

/// Takes the text read so far out of `pending`, leaving a character split across reads to wait for
/// the rest of it.
pub(crate) fn take_text(pending: &mut Vec<u8>) -> String {
    let complete = match std::str::from_utf8(pending) {
        Ok(_) => pending.len(),
        // An incomplete character at the end waits for the rest; anything else is invalid.
        Err(err) if err.error_len().is_none() => err.valid_up_to(),
        Err(_) => pending.len(),
    };
    let text = String::from_utf8_lossy(&pending[..complete]).into_owned();
    pending.drain(..complete);
    text
}

/// The program and arguments that run `command` in the configured shell.
async fn shell_command(
    shell: WindowsShell,
    command: &str,
) -> Result<(PathBuf, Vec<String>), String> {
    if !cfg!(windows) {
        return Ok(("bash".into(), vec!["-c".into(), command.into()]));
    }
    Ok(match shell {
        WindowsShell::GitBash => (git_bash().await?, vec!["-c".into(), command.into()]),
        WindowsShell::Powershell => {
            let program = if on_path("pwsh.exe") {
                "pwsh"
            } else {
                "powershell"
            };
            // Output as UTF-8 (it's read as such), no progress bars (there's no console to draw them
            // in), and scripts like pnpm.ps1 allowed to run.
            let script = format!(
                "[Console]::OutputEncoding = $OutputEncoding = [System.Text.UTF8Encoding]::new($false)\n$ProgressPreference = 'SilentlyContinue'\n{command}"
            );
            (
                program.into(),
                vec![
                    "-NoLogo".into(),
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-ExecutionPolicy".into(),
                    "Bypass".into(),
                    "-Command".into(),
                    script,
                ],
            )
        }
    })
}

/// How long the login shell gets to report its environment before setup goes on without it.
#[cfg(unix)]
const SHELL_ENV_TIMEOUT: Duration = Duration::from_secs(10);

/// The environment the user's `$SHELL` sets up when started as an interactive login shell, read
/// once. Started from the desktop, Orchard lacks what the shell's profile adds: a version manager's
/// `node` first on `PATH` (nvm, fnm, mise), say. Empty if the shell couldn't report it.
#[cfg(unix)]
pub(crate) async fn shell_env() -> &'static [(OsString, OsString)] {
    static ENV: tokio::sync::OnceCell<Vec<(OsString, OsString)>> =
        tokio::sync::OnceCell::const_new();
    ENV.get_or_init(|| async {
        let Some(shell) = std::env::var_os("SHELL").filter(|shell| !shell.is_empty()) else {
            return vec![];
        };
        load_shell_env(&shell).await.unwrap_or_default()
    })
    .await
}

/// Runs `shell` as an interactive login shell and reads its environment.
#[cfg(unix)]
async fn load_shell_env(shell: &OsStr) -> Option<Vec<(OsString, OsString)>> {
    // Between markers, as the profile may print things of its own.
    const MARK: &str = "__ORCHARD_SHELL_ENV__";
    let mut cmd = crate::process::command(shell);
    cmd.args(["-i", "-l", "-c"])
        .arg(format!(
            "printf '%s' {MARK}; command env -0; printf '%s' {MARK}"
        ))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    ProcessTree::own_group(&mut cmd);
    let child = cmd.spawn().ok()?;
    let _tree = ProcessTree::attach(&child);
    let output = tokio::time::timeout(SHELL_ENV_TIMEOUT, child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    parse_env(&output.stdout, MARK.as_bytes())
}

/// The variables in `env -0` output found between two `mark`s, less the ones that describe the
/// shell that printed them rather than the user's setup.
#[cfg(unix)]
fn parse_env(output: &[u8], mark: &[u8]) -> Option<Vec<(OsString, OsString)>> {
    use std::os::unix::ffi::OsStrExt;
    let start = output.windows(mark.len()).position(|w| w == mark)? + mark.len();
    let end = start
        + output[start..]
            .windows(mark.len())
            .position(|w| w == mark)?;
    const SKIP: &[&[u8]] = &[b"PWD", b"OLDPWD", b"SHLVL", b"_"];
    Some(
        output[start..end]
            .split(|&byte| byte == 0)
            .filter_map(|entry| {
                let at = entry.iter().position(|&byte| byte == b'=')?;
                let (key, value) = (&entry[..at], &entry[at + 1..]);
                (!key.is_empty() && !SKIP.contains(&key)).then(|| {
                    (
                        OsStr::from_bytes(key).to_owned(),
                        OsStr::from_bytes(value).to_owned(),
                    )
                })
            })
            .collect(),
    )
}

/// Git for Windows' `bash.exe`, found from git's own install. Never a bare `bash`: on Windows that
/// finds WSL's first.
pub(crate) async fn git_bash() -> Result<PathBuf, String> {
    if let Some(exec_path) = crate::git::exec_path().await {
        for folder in exec_path.ancestors() {
            let bash = folder.join("bin").join("bash.exe");
            if bash.is_file() {
                return Ok(bash);
            }
        }
    }
    Err("couldn't find Git Bash (bin\\bash.exe in the Git for Windows install)".into())
}

pub(crate) fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn the_shell_env_is_read_between_the_marks_without_the_shells_own_variables() {
        let output = b"profile noise MARKPATH=/nvm/bin:/usr/bin\0PWD=/x\0MULTI=a\nb\0MARK more";
        let env = parse_env(output, b"MARK").unwrap();
        assert_eq!(
            env,
            [
                ("PATH".into(), "/nvm/bin:/usr/bin".into()),
                ("MULTI".into(), "a\nb".into()),
            ]
        );
        assert_eq!(parse_env(b"no marks here", b"MARK"), None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn what_the_shells_profile_sets_reaches_the_environment() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        // Stands in for a login shell whose profile prints a greeting and exports a variable.
        let shell = dir.path().join("shell");
        std::fs::write(
            &shell,
            "#!/bin/bash\necho welcome\nexport ORCHARD_TEST_VAR=from-profile\neval \"$4\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        let env = load_shell_env(shell.as_os_str()).await.unwrap();
        assert!(env
            .iter()
            .any(|(key, value)| key == "ORCHARD_TEST_VAR" && value == "from-profile"));
        assert!(!env.iter().any(|(key, _)| key == "PWD"));
    }

    #[test]
    fn kept_output_drops_the_oldest_text_on_a_character_boundary() {
        let mut output = "é".repeat(OUTPUT_LIMIT / 2);
        keep_output(&mut output, "x");
        assert!(output.len() <= OUTPUT_LIMIT);
        assert!(output.ends_with('x'));
    }

    #[tokio::test]
    async fn characters_split_across_reads_arrive_whole() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let bytes = "aé".as_bytes().to_vec();
        let (mut writer, reader) = tokio::io::duplex(1);
        let task = tokio::spawn(forward(reader, tx));
        use tokio::io::AsyncWriteExt;
        for byte in bytes {
            writer.write_all(&[byte]).await.unwrap();
            writer.flush().await.unwrap();
            tokio::task::yield_now().await;
        }
        drop(writer);
        task.await.unwrap();
        let mut text = String::new();
        while let Ok(part) = rx.try_recv() {
            assert!(!part.contains('\u{FFFD}'), "{part:?}");
            text.push_str(&part);
        }
        assert_eq!(text, "aé");
    }
}
