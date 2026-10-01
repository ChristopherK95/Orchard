# 02: Rich chat rendering

**What to build:** Agent messages in a Tab render as markdown. Code blocks are static highlighted HTML using Lezer grammars, with line numbers and a Copy button. Tool calls appear as collapsed rows. Long transcripts stay fast through a virtualised view.

**Blocked by:** 01 (Walking skeleton).

**Status:** done (verified in the real app on Windows 11, 2026-10-01; benchmark passes with a 2,000-message transcript). Follow-ups: collapsible tool rows and line numbers/Copy on diffs (spec story 17 and planning ticket 09), "Open in editor" (ticket 14).

- [x] Markdown renders: paragraphs, lists, inline code, links, emphasis.
- [x] Fenced code blocks render as static highlighted HTML (no editor instance), with line numbers, a language label and Copy.
- [x] Unknown languages render as plain monospace.
- [x] Permission-card diffs (from ticket 03) use the same static highlighter, by the file's language, keeping their +/− line colours.
- [x] Tool calls (Read, Edit, Bash, …) render as compact rows naming the tool and target.
- [x] A transcript of 1,000+ messages scrolls smoothly, with only on-screen items in the DOM.
- [x] Scrolling back past the loaded page fetches earlier pages from the core (`transcript_page_before`) automatically, replacing ticket 04's "Load earlier messages" button.
- [x] The fake ACP agent can emit tool-call events, and a core test asserts they reach the transcript.
