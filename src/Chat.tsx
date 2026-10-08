// Pieces of the chat surface shared by the Tabs view and each column of the Columns view: the
// composer, banners, the Recent sessions list and the state labels.
import { createEffect, createResource, createSignal, For, type JSX, Match, on, onCleanup, onMount, Show, Switch } from "solid-js";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { type Attachment, core, type OtherConversation, type PermissionMode, type QueuedPrompt, type RecentSession, type SessionId, type SessionInfo } from "./core";
import { EditNotes } from "./EditNotes";
import { createSlashMenu, SlashMenu, useSlashCommands } from "./SlashCommands";
import {
  ArrowUp,
  BookOpen,
  ChevronDown,
  CirclePlay,
  Clock,
  FileText,
  ImageIcon,
  Info,
  Loader,
  Paperclip,
  ShieldQuestion,
  Sparkles,
  Square,
  SquareTerminal,
  TriangleAlert,
  X,
  Zap,
} from "./icons";
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
export function ago(iso: string | null): string {
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
  auto: "Auto",
};

/** Unsent composer text and attachments by session. */
const drafts = new Map<SessionId, { text: string; attachments: Attachment[] }>();

/** Reports an element's height as it changes (the floating composer's, for the transcript). */
export function reportHeight(el: HTMLElement, onHeight?: (px: number) => void) {
  if (!onHeight) return;
  const observer = new ResizeObserver(() => onHeight(el.offsetHeight));
  observer.observe(el);
  onCleanup(() => observer.disconnect());
}

export function Composer(props: { session: SessionInfo; onHeight?: (px: number) => void }) {
  // The draft outlives the composer: it moves between columns with focus, and Tabs get switched.
  const saved = drafts.get(props.session.id);
  const [text, setDraft] = createSignal(saved?.text ?? "");
  const [attachments, setRawAttachments] = createSignal<Attachment[]>(saved?.attachments ?? []);
  const keepDraft = () => {
    if (text() || attachments().length) drafts.set(props.session.id, { text: text(), attachments: attachments() });
    else drafts.delete(props.session.id);
  };
  const setText = (value: string) => {
    setDraft(value);
    keepDraft();
  };
  const setAttachments = (list: Attachment[]) => {
    setRawAttachments(list);
    keepDraft();
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
  const working = () => props.session.state === "working";
  // Mid-turn: a message queues, and Stop is offered.
  const busy = () => working() || props.session.state === "needsYou";
  // A Suspended session takes a message too: sending it resumes the session. A Working one queues it.
  const disabled = () => props.session.state !== "idle" && props.session.state !== "suspended" && !working();
  let input!: HTMLTextAreaElement;
  let root!: HTMLDivElement;
  onMount(() => input.focus());
  // Back to the composer once a turn (or a permission question) is over.
  createEffect(() => props.session.state === "idle" && input.focus());

  // The queued message lives in the core (it outlives this composer); this shows it.
  const [queued, setQueued] = createSignal<QueuedPrompt | null>(null);
  /** An event came since the queued message was asked for: it's newer than the reply. */
  let heardQueued = false;
  let alive = true;
  let unlisten: (() => void)[] = [];
  onCleanup(() => {
    alive = false;
    unlisten.forEach((stop) => stop());
  });
  const listening = (promise: Promise<() => void>) => void promise.then((stop) => (alive ? unlisten.push(stop) : stop()));
  listening(
    core.onEvent((event) => {
      if (event.kind !== "queuedPromptChanged" || event.sessionId !== props.session.id) return;
      heardQueued = true;
      setQueued(event.queued);
    }),
  );
  void core
    .queuedPrompt(props.session.id)
    .then((q) => !heardQueued && setQueued(q))
    .catch(() => {});
  /** Takes the queued message back into the input (ahead of anything typed since), to edit it. */
  const takeBack = async () => {
    try {
      const q = await core.takeQueuedPrompt(props.session.id);
      if (!q) return;
      setText([q.text, text()].filter((t) => t.trim()).join("\n\n"));
      setAttachments([...q.attachments, ...attachments()]);
      input.focus();
    } catch (err) {
      setError(String(err));
    }
  };
  const unqueue = () => void core.takeQueuedPrompt(props.session.id).catch((err) => setError(String(err)));
  // A turn that ended without sending it (stopped, failed, or the Agent exited): it comes back here.
  createEffect(() => !busy() && queued() && void takeBack());

  /** Adds attachments, showing why any couldn't be attached. */
  const attach = async (loads: (() => Promise<Attachment>)[]) => {
    setError("");
    const refused: string[] = [];
    for (const load of loads) {
      try {
        const attachment = await load();
        setAttachments([...attachments(), attachment]);
      } catch (err) {
        refused.push(String(err));
      }
    }
    setError(refused.join(" · "));
  };
  const attachPaths = (paths: string[]) => attach(paths.map((path) => () => core.attachFile(props.session.id, path)));
  const pickFiles = async () => {
    try {
      await attachPaths(await core.pickFiles());
    } catch (err) {
      setError(String(err));
    }
    input.focus();
  };
  const onPaste = (e: ClipboardEvent) => {
    const files = [...(e.clipboardData?.files ?? [])];
    // (Text that comes with a picture of itself, from an office app say, is pasted as text.)
    if (e.clipboardData?.getData("text/plain")) return;
    e.preventDefault();
    if (files.length) {
      void attach(files.map((file) => async () => core.attachData(props.session.id, file.name || "pasted image", file.type || null, await base64(file))));
      return;
    }
    // Nothing the page can read (WebKitGTK doesn't hand it an image copied from another app): the
    // image, if there is one, is read off the clipboard natively.
    setError("");
    void core
      .attachClipboardImage(props.session.id)
      .then((image) => image && setAttachments([...attachments(), image]))
      .catch((err) => setError(String(err)));
  };
  // Files dropped on the composer (the webview reports drops itself, with their paths).
  const [dropping, setDropping] = createSignal(false);
  /** Whether a drag at `position` (physical pixels in the webview) is over the composer. */
  const overComposer = (position: { x: number; y: number }) => {
    const r = root.getBoundingClientRect();
    const [x, y] = [position.x / window.devicePixelRatio, position.y / window.devicePixelRatio];
    return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
  };
  listening(
    getCurrentWebview().onDragDropEvent(({ payload }) => {
      if (payload.type === "leave") return setDropping(false);
      const here = overComposer(payload.position) && props.session.state !== "exited";
      if (payload.type !== "drop") return setDropping(here);
      setDropping(false);
      if (here) void attachPaths(payload.paths).then(() => input.focus());
    }),
  );

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

  const stop = async () => {
    setError("");
    try {
      await core.cancelTurn(props.session.id);
    } catch (err) {
      setError(String(err));
    }
  };

  const ready = () => !!text().trim() || attachments().length > 0;
  const send = async () => {
    const prompt = text().trim();
    const files = attachments();
    if (!ready() || disabled()) return;
    setError("");
    try {
      if (working()) await core.queuePrompt(props.session.id, prompt, files);
      else await core.sendPrompt(props.session.id, prompt, files);
      setText("");
      setAttachments([]);
      input.focus();
    } catch (err) {
      setError(String(err));
    }
  };

  const locked = () => props.session.state === "exited" || props.session.state === "needsYou";

  return (
    <div class="composer" ref={(el) => ((root = el), reportHeight(el, props.onHeight))} style={{ "--c": worktreeColour(props.session.worktree) }}>
      <EditNotes sessionId={props.session.id} />
      <Show when={busy() && queued()}>
        {(q) => (
          <div class="queued-msg" title="Sent as your next message once the Agent's turn ends">
            <Clock />
            <span class="grow ellipsis">{q().text || "(no text)"}</span>
            <Show when={q().attachments.length}>
              <span class="muted">
                <Paperclip />
                {q().attachments.length}
              </span>
            </Show>
            <button class="link" onClick={() => void takeBack()} title="Back into the input to change it (queue it again with Enter)">
              Edit
            </button>
            <button class="close-tab" onClick={unqueue} aria-label="Remove the queued message" title="Remove it: it won't be sent">
              <X />
            </button>
          </div>
        )}
      </Show>
      <Show when={attachments().length}>
        <div class="edit-notes attachments">
          <For each={attachments()}>
            {(a) => (
              <span class="edit-note">
                <span class="edit-note-name" title={a.kind === "text" ? (a.path ?? a.name) : a.name}>
                  {a.kind === "image" ? <ImageIcon /> : <FileText />}
                  <span class="mono">{a.name}</span>
                </span>
                <button
                  class="close-tab"
                  aria-label={`Don't attach ${a.name}`}
                  title="Don't attach it"
                  onClick={() => setAttachments(attachments().filter((x) => x !== a))}
                >
                  <X />
                </button>
              </span>
            )}
          </For>
          <span class="note">attached to your next message</span>
        </div>
      </Show>
      <Show when={error()}>
        <p class="error">
          <TriangleAlert />
          {error()}
        </p>
      </Show>
      <div class="composer-box" classList={{ locked: locked(), dropping: dropping(), working: working() }}>
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
            onPaste={onPaste}
            onKeyDown={(e) => {
              if (slash.key(e, (command) => complete(command.name))) return;
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                void send();
              } else if (e.key === "Escape" && busy()) {
                e.preventDefault();
                e.stopPropagation();
                void stop();
              }
            }}
            placeholder={
              props.session.state === "exited"
                ? "The Agent exited."
                : props.session.state === "needsYou"
                  ? "Answer the permission card above first (Y / N)."
                  : working()
                    ? "Queue a message for when the Agent's done (Enter to queue, Esc to stop the Agent)"
                    : "Message the Agent (Enter to send, Shift+Enter for a newline, / for commands)"
            }
            disabled={locked()}
          />
          <div class="composer-bar">
            <button
              class="ghost attach icon-only"
              onClick={() => void pickFiles()}
              disabled={props.session.state === "exited"}
              title="Attach files or images to your next message (or drop or paste them here)"
              aria-label="Attach files"
            >
              <Paperclip />
            </button>
            <label class={`mode-pill ${props.session.permissionMode}`} title="Permission mode">
              <Switch fallback={<ShieldQuestion />}>
                <Match when={props.session.permissionMode === "acceptEdits"}>
                  <Zap />
                </Match>
                <Match when={props.session.permissionMode === "plan"}>
                  <BookOpen />
                </Match>
                <Match when={props.session.permissionMode === "auto"}>
                  <Sparkles />
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
            <Show when={busy() && queued()}>
              <span class="queued-chip">
                <Clock />
                Queued — will be sent when this turn ends
              </span>
            </Show>
            <span class="grow" />
            <Show when={busy()}>
              <button class="stop" onClick={() => void stop()} title="Stop the Agent's turn (Esc)">
                <Square />
                Stop
              </button>
            </Show>
            <Switch
              fallback={
                <button class="primary send icon-only" onClick={send} disabled={disabled() || !ready()} title="Send (Enter)" aria-label="Send">
                  <ArrowUp />
                </button>
              }
            >
              <Match when={props.session.state === "exited"}>
                <button class="primary send" onClick={() => void resume()} title="Bring the Agent back with this conversation">
                  <CirclePlay />
                  Resume
                </button>
              </Match>
              <Match when={working()}>
                <button class="primary send" onClick={send} disabled={!ready()} title="Send it once the Agent's turn ends (Enter)">
                  Queue
                </button>
              </Match>
            </Switch>
          </div>
        </div>
      </div>
    </div>
  );
}

/** A file's contents, base64-encoded. */
function base64(file: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result).replace(/^data:[^,]*,/, ""));
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}
