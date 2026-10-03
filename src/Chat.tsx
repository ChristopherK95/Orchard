// Pieces of the chat surface shared by the Tabs view and each column of the Columns view: the
// composer, banners, the Recent sessions list and the state labels.
import { createEffect, createSignal, For, type JSX, Match, onMount, Show, Switch } from "solid-js";
import { core, type PermissionMode, type RecentSession, type SessionId, type SessionInfo, type SessionState } from "./core";
import { EditNotes } from "./EditNotes";
import { createSlashMenu, SlashMenu, useSlashCommands } from "./SlashCommands";
import { BookOpen, ChevronDown, CirclePlay, Info, ShieldQuestion, Sparkles, TriangleAlert, X, Zap } from "./icons";
import { worktreeColour } from "./Worktrees";

export const STATE_LABEL: Record<SessionState, string> = {
  working: "Working",
  needsYou: "Needs you",
  idle: "Idle",
  suspended: "Suspended",
  exited: "Exited",
};

export function RecentList(props: { sessions: RecentSession[]; onReopen: (session: RecentSession) => void }) {
  return (
    <div class="list-box">
      <div class="section-label">
        Recent sessions
        <span class="note">closing a Tab is never destructive</span>
      </div>
      <For each={props.sessions}>
        {(s) => (
          <div class="recent-row">
            <Sparkles />
            <span class="grow">{s.name}</span>
            <button class="link" onClick={() => props.onReopen(s)} title="Reopen with its conversation">
              Reopen
            </button>
          </div>
        )}
      </For>
    </div>
  );
}

export /** One line between the context bar and the content: a standing fact, a notice or an error. */
function Banner(props: { tone: "warn" | "info" | "error"; children: JSX.Element; action?: JSX.Element; onDismiss?: () => void }) {
  return (
    <div class={`banner ${props.tone}`} role={props.tone === "error" ? "alert" : "status"}>
      {props.tone === "info" ? <Info /> : <TriangleAlert />}
      <span class="text">{props.children}</span>
      {props.action}
      <Show when={props.onDismiss}>
        <button class="dismiss" onClick={() => props.onDismiss!()} aria-label="Dismiss" title="Dismiss">
          <X />
        </button>
      </Show>
    </div>
  );
}

const MODE_LABEL: Record<PermissionMode, string> = {
  askForEdits: "Ask for edits",
  acceptEdits: "Accept edits",
  plan: "Plan",
};

/** Unsent composer text by session. */
const drafts = new Map<SessionId, string>();

export function Composer(props: { session: SessionInfo }) {
  // The draft outlives the composer: it moves between columns with focus, and Tabs get switched.
  const [text, setDraft] = createSignal(drafts.get(props.session.id) ?? "");
  const setText = (value: string) => {
    setDraft(value);
    if (value) drafts.set(props.session.id, value);
    else drafts.delete(props.session.id);
  };
  const [error, setError] = createSignal("");
  // Typing `/` lists the Agent's slash commands and skills.
  const commands = useSlashCommands(() => props.session);
  const slash = createSlashMenu(text, commands);
  const complete = (name: string) => {
    setText(`/${name} `);
    input.focus();
    input.setSelectionRange(input.value.length, input.value.length);
  };
  // A Suspended session takes a message too: sending it resumes the session.
  const disabled = () => props.session.state !== "idle" && props.session.state !== "suspended";
  let input!: HTMLTextAreaElement;
  onMount(() => input.focus());
  // Back to the composer once a turn (or a permission question) is over.
  createEffect(() => props.session.state === "idle" && input.focus());

  const setMode = async (select: HTMLSelectElement) => {
    setError("");
    try {
      await core.setPermissionMode(props.session.id, select.value as PermissionMode);
    } catch (err) {
      setError(String(err));
      select.value = props.session.permissionMode; // back to the mode it's really in
    }
  };

  const resume = async () => {
    setError("");
    try {
      await core.resumeSession(props.session.id);
    } catch (err) {
      setError(String(err));
    }
  };

  const send = async () => {
    const prompt = text().trim();
    if (!prompt || disabled()) return;
    setError("");
    try {
      await core.sendPrompt(props.session.id, prompt);
      setText("");
      input.focus();
    } catch (err) {
      setError(String(err));
    }
  };

  const locked = () => props.session.state === "exited" || props.session.state === "needsYou";

  return (
    <div class="composer" style={{ "--c": worktreeColour(props.session.worktree) }}>
      <EditNotes sessionId={props.session.id} />
      <Show when={error()}>
        <p class="error">
          <TriangleAlert />
          {error()}
        </p>
      </Show>
      <div class="composer-box" classList={{ locked: locked() }}>
        <Show when={slash.open() && !locked()}>
          <SlashMenu
            items={slash.items()}
            selected={slash.selected()}
            waiting={commands().length === 0}
            onHover={slash.setSelected}
            onPick={(command) => complete(command.name)}
          />
        </Show>
        <div class="composer-inner">
          <textarea
            ref={input}
            value={text()}
            onInput={(e) => setText(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (slash.key(e, (command) => complete(command.name))) return;
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                void send();
              }
            }}
            placeholder={
              props.session.state === "exited"
                ? "The Agent exited."
                : props.session.state === "needsYou"
                  ? "Answer the permission card above first (Y / N)."
                  : "Message the Agent (Enter to send, Shift+Enter for a newline, / for commands)"
            }
            disabled={locked()}
          />
          <div class="composer-bar">
            <label class={`mode-pill ${props.session.permissionMode}`} title="Permission mode">
              <Switch fallback={<ShieldQuestion />}>
                <Match when={props.session.permissionMode === "acceptEdits"}>
                  <Zap />
                </Match>
                <Match when={props.session.permissionMode === "plan"}>
                  <BookOpen />
                </Match>
              </Switch>
              {MODE_LABEL[props.session.permissionMode]}
              <ChevronDown class="chev" />
              <select value={props.session.permissionMode} onChange={(e) => void setMode(e.currentTarget)}>
                <For each={Object.keys(MODE_LABEL) as PermissionMode[]}>{(mode) => <option value={mode}>{MODE_LABEL[mode]}</option>}</For>
              </select>
            </label>
            <Show when={props.session.state === "suspended"}>
              <span class="hint">Sending resumes this session</span>
            </Show>
            <Show when={props.session.state === "working"}>
              <span class="hint">The Agent is working; send once its turn is done.</span>
            </Show>
            <span class="grow" />
            <Show
              when={props.session.state === "exited"}
              fallback={
                <button class="primary send" onClick={send} disabled={disabled() || !text().trim()}>
                  Send <kbd>Enter</kbd>
                </button>
              }
            >
              <button class="primary send" onClick={() => void resume()} title="Bring the Agent back with this conversation">
                <CirclePlay />
                Resume
              </button>
            </Show>
          </div>
        </div>
      </div>
    </div>
  );
}
