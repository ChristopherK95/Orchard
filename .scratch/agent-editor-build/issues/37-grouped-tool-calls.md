# 37: Group consecutive tool calls into one collapsible line

**What to build:** A run of tool calls between two Agent messages shows as one quiet line that sums them up, e.g. "Read 2 files, searched once · 1.1s". A chevron expands it into the individual tool call rows.

**Blocked by:** none.

**Status:** done (Windows: type-checked; not run in the app. Arch pending)

- [x] Consecutive `toolCall` items in the transcript collapse into one group row: a chevron, the summary, and the total duration (mono, dim).
- [x] The summary counts by kind, in a fixed order: read N files, edited N, ran N commands, searched N times, fetched N. Small counts read "once" and "twice".
- [x] Groups are collapsed by default; expanding shows the existing rows. A group stays expanded or collapsed while its Tab is open.
- [x] A group with a failed tool call says so in the summary (red, "1 failed") and starts expanded. While a call is running, its row shows under the summary.
- [x] A permission card is never folded into a group: it splits the run.
- [x] Works with the virtualised transcript: groups are built from the items before rendering, keep stable keys when earlier pages load, and are re-measured when expanded.

Notes:
- The duration needs start and end times for tool calls. Check whether the core reports them; if not, leave the duration out.

**Notes (done):**
- **Deviation:** no duration. The core's `toolCall` item has no start or end time (ticket 25 lists adding them), so the summary is the counts alone.
- Frontend only (`src/Transcript.tsx`). The rows are built from the items before the virtualiser sees them: a run of two or more tool calls is one row, keyed by its first item's absolute index, so keys stay put as calls arrive and as earlier pages load. One call alone stays a plain row. The virtualiser's ResizeObserver picks up the new height when a group opens.
- Kinds outside the five ("think", "delete", "move", none) are counted last as "used N other tools". "Once" and "twice" are for searched and fetched, the parts counted in times.
- Open or closed is kept per session and group (by its first call's id) while the app runs. A group that grows backwards when an earlier page loads gets a new first call, so it falls back to the default.
