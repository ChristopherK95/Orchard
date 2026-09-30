# 02: Rich chat rendering

**What to build:** Agent messages in a Tab render as markdown. Code blocks are static highlighted HTML using Lezer grammars, with line numbers and a Copy button. Tool calls appear as collapsed rows. Long transcripts stay fast through a virtualised view.

**Blocked by:** 01 (Walking skeleton).

**Status:** ready-for-agent

- [ ] Markdown renders: paragraphs, lists, inline code, links, emphasis.
- [ ] Fenced code blocks render as static highlighted HTML (no editor instance), with line numbers, a language label and Copy.
- [ ] Unknown languages render as plain monospace.
- [ ] Tool calls (Read, Edit, Bash, …) render as compact rows naming the tool and target.
- [ ] A transcript of 1,000+ messages scrolls smoothly, with only on-screen items in the DOM.
- [ ] The fake ACP agent can emit tool-call events, and a core test asserts they reach the transcript.
