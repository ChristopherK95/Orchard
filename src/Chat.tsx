// Pieces of the chat surface shared by the Tabs view and each column of the Columns view: the
// composer, banners, the Recent sessions list and the state labels.
import { createEffect, createResource, createSignal, For, type JSX, Match, on, onCleanup, onMount, Show, Switch } from "solid-js";
import { core, type OtherConversation, type PermissionMode, type RecentSession, type SessionId, type SessionInfo } from "./core";
import { EditNotes } from "./EditNotes";
import { createSlashMenu, SlashMenu, useSlashCommands } from "./SlashCommands";
import { ArrowUp, BookOpen, ChevronDown, CirclePlay, Info, Loader, ShieldQuestion, Sparkles, SquareTerminal, TriangleAlert, X, Zap } from "./icons";
import { worktreeColour } from "./Worktrees";

export { STATE_LABEL } from "./stateLabels";

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

/** Roughly how long ago `iso` was ("just now", "5 min ago", "3 days ago"). */
function ago(iso: string | null): string {
  const then = iso ? Date.parse(iso) : NaN;
  if (Number.isNaN(then)) return "";
  const min = Math.round((Date.now() - then) / 60_000);
  if (min < 1) return "just now";
  if (min < 60) return `${min} min ago`;
  const h = Math.round(min / 60);
  if (h < 24) return `${h} h ago`;
  const d = Math.round(h / 24);
  return d < 60 ? `${d} day${d === 1 ? "" : "s"} ago` : new Date(then).toLocaleDateString();
}

const OPEN_HINT = "Open in a Tab, with its conversation. If it's still running in a terminal, exit it there first: both would write to the same conversation.";

/** The Agent's conversations in `worktree` that aren't Tabs or Recent sessions (started in a
 *  terminal, say), looked up when shown: as items of the Recent menu, or as a list box. */
export function OtherConversations(props: { worktree: string; inMenu?: boolean; onOpen: (c: OtherConversation) => void }) {
  const [list] = createResource(() => props.worktree, (w) => core.otherConversations(w));
  const title = (c: OtherConversation) => c.title ?? "Untitled conversation";
  const status = (text: string) =>
    props.inMenu ? <div class="menu-note">{text}</div> : <div class="recent-row"><span class="grow note">{text}</span></div>;
  const body = (
    <Switch>
      <Match when={list.loading}>
        {props.inMenu ? (
          <div class="menu-note">
            <Loader class="spin" /> Looking…
          </div>
        ) : (
          <div class="recent-row">
            <Loader class="spin" />
            <span class="grow note">Looking…</span>
          </div>
        )}
      </Match>
      <Match when={list.error}>{status(`Couldn't list them: ${String(list.error)}`)}</Match>
      <Match when={list()?.length === 0}>{status("None in this Worktree")}</Match>
      <Match when={list()}>
        <For each={list()}>
          {(c) =>
            props.inMenu ? (
              <button class="menu-item" onClick={() => props.onOpen(c)} title={OPEN_HINT}>
                <SquareTerminal />
                <span class="grow ellipsis">{title(c)}</span>
                <span class="when">{ago(c.updatedAt)}</span>
              </button>
            ) : (
              <div class="recent-row">
                <SquareTerminal />
                <span class="grow ellipsis">{title(c)}</span>
                <span class="when">{ago(c.updatedAt)}</span>
                <button class="link" onClick={() => props.onOpen(c)} title={OPEN_HINT}>
                  Open
                </button>
              </div>
            )
          }
        </For>
      </Match>
    </Switch>
  );
  return props.inMenu ? (
    <>
      <div class="menu-label">Started elsewhere</div>
      {body}
    </>
  ) : (
    <div class="list-box">
      <div class="section-label">
        Started elsewhere
        <span class="note">Claude Code conversations in this Worktree, e.g. from a terminal</span>
      </div>
      {body}
    </div>
  );
}

/** "Started elsewhere" in an empty Worktree: looked up only once asked for (it starts the Agent). */
export function FindOtherConversations(props: { worktree: string; onOpen: (c: OtherConversation) => void }) {
  const [shown, setShown] = createSignal(false);
  createEffect(on(() => props.worktree, () => setShown(false), { defer: true }));
  return (
    <Show
      when={shown()}
      fallback={
        <button class="link" onClick={() => setShown(true)} title="Claude Code conversations in this Worktree that weren't started here, e.g. in a terminal">
          Conversations started elsewhere…
        </button>
      }
    >
      <OtherConversations worktree={props.worktree} onOpen={props.onOpen} />
    </Show>
  );
}

export /** One line between the context strip and the content: a standing fact, a notice or an error. */
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

/** Reports an element's height as it changes (the floating composer's, for the transcript). */
export function reportHeight(el: HTMLElement, onHeight?: (px: number) => void) {
  if (!onHeight) return;
  const observer = new ResizeObserver(() => onHeight(el.offsetHeight));
  observer.observe(el);
  onCleanup(() => observer.disconnect());
}

export function Composer(props: { session: SessionInfo; onHeight?: (px: number) => void }) {
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
    <div class="composer" ref={(el) => reportHeight(el, props.onHeight)} style={{ "--c": worktreeColour(props.session.worktree) }}>
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
                <button class="primary send icon-only" onClick={send} disabled={disabled() || !text().trim()} title="Send (Enter)" aria-label="Send">
                  <ArrowUp />
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
