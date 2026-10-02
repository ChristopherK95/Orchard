# 26: Queue a message while the Agent works, and Stop a turn

**What to build:** The composer stays usable while an Agent session is **Working**. Enter queues the message instead of being refused: the Send button reads "Queue", and a chip says "Queued — will be sent after the current step". The queued message is sent as the next prompt as soon as the turn ends. Next to it, a **Stop** button ends the turn in progress (ACP `session/cancel`). Stop is offered but optional; queuing is the default. Esc never cancels a turn.

**Blocked by:** 03 (Permission cards), 10 (Suspend and resume).

**Status:** ready-for-agent

- [ ] While Working, Enter (or Queue) queues the text. The composer shows the queued chip and keeps the text editable; changing it changes what will be sent, and clearing it cancels the queued message.
- [ ] One message can be queued at a time. Queuing again replaces it (the composer shows the current queued text, so nothing is lost silently).
- [ ] When the turn ends Idle, the queued message is sent automatically, with any Edit notes as a normal prompt would have.
- [ ] A turn that ends by asking for permission (Needs you) keeps the queued message. The composer stays locked with the chip showing, and the message goes once the question is answered and the turn finishes. (The design's assumption; flushing would lose typing.)
- [ ] If the session Exits mid-turn, the queued text stays in the composer, not sent, with the usual Resume.
- [ ] Stop sends `session/cancel`: the session goes Idle, open permission cards in that turn show "Cancelled", and a queued message is **not** sent (Stop means "wait, let me rethink"). It stays in the composer instead.
- [ ] Stop shows only while Working. Its button reads "Stop", never an icon alone.
- [ ] Core tests with the fake agent: a prompt queued during an `untilCancelled` turn is sent after the turn ends; Stop cancels and the queued prompt is kept, not sent; a queued prompt survives a permission in that turn.

Notes: today `send_prompt` refuses with `SessionBusy` while Working, and `session/cancel` is only sent when closing a Tab. Per ADR 0003 the queue belongs in the core (the session's state, kept if the Tab is switched away and back), not in the composer. A queued message is shown on the Board card too ("1 queued"). Design reference: `pen_design.pen`, Components · "Composer & Edit notes" (Working state) and Notes · "Stop is offered while the Agent works".
