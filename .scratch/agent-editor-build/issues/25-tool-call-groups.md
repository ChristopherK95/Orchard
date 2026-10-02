# 25: Group runs of tool calls

**What to build:** Consecutive **Tool calls** with no Agent text between them fold into one quiet row: a chevron, "6 tool calls", one small icon per call (read, search, edit, run, …) and the run's total duration on the right. Clicking it expands the rows underneath. Expanding a single row shows what it did: a run's command and the end of its output, or a read or edit's file. A turn can hold thirty calls, and they must never out-shout the Agent's prose.

**Blocked by:** 02 (Rich chat rendering; this is its "collapsible tool rows" follow-up, spec story 17).

**Status:** ready-for-agent

- [ ] Two or more calls in a row become a group, collapsed by default; one call alone stays a plain row.
- [ ] A group that is still running shows the running call's spinner on its row, and stays collapsed as more calls arrive.
- [ ] A group containing a failed call says so on the collapsed row (danger edge and "1 failed"), so a failure is never hidden by folding.
- [ ] Each row shows its duration on the right (`12 ms`, `4.2 s`) and its status icon.
- [ ] Expanding a run shows `$ command` and the last lines of its output, in mono, with failure lines in the danger colour.
- [ ] Expanded or collapsed state survives scrolling the virtualised transcript (rows are measured again when they change height).
- [ ] The memory benchmark's long transcript still passes.

Notes: the core's `toolCall` item has no timing or output today. The core needs to keep each call's start and finish times and a capped tail of its output from ACP `tool_call_update` content, and send them with the item, or on demand when a row is expanded, which keeps the transcript small. Grouping can be done in the view. Design reference: `pen_design.pen`, Components · "Tool calls" (Statuses and Grouping).
