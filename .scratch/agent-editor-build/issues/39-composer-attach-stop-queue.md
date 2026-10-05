# 39: Composer: attach files, Stop, and queue a message

**What to build:** The rest of the design's composer toolbar:
- a paperclip to attach files or images to the next prompt
- a Stop button while the Agent works
- queuing a message sent while the Agent works ("Queued — sent after the current step")

**Blocked by:** none.

**Status:** not started

Attach:
- [ ] The paperclip opens a file dialog. Dropping or pasting a file or image into the composer attaches it too.
- [ ] Attachments show as removable chips above the input (like Edit notes). They go with the next prompt as ACP content blocks: images as images, text files as resources.
- [ ] Only kinds the Agent accepts (its prompt capabilities) can be attached; anything else is refused with a reason.

Stop:
- [ ] While the session is Working, a Stop button (and Esc in the composer) cancels the turn (ACP `session/cancel`). The session goes Idle and the transcript says the turn was stopped.
- [ ] An open permission card is cancelled along with the turn, as when a turn ends.

Queue:
- [ ] While the session is Working, the input stays enabled, and Enter (the button reads "Queue") queues the message instead of refusing it.
- [ ] A queued message shows in the composer as queued and can be edited or removed until it's sent. It's sent when the turn ends Idle, not if it ends Exited or Needs you.
- [ ] One queued message per session. Queuing again replaces it or appends to it (to decide).

Notes:
- The core only has `send_prompt`. Stop needs a `cancel_turn` command. Queuing could live in the core, so a queued message survives switching Tabs and columns.
