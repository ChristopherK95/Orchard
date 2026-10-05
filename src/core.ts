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

/** One of the Recent Workspaces, as the Workspace picker lists it. */
export interface RecentWorkspace {
  root: string;
  name: string;
  /** When it was last opened (ms since the Unix epoch); null if before this was recorded. */
  opened: number | null;
  /** How many Tabs opening it restores. */
  sessions: number;
  /** Its folder is there (one that isn't can't be opened). */
  exists: boolean;
  /** Byte offsets into `root` of the characters the filter matched. */
  indices: number[];
}

/** A running shell (a Worktree can have several), numbered from 0 per Workspace. */
export interface TerminalInfo {
  id: number;
  worktree: string;
  /** What it's running: the foreground command where that can be told (Linux), else the shell. */
  name: string;
  /** Something other than the shell is in the foreground (Linux only). */
  busy: boolean;
}

/** What a Worktree's terminal sends the Terminal panel. */
export type TerminalOutput =
  | { kind: "output"; text: string }
  /** What it printed before the panel came back to it: its queries were answered then. */
  | { kind: "replay"; text: string }
  | { kind: "exited"; code: number | null };

/** The Tabs view's view slot (each Columns view column has its own). */
export const TABS_SLOT = "tabs";

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

/** A conversation the Agent has for a Worktree that the editor doesn't (started in a terminal…). */
export interface OtherConversation {
  /** The ACP session id, which `openConversation` takes. */
  acpId: string;
  /** What the Agent calls it (its summary or first prompt). */
  title: string | null;
  worktree: string;
  /** When it last changed (ISO 8601). */
  updatedAt: string | null;
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
  /** Merged into the Base (or its changes are, after a squash or rebase merge), or nothing of its own:
   *  "Delete branch too" starts ticked, and deleting the branch loses nothing. */
  merged: boolean;
  /** Where: the Base, or another branch its PR went into (`origin/project`). Null if not merged. */
  mergedInto: string | null;
  /** Removing it at all needs "Discard and remove". */
  discardToRemove: boolean;
  /** Deleting the branch too needs "Discard and remove". */
  discardToDeleteBranch: boolean;
  /** Identifies exactly the work listed; "Discard and remove" sends it back. */
  fingerprint: string;
}

/** How a Worktree's branch stands against its Base, or another branch its PR went into. */
export type MergeState =
  /** Its commits are in `into` (a merge commit, or a fast-forward into the Base). */
  | { kind: "merged"; into: string }
  /** Its changes are in `into` under other commits: a squash or rebase merge. */
  | { kind: "changesInBase"; into: string }
  /** No commits of its own yet. */
  | { kind: "nothingNew" }
  /** Not found in the Base, but its remote branch was deleted (the PR merged some other way, or closed). */
  | { kind: "remoteDeleted"; commits: number }
  | { kind: "notMerged"; commits: number };

/** One row of the Worktrees overview. */
export interface WorktreeMerge {
  path: string;
  branch: string | null;
  isMain: boolean;
  /** The Worktree's Base, which it's measured against. */
  base: string;
  /** Null for the main checkout, and when it couldn't be told (`error`). */
  merge: MergeState | null;
  error: string | null;
  /** Uncommitted changes (removal loses them whatever `merge` says). */
  changed: number;
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

export interface Appearance {
  /** null: the bundled default (Geist / Geist Mono). */
  uiFont: string | null;
  codeFont: string | null;
  codeFontSize: number;
  ligatures: boolean;
}

export interface FontFamily {
  name: string;
  monospace: boolean;
}

export interface Settings {
  appearance: Appearance;
  editor: { vim: boolean };
  notifications: { turnFinished: boolean };
  agents: { memoryLimitMb: number; idleSuspend: boolean; idleSuspendMinutes: number };
}

export type WindowsShell = "powershell" | "git-bash";

/** One repo's section of the settings file. */
export interface RepoSettings {
  /** Worktree setup commands, run in order in each new Worktree. */
  setup: string[];
  /** Replaces `setup` on Windows / Linux, when set. */
  setupWindows: string[] | null;
  setupLinux: string[] | null;
  windowsShell: WindowsShell;
}

/** One change the settings page makes (repo ones are to the open repo's section). */
export type SettingChange =
  | { kind: "uiFont"; family: string | null }
  | { kind: "codeFont"; family: string | null }
  | { kind: "codeFontSize"; px: number }
  | { kind: "ligatures"; on: boolean }
  | { kind: "vim"; on: boolean }
  | { kind: "turnFinished"; on: boolean }
  | { kind: "memoryLimitMb"; mb: number }
  | { kind: "idleSuspend"; on: boolean }
  | { kind: "idleSuspendMinutes"; minutes: number }
  | { kind: "setup"; commands: string[] }
  | { kind: "setupWindows"; commands: string[] | null }
  | { kind: "setupLinux"; commands: string[] | null }
  | { kind: "windowsShell"; shell: WindowsShell };

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
  /** A merge, rebase, cherry-pick or revert in progress; its conflicted files are the `conflicted` ones. */
  operation: "merge" | "rebase" | "cherryPick" | "revert" | "am" | null;
  /** Sessions in this Worktree in the middle of a turn (a branch switch waits for them). */
  midTurn: string[];
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

/** "Changes vs base": what the branch changed since it split from its Base. */
export interface BaseChanges {
  /** The Worktree's Base: the default (`origin/<default>`), its start point, or one set for it. */
  base: string;
  isDefault: boolean;
  /** Where the branch split from it: what each file's diff runs from. */
  split: string;
  files: BaseChange[];
}

export interface BaseChange {
  /** Relative to the Worktree. */
  path: string;
  change: "added" | "modified" | "deleted" | "renamed";
  renamedFrom: string | null;
}

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

/** A slash command the Agent offers (Claude Code's own, a custom command or a skill); typing
 *  `/name args` as the prompt runs it. */
export interface SlashCommand {
  name: string;
  description: string;
  /** What to type after it, if it takes input. */
  hint: string | null;
}

export type CoreEvent =
  | { kind: "sessionCreated"; session: SessionInfo }
  | { kind: "sessionStateChanged"; sessionId: SessionId; state: SessionState }
  | { kind: "permissionModeChanged"; sessionId: SessionId; mode: PermissionMode }
  | { kind: "sessionUnreadChanged"; sessionId: SessionId; unread: number }
  /** The slash commands (and skills) the session's Agent offers: the whole list. */
  | { kind: "availableCommandsChanged"; sessionId: SessionId; commands: SlashCommand[] }
  | { kind: "worktreesChanged"; worktrees: WorktreeInfo[] }
  /** A shell started or went: every running shell, oldest first. */
  | { kind: "terminalsChanged"; terminals: TerminalInfo[] }
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
  /** Whether an installed app still has to fetch the ACP adapter (its first run). */
  adapterMissing: () => invoke<boolean>("adapter_missing"),
  /** Fetches the pinned ACP adapter with npm; rejects with npm's error. */
  installAdapter: () => invoke<void>("install_adapter"),
  defaultWorkspacePath: () => invoke<string | null>("default_workspace_path"),
  /** Opens the repository containing `path` (a leading `~` is the home folder). */
  openWorkspace: (path: string) => invoke<WorkspaceInfo>("open_workspace", { path }),
  /** The Workspace opening `path` would open, without opening it. */
  workspaceFor: (path: string) => invoke<WorkspaceInfo>("workspace_for", { path }),
  /** Closes every popped-out Manual editor window, unsaved changes or not. */
  closePopOuts: () => invoke<void>("close_pop_outs"),
  /** The Workspace picker's list: the Recent Workspaces matching `query` (all, newest first, if empty). */
  recentWorkspaces: (query: string) => invoke<RecentWorkspace[]>("recent_workspaces", { query }),
  /** Takes a Workspace off the Recent Workspaces; its saved Tabs stay. */
  removeRecentWorkspace: (root: string) => invoke<void>("remove_recent_workspace", { root }),
  /** The native folder dialog; null if it was cancelled. */
  pickFolder: () => invoke<string | null>("pick_folder"),
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
  /** The Agent's conversations in a Worktree that aren't Tabs or Recent sessions, newest first. */
  otherConversations: (worktree: string) => invoke<OtherConversation[]>("other_conversations", { worktree }),
  /** Opens one of `otherConversations` in a new Tab (or shows its Tab, if it's one already). */
  openConversation: (c: OtherConversation) =>
    invoke<SessionId>("open_conversation", { worktree: c.worktree, acpId: c.acpId, title: c.title }),
  /** `Ctrl+Shift+T`: the most recently closed session (null when there's none). */
  reopenLastClosed: () => invoke<SessionId | null>("reopen_last_closed"),
  /** The Tab shown last (before the restart, if restored). */
  lastActiveSession: () => invoke<SessionId | null>("last_active_session"),
  worktrees: () => invoke<WorktreeInfo[]>("worktrees"),
  refreshWorktrees: () => invoke<void>("refresh_worktrees"),
  createWorktree: (spec: NewWorktree) => invoke<CreatedWorktree>("create_worktree", { spec }),
  /** Every Worktree, main checkout first, with whether its branch is merged into its Base (as last fetched). */
  mergeOverview: () => invoke<WorktreeMerge[]>("merge_overview"),
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
  /** The fonts installed on this machine (found once per run). */
  installedFonts: () => invoke<FontFamily[]>("installed_fonts"),
  /** The open repo's settings (its defaults if the file has no section for it). */
  repoSettings: () => invoke<RepoSettings>("repo_settings"),
  /** Writes one change from the settings page to the settings file; the settings now in force. */
  changeSetting: (change: SettingChange) => invoke<LoadedSettings>("change_setting", { change }),
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
  /** Switches the Worktree to another branch (local, or remote: it gets a local one tracking it). */
  switchBranch: (worktree: string, branch: string) => invoke<void>("switch_branch", { worktree, branch }),
  /** What the branch changed since it split from its Base. */
  changesVsBase: (worktree: string) => invoke<BaseChanges>("changes_vs_base", { worktree }),
  /** The Worktree's Base from now on (null: the default). */
  setBase: (worktree: string, base: string | null) => invoke<void>("set_base", { worktree, base }),
  /** One file's change since the split (as the list gave it). */
  diffVsBase: (worktree: string, split: string, file: BaseChange) =>
    invoke<DiffLine[]>("diff_vs_base", { worktree, split, path: file.path, renamedFrom: file.renamedFrom, change: file.change }),
  /** Aborts the merge, rebase, cherry-pick or revert in progress. */
  abortOperation: (worktree: string) => invoke<void>("abort_operation", { worktree }),
  /** Throws away every change to the file (ask first). */
  gitDiscard: (worktree: string, path: string) => invoke<void>("git_discard", { worktree, path }),
  /** The session's Edit notes (sent with its next prompt). */
  editNotes: (sessionId: SessionId) => invoke<EditNote[]>("edit_notes", { sessionId }),
  /** Don't tell the Agent about this hand edit. */
  removeEditNote: (sessionId: SessionId, path: string) => invoke<void>("remove_edit_note", { sessionId, path }),
  /** The oldest permission card waiting in the session (for the Board). */
  pendingPermission: (sessionId: SessionId) => invoke<PermissionRequest | null>("pending_permission", { sessionId }),
  answerPermission: (sessionId: SessionId, toolCallId: string, optionId: string) =>
    invoke<void>("answer_permission", { sessionId, toolCallId, optionId }),
  setPermissionMode: (sessionId: SessionId, mode: PermissionMode) =>
    invoke<void>("set_permission_mode", { sessionId, mode }),
  /** Makes the session the visible Tab of view `slot` (the Tabs view's by default, or a column's);
   *  its transcript streams to `onBatch` until another is shown there. Slots stream side by side. */
  showSession: (sessionId: SessionId, onBatch: (batch: TranscriptDelta[]) => void, slot: string = TABS_SLOT) => {
    const channel = new Channel<TranscriptDelta[]>();
    channel.onmessage = onBatch;
    return invoke<void>("show_session", { sessionId, slot, onBatch: channel });
  },
  /** No Tab visible in `slot` (an empty Worktree is selected there, or a column went away): the
   *  Tab it showed stops streaming. */
  hideTabs: (slot: string = TABS_SLOT) => invoke<void>("hide_tabs", { slot }),
  /** The Worktrees pinned as columns of the Columns view. */
  pinnedWorktrees: () => invoke<string[]>("pinned_worktrees"),
  /** The slash commands the session's Agent last said it offers (none before its process starts). */
  availableCommands: (sessionId: SessionId) => invoke<SlashCommand[]>("available_commands", { sessionId }),
  /** Pins or unpins a Worktree as a column; resolves to the pinned Worktrees now. */
  setPinned: (worktree: string, pinned: boolean) => invoke<string[]>("set_pinned", { worktree, pinned }),
  /** The columns' widths, as shares of the row (1000 each when even); a column without one has
   *  1000. Pinning or unpinning evens them out (clears them). */
  columnShares: () => invoke<Record<string, number>>("column_shares"),
  setColumnShares: (shares: Record<string, number>) => invoke<void>("set_column_shares", { shares }),
  /** Up to a page of items just before index `before` (for scrolling back). */
  transcriptPageBefore: (sessionId: SessionId, before: number) =>
    invoke<TranscriptPage>("transcript_page_before", { sessionId, before }),
  /** Every running shell, oldest first. */
  terminals: () => invoke<TerminalInfo[]>("terminals"),
  /** Shows a shell of the Worktree in view slot `slot`'s Terminal panel: `id` if it still runs
   *  there, else the Worktree's oldest, else a new one (`cols` x `rows`). Its output streams to
   *  `onOutput`; whatever the slot showed before stops sending there. */
  openTerminal: (slot: string, worktree: string, id: number | null, cols: number, rows: number, onOutput: (batch: TerminalOutput[]) => void) => {
    const channel = new Channel<TerminalOutput[]>();
    channel.onmessage = onOutput;
    return invoke<TerminalInfo>("open_terminal", { slot, worktree, id, cols, rows, onOutput: channel });
  },
  /** Starts another shell in the Worktree and shows it in view slot `slot`'s Terminal panel. */
  newTerminal: (slot: string, worktree: string, cols: number, rows: number, onOutput: (batch: TerminalOutput[]) => void) => {
    const channel = new Channel<TerminalOutput[]>();
    channel.onmessage = onOutput;
    return invoke<TerminalInfo>("new_terminal", { slot, worktree, cols, rows, onOutput: channel });
  },
  /** Types keystrokes (or a paste) into a shell. */
  terminalInput: (id: number, data: string) => invoke<void>("terminal_input", { id, data }),
  resizeTerminal: (id: number, cols: number, rows: number) => invoke<void>("resize_terminal", { id, cols, rows }),
  /** View slot `slot`'s Terminal panel is hidden; the shells keep running. */
  hideTerminal: (slot: string) => invoke<void>("hide_terminal", { slot }),
  /** Stops a shell and everything it started. */
  closeTerminal: (id: number) => invoke<void>("close_terminal", { id }),
  /** True when launched by the memory benchmark (ticket 05). */
  benchMode: () => invoke<boolean>("bench_mode"),
  /** Records that the benchmark scenario reached `phase`. */
  benchMark: (phase: string) => invoke<void>("bench_mark", { phase }),
  onEvent: (handler: (event: CoreEvent) => void): Promise<UnlistenFn> =>
    listen<CoreEvent>("core-event", (e) => handler(e.payload)),
};
