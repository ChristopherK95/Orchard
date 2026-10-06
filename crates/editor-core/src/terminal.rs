//! Worktree terminals: interactive shells in a Worktree's folder, each in a pseudo-terminal (ConPTY
//! on Windows). The first starts when a Terminal panel first asks for the Worktree's; more when the
//! panel asks for another. Each keeps running out of sight until it exits, is stopped, or its
//! Worktree or Workspace goes. A Terminal panel draws one at a time (xterm.js): the Tabs view's, or
//! a column's in the Columns view, each a view slot of its own. The core keeps each one's latest
//! output, so a panel can come back to a shell it left.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tokio::sync::{mpsc, watch};

use crate::settings::WindowsShell;
use crate::setup::{git_bash, on_path, take_text};

/// Numbers each shell, from 0 for the Workspace's first.
pub type TerminalId = u64;

/// A running shell, for the Terminal panel's tabs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    pub id: TerminalId,
    pub worktree: PathBuf,
    /// What it's running: the command in the foreground where that can be told (Linux), else the
    /// shell's name (`pwsh`, `bash`…).
    pub name: String,
    /// Something other than the shell is in the foreground (Linux only; never on Windows), so
    /// stopping it would stop that too.
    pub busy: bool,
}

/// What the Terminal panel is sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TerminalOutput {
    /// What the shell printed, escape sequences and all.
    Output { text: String },
    /// What it printed before the panel came back to it. Queries in it (the cursor position, say)
    /// were answered then, and mustn't be again: the answer would be typed into the shell.
    Replay { text: String },
    /// The shell exited, with its exit code where it has one.
    Exited { code: Option<u32> },
}

/// A terminal's output for a panel: what it kept first, then what it prints from now on. Ends when
/// the panel shows another terminal, or this one goes.
pub struct TerminalStream(mpsc::UnboundedReceiver<TerminalOutput>);

impl TerminalStream {
    pub async fn next(&mut self) -> Option<TerminalOutput> {
        self.0.recv().await
    }

    /// The next output if it has already arrived (to send what's waiting in one go).
    pub fn try_next(&mut self) -> Option<TerminalOutput> {
        self.0.try_recv().ok()
    }
}

/// A terminal asking where the cursor is (DSR 6), and the answer given when no panel can: the top
/// left corner.
const CURSOR_QUERY: &str = "\x1b[6n";
const CURSOR_ANSWER: &str = "\x1b[1;1R";

/// How much output is kept for the panel coming back.
const SCROLLBACK: usize = 512 * 1024;

/// After the shell exits, how long output still on its way has to arrive before the exit is told.
const LINGER: Duration = Duration::from_millis(200);

/// How long a stopped shell has to exit (a Worktree being removed waits for it to let go).
const STOP_WAIT: Duration = Duration::from_secs(3);

pub(crate) struct Terminal {
    pub(crate) id: TerminalId,
    /// The Worktree it runs in.
    pub(crate) worktree: PathBuf,
    /// The shell's program name, without a path or extension.
    shell: String,
    /// Taken when the terminal goes, to be closed on a thread of its own: closing a ConPTY can
    /// block for a while.
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    /// Keystrokes, written in order on a thread of their own: a shell that isn't reading can block
    /// a write.
    input: std::sync::mpsc::Sender<Vec<u8>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    shared: Arc<Mutex<Shared>>,
    exited: watch::Receiver<bool>,
    /// Whatever the shell starts goes with it (Linux: the pty hanging up does that).
    #[cfg(windows)]
    _tree: Option<crate::process::ProcessTree>,
}

#[derive(Default)]
struct Shared {
    /// The latest `SCROLLBACK` bytes of output.
    output: String,
    /// The panels showing this terminal, by view slot.
    viewers: HashMap<String, mpsc::UnboundedSender<TerminalOutput>>,
    /// How the shell exited, once it has (told again to a panel that comes back after).
    exited: Option<Option<u32>>,
    /// The core answered a query in the kept output itself, so a panel mustn't again.
    answered: bool,
    /// A cursor position query went to a panel that hasn't typed anything since (its answer, say).
    query_pending: bool,
}

impl Shared {
    fn send(&mut self, message: TerminalOutput) {
        self.viewers
            .retain(|_, viewer| viewer.send(message.clone()).is_ok());
    }
}

impl Terminal {
    /// Starts `shell` in `cwd` at `cols` × `rows`; `on_exit` runs (on another thread) once it has
    /// exited.
    pub(crate) fn start(
        id: TerminalId,
        mut shell: CommandBuilder,
        cwd: &Path,
        cols: u16,
        rows: u16,
        on_exit: impl FnOnce() + Send + 'static,
    ) -> Result<Arc<Self>, String> {
        let pair = portable_pty::native_pty_system()
            .openpty(size(cols, rows))
            .map_err(|e| format!("couldn't open a pseudo-terminal: {e}"))?;
        let shell_name = shell
            .get_argv()
            .first()
            .and_then(|program| Path::new(program).file_stem())
            .map_or_else(
                || "shell".to_owned(),
                |stem| stem.to_string_lossy().into_owned(),
            );
        shell.cwd(cwd);
        for (key, value) in crate::process::host_env() {
            match value {
                Some(value) => shell.env(key, value),
                None => shell.env_remove(key),
            }
        }
        if !cfg!(windows) {
            shell.env("TERM", "xterm-256color");
        }
        shell.env("COLORTERM", "truecolor");
        shell.env("TERM_PROGRAM", "Orchard");
        let mut child = pair
            .slave
            .spawn_command(shell)
            .map_err(|e| format!("couldn't start the shell: {e}"))?;
        drop(pair.slave); // (the shell has its own; ours would keep the pty open after it exits)
        #[cfg(windows)]
        let tree = child
            .as_raw_handle()
            .and_then(crate::process::ProcessTree::attach_handle);
        let killer = child.clone_killer();
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("couldn't read the terminal: {e}"))?;
        let mut writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("couldn't write to the terminal: {e}"))?;
        let shared = Arc::new(Mutex::new(Shared::default()));

        let (input, keystrokes) = std::sync::mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            for bytes in keystrokes {
                if writer
                    .write_all(&bytes)
                    .and_then(|_| writer.flush())
                    .is_err()
                {
                    break;
                }
            }
        });

        let output = shared.clone();
        let answer = input.clone();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            let mut pending = vec![];
            while let Ok(read) = reader.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                pending.extend_from_slice(&buffer[..read]);
                let text = take_text(&mut pending);
                if text.is_empty() {
                    continue;
                }
                let mut shared = output.lock().expect("terminal lock");
                // A cursor position query with no panel to answer it (a shell started, or still
                // starting, out of sight): ConPTY waits for the answer, so the core gives one. A
                // panel coming back replays the query and doesn't answer again.
                if shared.viewers.is_empty() {
                    for _ in text.matches(CURSOR_QUERY) {
                        let _ = answer.send(CURSOR_ANSWER.as_bytes().to_vec());
                        shared.answered = true;
                    }
                } else if text.contains(CURSOR_QUERY) {
                    shared.query_pending = true;
                }
                keep_scrollback(&mut shared.output, &text);
                shared.send(TerminalOutput::Output { text });
            }
        });

        let (exit_tx, exited) = watch::channel(false);
        let told = shared.clone();
        std::thread::spawn(move || {
            let code = child.wait().ok().map(|status| status.exit_code());
            std::thread::sleep(LINGER);
            let mut shared = told.lock().expect("terminal lock");
            shared.exited = Some(code);
            shared.send(TerminalOutput::Exited { code });
            drop(shared);
            let _ = exit_tx.send(true);
            on_exit();
        });

        Ok(Arc::new(Self {
            id,
            worktree: cwd.to_owned(),
            shell: shell_name,
            master: Mutex::new(Some(pair.master)),
            input,
            killer: Mutex::new(killer),
            shared,
            exited,
            #[cfg(windows)]
            _tree: tree,
        }))
    }

    pub(crate) fn info(&self) -> TerminalInfo {
        let foreground = self.foreground();
        TerminalInfo {
            id: self.id,
            worktree: self.worktree.clone(),
            busy: foreground.is_some(),
            name: foreground.unwrap_or_else(|| self.shell.clone()),
        }
    }

    /// The command in the shell's foreground, if it isn't the shell itself (Linux: the pty's
    /// process group leader, by its `/proc` name).
    #[cfg(unix)]
    fn foreground(&self) -> Option<String> {
        let leader = self
            .master
            .lock()
            .expect("terminal lock")
            .as_ref()?
            .process_group_leader()?;
        let name = std::fs::read_to_string(format!("/proc/{leader}/comm")).ok()?;
        let name = name.trim();
        (!name.is_empty() && name != self.shell).then(|| name.to_owned())
    }

    #[cfg(not(unix))]
    fn foreground(&self) -> Option<String> {
        None
    }

    /// Shows this terminal in view slot `slot`: its kept output comes first, as a `Replay` if
    /// `returning` (a new shell's first output still wants its queries answered). What the slot
    /// was sent before ends.
    pub(crate) fn attach(&self, slot: &str, returning: bool) -> TerminalStream {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut shared = self.shared.lock().expect("terminal lock");
        if !shared.output.is_empty() {
            let text = shared.output.clone();
            let _ = tx.send(match returning || shared.answered {
                true => TerminalOutput::Replay { text },
                false => TerminalOutput::Output { text },
            });
        }
        if let Some(code) = shared.exited {
            let _ = tx.send(TerminalOutput::Exited { code });
        }
        shared.viewers.insert(slot.to_owned(), tx);
        TerminalStream(rx)
    }

    /// View slot `slot` no longer shows this terminal.
    /// View slot `slot` no longer shows this terminal. A query it was sent and didn't answer is
    /// answered by the core when no panel is left to (the panel moved on before it could).
    pub(crate) fn detach(&self, slot: &str) {
        let mut shared = self.shared.lock().expect("terminal lock");
        if shared.viewers.remove(slot).is_some()
            && shared.viewers.is_empty()
            && shared.query_pending
        {
            shared.query_pending = false;
            shared.answered = true;
            let _ = self.input.send(CURSOR_ANSWER.as_bytes().to_vec());
        }
    }

    pub(crate) fn write(&self, data: &str) {
        self.shared.lock().expect("terminal lock").query_pending = false;
        let _ = self.input.send(data.as_bytes().to_vec());
    }

    pub(crate) fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        match self.master.lock().expect("terminal lock").as_ref() {
            Some(master) => master
                .resize(size(cols, rows))
                .map_err(|e| format!("couldn't resize the terminal: {e}")),
            None => Ok(()),
        }
    }

    /// Stops the shell (and what it started), waiting a while for it to exit.
    pub(crate) async fn stop(&self) {
        let _ = self.killer.lock().expect("terminal lock").kill();
        let mut exited = self.exited.clone();
        let _ = tokio::time::timeout(STOP_WAIT, exited.wait_for(|exited| *exited)).await;
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.killer.lock().expect("terminal lock").kill();
        if let Some(master) = self.master.lock().expect("terminal lock").take() {
            std::thread::spawn(move || drop(master));
        }
    }
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// Appends to the kept output, dropping the oldest lines past `SCROLLBACK` (whole lines, so the
/// replay doesn't start inside an escape sequence).
fn keep_scrollback(output: &mut String, text: &str) {
    output.push_str(text);
    if output.len() > SCROLLBACK {
        let mut over = output.len() - SCROLLBACK;
        while !output.is_char_boundary(over) {
            over += 1;
        }
        let cut = output[over..]
            .find('\n')
            .map_or(output.len(), |at| over + at + 1);
        output.drain(..cut);
    }
}

/// The shell a Worktree's terminal runs: on Windows the repo's `windows_shell` (`pwsh` if it's
/// installed, else Windows PowerShell; or Git Bash), on Linux the user's `$SHELL`.
pub(crate) async fn shell(windows_shell: WindowsShell) -> Result<CommandBuilder, String> {
    if !cfg!(windows) {
        let program = std::env::var_os("SHELL")
            .filter(|shell| !shell.is_empty())
            .unwrap_or_else(|| "bash".into());
        return Ok(CommandBuilder::new(program));
    }
    Ok(match windows_shell {
        WindowsShell::Powershell => {
            let mut cmd = CommandBuilder::new(if on_path("pwsh.exe") {
                "pwsh.exe"
            } else {
                "powershell.exe"
            });
            cmd.arg("-NoLogo");
            cmd
        }
        WindowsShell::GitBash => {
            let mut cmd = CommandBuilder::new(git_bash().await?);
            cmd.args(["--login", "-i"]);
            cmd.env("CHERE_INVOKING", "1"); // (else the login profile goes to the home folder)
            cmd
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kept_output_drops_whole_lines_from_the_front() {
        let mut output = "x".repeat(SCROLLBACK - 4) + "\nold\n";
        keep_scrollback(&mut output, "new\n");
        assert!(output.len() <= SCROLLBACK);
        assert_eq!(output, "old\nnew\n");
    }
}
