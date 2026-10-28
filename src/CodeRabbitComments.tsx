// CodeRabbit's open review comments on one of the user's PRs, from the Dashboard: what each asks
// for (how much it matters, its title, where), opened out for the rest of what it says, CodeRabbit's
// prompt for AI agents and the replies since. Every comment is chosen at first; an Agent session
// can be started on the chosen ones, to make each change or else reply on GitHub saying why not.
import { openUrl } from "@tauri-apps/plugin-opener";
import { createResource, createSignal, For, Show } from "solid-js";
import { core, type CodeRabbitComment, type CodeRabbitFeedback, type MyPullRequest } from "./core";
import { Bot, ChevronDown, ChevronRight, ExternalLink, Loader, MessageSquare, Sparkles, X } from "./icons";
import { handleTranscriptClick, renderMarkdown } from "./markdown";

export function CodeRabbitComments(props: {
  pr: MyPullRequest;
  load: () => Promise<CodeRabbitFeedback>;
  /** Starts an Agent session on the PR with this prompt (the Dashboard's row says how it goes). */
  onStart: (prompt: Promise<string>) => void;
  onClose: () => void;
}) {
  const [error, setError] = createSignal("");
  const [chosen, setChosen] = createSignal<number[]>([]);
  const [feedback] = createResource(async () => {
    try {
      const read = await props.load();
      setChosen(read.comments.map((c) => c.id));
      return read;
    } catch (err) {
      setError(String(err));
      return null;
    }
  });
  const comments = () => feedback()?.comments ?? [];
  const allChosen = () => comments().length > 0 && chosen().length === comments().length;
  const choose = (id: number) => setChosen(chosen().includes(id) ? chosen().filter((i) => i !== id) : [...chosen(), id]);
  const start = () => {
    const read = feedback();
    if (!read || !chosen().length) return;
    props.onStart(core.codeRabbitPrompt(read, chosen()));
    props.onClose();
  };
  const onBodyClick = (e: MouseEvent) => void handleTranscriptClick(e, openUrl, () => {}, () => {}, setError);

  return (
    <div class="modal-backdrop" onClick={(e) => e.target === e.currentTarget && props.onClose()}>
      <div class="modal code-rabbit" role="dialog" aria-label="CodeRabbit's comments">
        <div class="modal-head code-rabbit-title">
          <div>
            <span class="code-rabbit-eyebrow">
              <Bot />
              CodeRabbit asks for changes
            </span>
            <b>
              {props.pr.title} <span class="pr-number">#{props.pr.number}</span>
            </b>
          </div>
          <button class="ghost icon" onClick={() => props.onClose()} aria-label="Close">
            <X />
          </button>
        </div>
        <Show when={error()}>
          <p class="error small modal-error">{error()}</p>
        </Show>
        <Show when={comments().length > 1}>
          <div class="code-rabbit-bar">
            <label class="code-rabbit-all">
              <input type="checkbox" checked={allChosen()} onChange={() => setChosen(allChosen() ? [] : comments().map((c) => c.id))} />
              {chosen().length} of {comments().length} chosen
            </label>
          </div>
        </Show>
        <div class="modal-body" onClick={onBodyClick}>
          <Show
            when={!feedback.loading}
            fallback={
              <p class="dashboard-line muted">
                <Loader class="spin" />
                Asking GitHub…
              </p>
            }
          >
            <Show
              when={comments().length}
              fallback={
                <Show when={feedback()}>
                  <p class="dashboard-line muted">No open CodeRabbit comments</p>
                </Show>
              }
            >
              <ul class="code-rabbit-list">
                <For each={comments()}>{(c) => <Item comment={c} chosen={chosen().includes(c.id)} onChoose={() => choose(c.id)} onError={setError} />}</For>
              </ul>
            </Show>
          </Show>
        </div>
        <div class="modal-foot">
          <span class="hint">The Agent makes each change, or replies on GitHub saying why not</span>
          <button class="ghost" onClick={() => props.onClose()}>
            Cancel
          </button>
          <button class="primary" onClick={start} disabled={!chosen().length} title="A session on its branch, its composer filled with the chosen comments (not sent)">
            <Sparkles />
            Start an Agent session
          </button>
        </div>
      </div>
    </div>
  );
}

/** A comment's text as one plain line, to show a glimpse of it while its card is closed. */
const glimpse = (markdown: string) =>
  markdown
    .replace(/```[\s\S]*?```/g, " ")
    .replace(/[`*_>#]/g, "")
    .replace(/\s+/g, " ")
    .trim();

/** One comment: chosen with its box, opened out by a click anywhere on it (but its box and link). */
function Item(props: { comment: CodeRabbitComment; chosen: boolean; onChoose: () => void; onError: (message: string) => void }) {
  const c = () => props.comment;
  const [open, setOpen] = createSignal(false);
  const [promptOpen, setPromptOpen] = createSignal(false);
  const file = () => c().path.split("/").pop() ?? c().path;
  const dir = () => c().path.slice(0, c().path.length - file().length);
  return (
    <li
      classList={{ chosen: props.chosen, open: open() }}
      onClick={(e) => {
        // (Its box, its links and the opened-out text, which may be selected, don't close it.)
        if ((e.target as HTMLElement).closest("input, a, .ghost, .code-rabbit-open, .code-rabbit-more")) return;
        setOpen(!open());
      }}
    >
      <div class="code-rabbit-head">
        <input type="checkbox" class="overview-check" checked={props.chosen} onChange={() => props.onChoose()} aria-label={`Include “${c().headline}”`} />
        <button class="code-rabbit-open" onClick={() => setOpen(!open())} aria-expanded={open()} title={open() ? "Hide the comment" : "Show the comment"}>
          <Show when={open()} fallback={<ChevronRight />}>
            <ChevronDown />
          </Show>
          <Show when={c().severity}>{(s) => <span class={`code-rabbit-severity sev-${s().toLowerCase()}`}>{s()}</span>}</Show>
          <span class="code-rabbit-headline">{c().headline}</span>
        </button>
        <button class="ghost icon" onClick={() => void openUrl(c().url).catch((err) => props.onError(String(err)))} title="Open on GitHub" aria-label="Open on GitHub">
          <ExternalLink />
        </button>
      </div>
      <div class="code-rabbit-meta">
        <span class="code-rabbit-path mono" title={c().path}>
          <span class="code-rabbit-dir">{dir()}</span>
          <span class="code-rabbit-file">
            {file()}
            <Show when={c().line !== null}>
              <span class="muted">:{c().line}</span>
            </Show>
          </span>
        </span>
        <Show when={c().outdated}>
          <span class="pr-outdated">Outdated</span>
        </Show>
        <For each={c().tags}>{(t) => <span class="code-rabbit-tag">{t}</span>}</For>
        <Show when={c().replies.length}>
          <span class="code-rabbit-replies">
            <MessageSquare />
            {c().replies.length}
          </span>
        </Show>
      </div>
      <Show when={!open() && c().body}>
        <p class="code-rabbit-glimpse">{glimpse(c().body)}</p>
      </Show>
      <Show when={open()}>
        <div class="code-rabbit-more">
          <Show when={c().body}>
            <div class="markdown" innerHTML={renderMarkdown(c().body)} />
          </Show>
          <Show when={c().agentPrompt}>
            {(prompt) => (
              <div class="code-rabbit-prompt">
                <button class="link" onClick={() => setPromptOpen(!promptOpen())} aria-expanded={promptOpen()}>
                  <Show when={promptOpen()} fallback={<ChevronRight />}>
                    <ChevronDown />
                  </Show>
                  CodeRabbit's prompt for AI agents
                </button>
                <Show when={promptOpen()}>
                  <pre>{prompt()}</pre>
                </Show>
              </div>
            )}
          </Show>
          <For each={c().replies}>
            {(r) => (
              <div class="code-rabbit-reply">
                <b>{r.author}</b>
                <div class="markdown" innerHTML={renderMarkdown(r.body)} />
              </div>
            )}
          </For>
        </div>
      </Show>
    </li>
  );
}
