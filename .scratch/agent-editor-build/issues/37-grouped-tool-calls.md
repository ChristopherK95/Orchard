# 37: Group consecutive tool calls into one collapsible line

**What to build:** A run of tool calls between two Agent messages shows as one quiet line that sums them up, e.g. "Read 2 files, searched once · 1.1s". A chevron expands it into the individual tool call rows.

**Blocked by:** none.

**Status:** not started

- [ ] Consecutive `toolCall` items in the transcript collapse into one group row: a chevron, the summary, and the total duration (mono, dim).
- [ ] The summary counts by kind, in a fixed order: read N files, edited N, ran N commands, searched N times, fetched N. Small counts read "once" and "twice".
- [ ] Groups are collapsed by default; expanding shows the existing rows. A group stays expanded or collapsed while its Tab is open.
- [ ] A group with a failed tool call says so in the summary (red, "1 failed") and starts expanded. While a call is running, its row shows under the summary.
- [ ] A permission card is never folded into a group: it splits the run.
- [ ] Works with the virtualised transcript: groups are built from the items before rendering, keep stable keys when earlier pages load, and are re-measured when expanded.

Notes:
- The duration needs start and end times for tool calls. Check whether the core reports them; if not, leave the duration out.
