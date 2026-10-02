# 27: Drive the Board from the keyboard

**What to build:** The **Board** can be worked without the mouse: arrow keys move a focused card, Enter opens its Tab, and on a Needs-you card **Y / N** answer its permission. The Board can then clear a queue of blocked Agents without opening a Tab. The head's hint reads "arrows move · Enter opens · Y / N answer". Allow / Deny on a Needs-you card show their `Y` / `N` key caps.

**Blocked by:** 22 (Board view).

**Status:** ready-for-agent

- [ ] Opening the Board focuses the first Needs-you card, or the first card if none needs you; the focused card has the accent focus ring.
- [ ] ←/→ move between columns, keeping the row position where possible; ↑/↓ move within a column. (The design's assumption, to confirm: columns are the main grouping. Moving through one long list is the alternative.)
- [ ] Enter opens the focused card's Tab (as clicking it does).
- [ ] Y / N answer the **focused** card's question only, with the Agent's allow-once and reject options (`yesOption` / `noOption`), and never another card's. After answering, the next waiting question on that card shows, or focus stays on the card as it moves to its new column.
- [ ] When a card moves column because its state changed, focus follows it rather than jumping to whatever took its place.
- [ ] Y / N do nothing while focus is in the filter chips or a text field. Esc still returns to the Tabs.
- [ ] Reduced motion: cards jump between columns rather than sliding.

Notes: the app-wide Y/N handler already steps aside while the Board is open, so the Board can own these keys. Design reference: `pen_design.pen`, Screens · "Board busy" and Components · "Board card".
