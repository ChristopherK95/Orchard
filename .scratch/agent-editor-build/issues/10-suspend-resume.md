# 10: Suspend, resume and crash recovery

**What to build:** A Tab can be **Suspended**: its `claude` process stops and the saved conversation stays visible. Sending a message resumes it lazily through ACP. An Exited session offers resume. If the shared ACP adapter crashes, the core restarts it and resumes the sessions; sessions that were mid-turn become Exited.

**Blocked by:** 04 (Multiple sessions and Tab states).

**Status:** ready-for-agent

- [ ] Suspend stops the session's Agent process; the Tab shows the conversation and a Suspended state.
- [ ] Sending a message from a Suspended Tab resumes the same conversation, then processes the message.
- [ ] Exited sessions show a resume action that restores the conversation.
- [ ] Killing the adapter process leads to an automatic restart; Idle sessions come back, and mid-turn sessions become Exited.
- [ ] Core tests with the fake agent: suspend → send → resume; adapter crash → recovery.
