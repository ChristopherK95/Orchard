// Typed access to the Rust core over Tauri IPC. The shapes mirror editor-core's serde types.
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type SessionId = number;
export type SessionState = "working" | "needsYou" | "idle" | "exited";
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
  notifications: { turnFinished: boolean };
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
  diff: DiffLine[] | null;
  options: PermissionOption[];
}

export type PermissionOutcome = { kind: "selected"; optionId: string } | { kind: "cancelled" };

export type TranscriptItem =
  | { kind: "user"; text: string }
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

export type CoreEvent =
  | { kind: "sessionCreated"; session: SessionInfo }
  | { kind: "sessionStateChanged"; sessionId: SessionId; state: SessionState }
  | { kind: "permissionModeChanged"; sessionId: SessionId; mode: PermissionMode }
  | { kind: "sessionUnreadChanged"; sessionId: SessionId; unread: number }
  | { kind: "worktreesChanged"; worktrees: WorktreeInfo[] }
  | { kind: "settingsChanged"; settings: LoadedSettings }
  | { kind: "setupChanged"; worktree: string; commands: string[]; status: SetupStatus }
  | { kind: "setupOutput"; worktree: string; text: string };

export const core = {
  prerequisites: () => invoke<MissingPrerequisite[]>("prerequisites"),
  defaultWorkspacePath: () => invoke<string | null>("default_workspace_path"),
  openWorkspace: (path: string) => invoke<WorkspaceInfo>("open_workspace", { path }),
  newSession: () => invoke<SessionId>("new_session"),
  newSessionIn: (worktree: string) => invoke<SessionId>("new_session_in", { worktree }),
  worktrees: () => invoke<WorktreeInfo[]>("worktrees"),
  refreshWorktrees: () => invoke<void>("refresh_worktrees"),
  createWorktree: (spec: NewWorktree) => invoke<CreatedWorktree>("create_worktree", { spec }),
  /** Fetches first, so remote branches are current. */
  branches: () => invoke<BranchList>("branches"),
  suggestBranchName: () => invoke<string>("suggest_branch_name"),
  defaultStartPoint: () => invoke<string>("default_start_point"),
  settings: () => invoke<LoadedSettings>("settings"),
  /** Adds this repo's section to the settings file if need be, and opens the file. */
  openRepoSettings: () => invoke<void>("open_repo_settings"),
  /** Setups this editor ran or is running, for a view that opened after they started. */
  setups: () => invoke<SetupInfo[]>("setups"),
  /** Reruns a failed setup from the command that failed, as the settings file has it now. */
  retrySetup: (worktree: string) => invoke<void>("retry_setup", { worktree }),
  /** Skips the rest of a failed setup and starts the session. */
  startAnyway: (worktree: string) => invoke<void>("start_anyway", { worktree }),
  sendPrompt: (sessionId: SessionId, text: string) => invoke<void>("send_prompt", { sessionId, text }),
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
