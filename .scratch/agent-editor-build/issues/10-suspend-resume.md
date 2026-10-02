# 10: Suspend, resume and crash recovery

**What to build:** A Tab can be **Suspended**: its `claude` process stops and the saved conversation stays visible. Sending a message resumes it lazily through ACP. An Exited session offers resume. If the shared ACP adapter crashes, the core restarts it and resumes the sessions; sessions that were mid-turn become Exited.

**Blocked by:** 04 (Multiple sessions and Tab states).

**Status:** done (Windows; Arch pending)

- [x] Suspend stops the session's Agent process; the Tab shows the conversation and a Suspended state. Suspend is offered on the right of the context bar (layout decision, ticket 08 of the plan).
- [x] Sending a message from a Suspended Tab resumes the same conversation, then processes the message.
- [x] Exited sessions show a resume action that restores the conversation.
- [x] Killing the adapter process leads to an automatic restart; Idle sessions come back, and mid-turn sessions become Exited.
- [x] Core tests with the fake agent: suspend → send → resume; adapter crash → recovery.

Notes: the Resume button is offered only for Exited sessions (spec story 36: a Suspended one resumes when you send). After 3 adapter crashes within a minute it isn't restarted again; Tabs say so and Resume tries again.
