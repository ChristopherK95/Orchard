// Typed access to the Rust core over Tauri IPC. The shapes mirror editor-core's serde types.
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type SessionId = number;
export type SessionState = "working" | "needsYou" | "idle" | "suspended" | "exited";
export type PermissionMode = "askForEdits" | "acceptEdits" | "plan";

export interface WorkspaceInfo {
  root: string;
  name: string;
}

export interface SessionInfo {
  id: SessionId;
  name: string;
  worktree: string;
  state: SessionState;
  permissionMode: PermissionMode;
  /** New transcript items since the Tab was last shown. */
  unread: number;
}

/** A file as the Manual editor opens it; `version` goes back with the save. */
export interface OpenedFile {
  path: string;
  content:
    | {
        kind: "text";
        /** With "\n" line endings; `lineEnding` is the file's own, put back on save. */
        text: string;
        lineEnding: string;
        mixedLineEndings: boolean;
        readOnly: boolean;
        minified: boolean;
      }
    | { kind: "binary"; bytes: number }
    | { kind: "notUtf8"; bytes: number }
    | { kind: "tooBig"; bytes: number };
  version: string;
}

/** A file popped out into a window of its own: what it carries (undo history doesn't come along). */
export interface PoppedOutFile {
  path: string;
  text: string;
  /** As last saved, so the new window knows what's unsaved. */
  savedText: string;
  /** CodeMirror offsets into `text`. */
  cursor: number;
  anchor: number;
  version: string;
  lineEnding: string;
  wrap: boolean;
}

/** What a save may write over: the version read, or (once the user said so) anything. */
export type SaveOver = { kind: "version"; version: string } | { kind: "anything" };

export type SaveOutcome = { kind: "saved"; version: string } | { kind: "changedOnDisk" };

/** A Ctrl+P result. */
export interface FileMatch {
  /** `/`-separated, relative to the Worktree. */
  path: string;
  /** Offsets of the matched characters, for highlighting. */
  indices: number[];
}

/** An entry of a folder in the Files drawer. */
export interface DirEntry {
  name: string;
  /** `/`-separated, relative to the Worktree. */
  path: string;
  isDir: boolean;
  /** The file's git status code (`M`, `A`, `D`, `??`), if it's changed. */
  change: string | null;
  /** A folder with changed files inside. */
  hasChanges: boolean;
}

/** A closed Tab in a Worktree's Recent sessions. */
export interface RecentSession {
  /** The ACP session id, which `reopenSession` takes. */
  acpId: string;
  name: string;
  worktree: string;
}

export interface WorktreeInfo {
  path: string;
  /** The checked-out branch, or null when HEAD is detached. */
  branch: string | null;
  /** HEAD's short commit id. */
  head: string;
  isMain: boolean;
  /** Commits ahead of / behind the upstream; null without an upstream. */
  ahead: number | null;
  behind: number | null;
  changed: number;
}

/** What "＋ worktree" creates: a new branch from a start point, or an existing (local or remote) branch. */
export type NewWorktree =
  | { kind: "newBranch"; name: string; startPoint: string | null }
  | { kind: "existingBranch"; name: string };

export interface BranchInfo {
  /** `feat/x` for a local branch, `origin/feat/x` for a remote one. */
  name: string;
  remote: boolean;
  /** Where a local branch is checked out; such a branch can't be checked out again. */
  checkedOutIn: string | null;
}

export interface CreatedWorktree {
  worktree: WorktreeInfo;
  /** E.g. the fetch failed, so it started from what was fetched last. */
  warning: string | null;
  /** The repo's Worktree setup, now running; the core starts the first session after it. Null when
   *  the repo has none, so the caller starts the session. */
  setup: SetupInfo | null;
}

export interface CommitSummary {
  id: string;
  subject: string;
}

/** What removing a Worktree would stop and lose, for the confirmation dialog. */
export interface RemovalCheck {
  worktree: string;
  branch: string | null;
  /** Agent sessions that removal stops first. */
  sessions: SessionId[];
  /** `XY path` lines as `git status --short` shows them (`??` = untracked); the first 100. */
  changes: string[];
  changedCount: number;
  /** Ignored files and folders (`node_modules/`, `.env`): deleted too, shown so it's no surprise. */
  ignored: string[];
  ignoredCount: number;
  /** Commits only this Worktree has (not pushed, merged into the Base, or on another branch). */
  unpushed: CommitSummary[];
  unpushedCount: number;
  base: string;
  /** Merged into the Base: "Delete branch too" starts ticked. */
  merged: boolean;
  /** Removing it at all needs "Discard and remove". */
  discardToRemove: boolean;
  /** Deleting the branch too needs "Discard and remove". */
  discardToDeleteBranch: boolean;
  /** Identifies exactly the work listed; "Discard and remove" sends it back. */
  fingerprint: string;
}

export interface RemoveWorktree {
  /** "Discard and remove": the shown check's fingerprint (work that changed since is refused). */
  discard: string | null;
  deleteBranch: boolean;
}

export interface RemovedWorktree {
  /** E.g. the Worktree is gone but its branch couldn't be deleted. */
  warning: string | null;
}

export type SetupStatus =
  | { kind: "running"; step: number }
  | { kind: "startingSession" }
  | { kind: "failed"; step: number; message: string }
  /** The commands ran (or were skipped), but the Agent session didn't start. */
  | { kind: "sessionFailed"; message: string }
  | { kind: "done"; sessionId: SessionId };

export interface SetupInfo {
  worktree: string;
  commands: string[];
  status: SetupStatus;
  /** What the commands printed so far (the latest 256 KB). */
  output: string;
}

export interface Settings {
  editor: { vim: boolean };
  notifications: { turnFinished: boolean };
  agents: { memoryLimitMb: number; idleSuspend: boolean; idleSuspendMinutes: number };
}

export interface LoadedSettings {
  settings: Settings;
  /** Why the settings file was rejected; the settings above are the last good ones. */
  error: string | null;
}

export interface BranchList {
  branches: BranchInfo[];
  /** Set when origin couldn't be fetched (the list is as last fetched). */
  warning: string | null;
}

export interface MissingPrerequisite {
  tool: string;
  found: string | null;
  message: string;
}

export type PermissionOptionKind = "allowOnce" | "allowAlways" | "rejectOnce" | "rejectAlways";

export interface PermissionOption {
  id: string;
  name: string;
  kind: PermissionOptionKind;
}

export interface DiffLine {
  kind: "hunk" | "context" | "added" | "removed";
  text: string;
}

export interface PermissionRequest {
  toolCallId: string;
  title: string;
  kind: string | null;
  target: string | null;
  /** The file it would change (absolute), if it's a file. */
  file: string | null;
  diff: DiffLine[] | null;
  options: PermissionOption[];
}

/** The Git drawer's view of a Worktree. */
export interface GitStatus {
  branch: string | null;
  upstream: string | null;
  ahead: number | null;
  behind: number | null;
  files: GitFile[];
  lastCommit: { id: string; subject: string } | null;
}

/** A changed file: git's status letters for what's staged and what isn't (`?`: untracked). */
export interface GitFile {
  /** Relative to the Worktree, `/`-separated. */
  path: string;
  staged: string | null;
  unstaged: string | null;
  renamedFrom: string | null;
  conflicted: boolean;
}

export interface CommitRequest {
  message: string;
  amend?: boolean;
  evenIfWorking?: boolean;
  evenIfPushed?: boolean;
}

export type CommitOutcome =
  | { kind: "committed"; id: string }
  /** Sessions in the Worktree are Working: ask first. */
  | { kind: "sessionsWorking"; sessions: string[] }
  /** Amending would rewrite a pushed commit: ask first. */
  | { kind: "alreadyPushed" };

/** What a push did. */
export type PushOutcome =
  | { kind: "pushed"; to: string }
  /** The remote has commits this branch hasn't. */
  | { kind: "rejected" };

/** What a pull did: it only ever fast-forwards. */
export type PullOutcome =
  /** Sessions in the Worktree are mid-turn: ask first. */
  | { kind: "sessionsWorking"; sessions: string[] }
  | { kind: "upToDate" }
  | { kind: "fastForwarded"; commits: number }
  /** A merge or rebase would be needed: left to an Agent or a terminal. */
  | { kind: "diverged"; ahead: number; behind: number };

/** A hand edit to a file the session read or edited, waiting for its next prompt. */
export interface EditNote {
  path: string;
  /** Relative to the session's Worktree. */
  name: string;
  added: number;
  removed: number;
  /** Null when it's too big: the note just says the file changed substantially. */
  diff: DiffLine[] | null;
}

/** A file open in a Manual editor, in some window. */
export interface OpenDocument {
  path: string;
  window: string;
  dirty: boolean;
  version: string;
}

export type PermissionOutcome = { kind: "selected"; optionId: string } | { kind: "cancelled" };

export type TranscriptItem =
  /** `editNotes`: the Edit notes sent with it ("a.rs (+1 −1)"). */
  | { kind: "user"; text: string; editNotes?: string[] }
  | { kind: "agent"; text: string }
  | { kind: "notice"; text: string }
  | { kind: "permission"; request: PermissionRequest; outcome: PermissionOutcome | null }
  | {
      kind: "toolCall";
      toolCallId: string;
      title: string;
      /** ACP's tool kind: read, edit, execute, … */
      toolKind: string | null;
      target: string | null;
      status: "pending" | "inProgress" | "completed" | "failed";
    };

/** A run of transcript items starting at absolute index `start`. */
export interface TranscriptPage {
  start: number;
  items: TranscriptItem[];
}

export type TranscriptDelta =
  /** The latest page: items `start..`. Indexes in the other deltas are absolute. */
  | { kind: "reset"; start: number; items: TranscriptItem[] }
  | { kind: "itemAdded"; index: number; item: TranscriptItem }
  | { kind: "textAppended"; index: number; text: string }
  | { kind: "itemUpdated"; index: number; item: TranscriptItem };

/** Why Idle sessions were Suspended automatically. */
export type AutoSuspendReason =
  | { kind: "memoryLimit"; usedBytes: number; limitBytes: number }
  | { kind: "lowMemory"; availableBytes: number }
  | { kind: "idle"; minutes: number };

export type CoreEvent =
  | { kind: "sessionCreated"; session: SessionInfo }
  | { kind: "sessionStateChanged"; sessionId: SessionId; state: SessionState }
  | { kind: "permissionModeChanged"; sessionId: SessionId; mode: PermissionMode }
  | { kind: "sessionUnreadChanged"; sessionId: SessionId; unread: number }
  | { kind: "worktreesChanged"; worktrees: WorktreeInfo[] }
  | { kind: "sessionClosed"; sessionId: SessionId }
  /** A popped-out window closed before it showed its file: `window` reopens it. */
  | { kind: "popOutReturned"; window: string; file: PoppedOutFile }
  /** A clean file open in `window` changed on disk: reload it. */
  | { kind: "documentChangedOnDisk"; window: string; path: string }
  /** A file with unsaved changes open in `window` changed on disk (or was deleted): ask. */
  | { kind: "documentConflicted"; window: string; path: string; deleted: boolean }
  /** A file `window` was told changed is back as the editor has it: nothing to ask. */
  | { kind: "documentBackOnDisk"; window: string; path: string }
  | { kind: "documentsChanged"; documents: OpenDocument[] }
  | { kind: "editNotesChanged"; sessionId: SessionId; notes: EditNote[] }
  /** The Worktree's git status may have changed (coalesced). */
  | { kind: "gitStatusChanged"; worktree: string }
  | { kind: "filesChanged"; worktree: string }
  | { kind: "fileWatchFallback"; worktree: string; message: string }
  | { kind: "autoSuspended"; suspended: { session: SessionInfo; reason: AutoSuspendReason }[] }
  | { kind: "recentSessionsChanged"; worktree: string; sessions: RecentSession[] }
  | { kind: "settingsChanged"; settings: LoadedSettings }
  | { kind: "setupChanged"; worktree: string; commands: string[]; status: SetupStatus }
  | { kind: "setupOutput"; worktree: string; text: string };

export const core = {
  prerequisites: () => invoke<MissingPrerequisite[]>("prerequisites"),
  defaultWorkspacePath: () => invoke<string | null>("default_workspace_path"),
  openWorkspace: (path: string) => invoke<WorkspaceInfo>("open_workspace", { path }),
  newSession: () => invoke<SessionId>("new_session"),
  newSessionIn: (worktree: string) => invoke<SessionId>("new_session_in", { worktree }),
  /** The Worktree being looked at (its files are indexed and watched). */
  showWorktree: (worktree: string) => invoke<void>("show_worktree", { worktree }),
  /** Ctrl+P: files best matching `query`, typos allowed. */
  findFiles: (worktree: string, query: string, limit: number) => invoke<FileMatch[]>("find_files", { worktree, query, limit }),
  /** A folder (`""` = the Worktree's root) for the Files drawer. */
  listDir: (worktree: string, dir: string) => invoke<DirEntry[]>("list_dir", { worktree, dir }),
  /** The open Tabs in order (after a restart, the restored ones, Suspended). */
  sessions: () => invoke<SessionInfo[]>("sessions"),
  /** Closes a Tab; its session goes to its Worktree's Recent sessions. */
  closeTab: (sessionId: SessionId) => invoke<void>("close_tab", { sessionId }),
  recentSessions: (worktree: string) => invoke<RecentSession[]>("recent_sessions", { worktree }),
  /** Reopens a Recent session in a new Tab, with its conversation. */
  reopenSession: (acpId: string) => invoke<SessionId>("reopen_session", { acpId }),
  /** `Ctrl+Shift+T`: the most recently closed session (null when there's none). */
  reopenLastClosed: () => invoke<SessionId | null>("reopen_last_closed"),
  /** The Tab shown last (before the restart, if restored). */
  lastActiveSession: () => invoke<SessionId | null>("last_active_session"),
  worktrees: () => invoke<WorktreeInfo[]>("worktrees"),
  refreshWorktrees: () => invoke<void>("refresh_worktrees"),
  createWorktree: (spec: NewWorktree) => invoke<CreatedWorktree>("create_worktree", { spec }),
  removalCheck: (worktree: string) => invoke<RemovalCheck>("removal_check", { worktree }),
  /** Stops the Worktree's sessions and setup, then removes it; refuses if that loses work without `discard`. */
  removeWorktree: (worktree: string, options: RemoveWorktree) => invoke<RemovedWorktree>("remove_worktree", { worktree, options }),
  /** Fetches first, so remote branches are current. */
  branches: () => invoke<BranchList>("branches"),
  suggestBranchName: () => invoke<string>("suggest_branch_name"),
  defaultStartPoint: () => invoke<string>("default_start_point"),
  settings: () => invoke<LoadedSettings>("settings"),
  /** Adds this repo's section to the settings file if need be; the file's path, to edit. */
  openRepoSettings: () => invoke<string>("open_repo_settings"),
  /** Tells the core this window's Manual editor opened, changed or closed a file. */
  documentOpened: (path: string, version: string) => invoke<void>("document_opened", { path, version }),
  documentChanged: (path: string, dirty: boolean) => invoke<void>("document_changed", { path, dirty }),
  documentClosed: (path: string) => invoke<void>("document_closed", { path }),
  /** The files open in Manual editors, in every window. */
  openDocuments: () => invoke<OpenDocument[]>("open_documents"),
  /** The diff from a Manual editor's `text` to the file on disk. */
  diffWithDisk: (path: string, text: string) => invoke<DiffLine[]>("diff_with_disk", { path, text }),
  /** If the file is open in another window, brings that window forward; whether it did. */
  showWhereOpen: (path: string) => invoke<boolean>("show_where_open", { path }),
  /** Moves a file into a window of its own; that window's label. */
  popOut: (file: PoppedOutFile) => invoke<string>("pop_out", { file }),
  /** In a popped-out window: its file (again after a reload). */
  collectPopOut: () => invoke<PoppedOutFile>("collect_pop_out"),
  /** A popped-out file as it is now (this window's, or window `label`'s). */
  updatePopOut: (label: string | null, text: string, savedText: string, version: string) =>
    invoke<void>("update_pop_out", { label, text, savedText, version }),
  /** Opens a file (a Worktree's, or the settings file) for the Manual editor. */
  readFile: (path: string) => invoke<OpenedFile>("read_file", { path }),
  /** Saves `text` ("\n" line endings, written as `lineEnding`) if `over` allows; refused with
   *  `changedOnDisk` if the file changed since it was read. */
  saveFile: (path: string, text: string, lineEnding: string, over: SaveOver) =>
    invoke<SaveOutcome>("save_file", { path, text, lineEnding, over }),
  /** Setups this editor ran or is running, for a view that opened after they started. */
  setups: () => invoke<SetupInfo[]>("setups"),
  /** Reruns a failed setup from the command that failed, as the settings file has it now. */
  retrySetup: (worktree: string) => invoke<void>("retry_setup", { worktree }),
  /** Skips the rest of a failed setup and starts the session. */
  startAnyway: (worktree: string) => invoke<void>("start_anyway", { worktree }),
  /** A Suspended session is resumed first. */
  sendPrompt: (sessionId: SessionId, text: string) => invoke<void>("send_prompt", { sessionId, text }),
  /** Stops an Idle session's Agent process to free memory; the conversation stays. */
  suspendSession: (sessionId: SessionId) => invoke<void>("suspend_session", { sessionId }),
  /** Brings a Suspended or Exited session back. */
  resumeSession: (sessionId: SessionId) => invoke<void>("resume_session", { sessionId }),
  gitStatus: (worktree: string) => invoke<GitStatus>("git_status", { worktree }),
  gitStage: (worktree: string, paths: string[]) => invoke<void>("git_stage", { worktree, paths }),
  gitUnstage: (worktree: string, paths: string[]) => invoke<void>("git_unstage", { worktree, paths }),
  gitStageAll: (worktree: string) => invoke<void>("git_stage_all", { worktree }),
  gitUnstageAll: (worktree: string) => invoke<void>("git_unstage_all", { worktree }),
  gitCommit: (worktree: string, request: CommitRequest) => invoke<CommitOutcome>("git_commit", { worktree, request }),
  gitFetch: (worktree: string) => invoke<void>("git_fetch", { worktree }),
  gitPush: (worktree: string) => invoke<PushOutcome>("git_push", { worktree }),
  /** Fast-forward only. */
  gitPull: (worktree: string, evenIfWorking = false) => invoke<PullOutcome>("git_pull", { worktree, evenIfWorking }),
  /** The window got focus: the core lists the Worktrees again, and fetches if it's due. */
  windowFocused: () => invoke<void>("window_focused"),
  /** Throws away every change to the file (ask first). */
  gitDiscard: (worktree: string, path: string) => invoke<void>("git_discard", { worktree, path }),
  /** The session's Edit notes (sent with its next prompt). */
  editNotes: (sessionId: SessionId) => invoke<EditNote[]>("edit_notes", { sessionId }),
  /** Don't tell the Agent about this hand edit. */
  removeEditNote: (sessionId: SessionId, path: string) => invoke<void>("remove_edit_note", { sessionId, path }),
  answerPermission: (sessionId: SessionId, toolCallId: string, optionId: string) =>
    invoke<void>("answer_permission", { sessionId, toolCallId, optionId }),
  setPermissionMode: (sessionId: SessionId, mode: PermissionMode) =>
    invoke<void>("set_permission_mode", { sessionId, mode }),
  /** Makes the session the visible Tab; its transcript streams to `onBatch` until another is shown. */
  showSession: (sessionId: SessionId, onBatch: (batch: TranscriptDelta[]) => void) => {
    const channel = new Channel<TranscriptDelta[]>();
    channel.onmessage = onBatch;
    return invoke<void>("show_session", { sessionId, onBatch: channel });
  },
  /** Up to a page of items just before index `before` (for scrolling back). */
  /** No Tab visible (an empty Worktree is selected): the previous Tab's stream ends. */
  hideTabs: () => invoke<void>("hide_tabs"),
  transcriptPageBefore: (sessionId: SessionId, before: number) =>
    invoke<TranscriptPage>("transcript_page_before", { sessionId, before }),
  /** True when launched by the memory benchmark (ticket 05). */
  benchMode: () => invoke<boolean>("bench_mode"),
  /** Records that the benchmark scenario reached `phase`. */
  benchMark: (phase: string) => invoke<void>("bench_mark", { phase }),
  onEvent: (handler: (event: CoreEvent) => void): Promise<UnlistenFn> =>
    listen<CoreEvent>("core-event", (e) => handler(e.payload)),
};
