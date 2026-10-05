//! Worktree terminals: one interactive shell per Worktree, in a pseudo-terminal (ConPTY on Windows),
//! started when the Terminal panel first asks for it. It keeps running out of sight until it exits,
//! is closed, or its Worktree or Workspace goes. The panel draws it (xterm.js); the core keeps its
//! latest output, so the panel can come back to a shell it left.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tokio::sync::{mpsc, watch};

use crate::settings::WindowsShell;
use crate::setup::{git_bash, on_path, take_text};

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

/// A terminal's output for the panel: what it kept first, then what it prints from now on. Ends when
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

/// How much output is kept for the panel coming back.
const SCROLLBACK: usize = 512 * 1024;

/// After the shell exits, how long output still on its way has to arrive before the exit is told.
const LINGER: Duration = Duration::from_millis(200);

/// How long a stopped shell has to exit (a Worktree being removed waits for it to let go).
const STOP_WAIT: Duration = Duration::from_secs(3);

pub(crate) struct Terminal {
    /// Tells this shell from a later one in the same Worktree.
    pub(crate) id: u64,
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
    /// The panel, while it shows this terminal.
    viewer: Option<mpsc::UnboundedSender<TerminalOutput>>,
    /// How the shell exited, once it has (told again to a panel that comes back after).
    exited: Option<Option<u32>>,
}

impl Shared {
    fn send(&mut self, message: TerminalOutput) {
        if let Some(viewer) = &self.viewer {
            if viewer.send(message).is_err() {
                self.viewer = None;
            }
        }
    }
}

impl Terminal {
    /// Starts `shell` in `cwd` at `cols` × `rows`; `on_exit` runs (on another thread) once it has
    /// exited.
    pub(crate) fn start(
        id: u64,
        mut shell: CommandBuilder,
        cwd: &Path,
        cols: u16,
        rows: u16,
        on_exit: impl FnOnce() + Send + 'static,
    ) -> Result<Arc<Self>, String> {
        let pair = portable_pty::native_pty_system()
            .openpty(size(cols, rows))
            .map_err(|e| format!("couldn't open a pseudo-terminal: {e}"))?;
        shell.cwd(cwd);
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

        let output = shared.clone();
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
                keep_scrollback(&mut shared.output, &text);
                shared.send(TerminalOutput::Output { text });
            }
        });

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
            master: Mutex::new(Some(pair.master)),
            input,
            killer: Mutex::new(killer),
            shared,
            exited,
            #[cfg(windows)]
            _tree: tree,
        }))
    }

    /// Makes the panel this terminal's viewer: its kept output comes first, as a `Replay` if
    /// `returning` (a new shell's first output still wants its queries answered). A viewer before
    /// it stops getting anything.
    pub(crate) fn attach(&self, returning: bool) -> TerminalStream {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut shared = self.shared.lock().expect("terminal lock");
        if !shared.output.is_empty() {
            let text = shared.output.clone();
            let _ = tx.send(match returning {
                true => TerminalOutput::Replay { text },
                false => TerminalOutput::Output { text },
            });
        }
        if let Some(code) = shared.exited {
            let _ = tx.send(TerminalOutput::Exited { code });
        }
        shared.viewer = Some(tx);
        TerminalStream(rx)
    }

    /// Nothing shows this terminal now; its output is only kept.
    pub(crate) fn detach(&self) {
        self.shared.lock().expect("terminal lock").viewer = None;
    }

    pub(crate) fn write(&self, data: &str) {
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
