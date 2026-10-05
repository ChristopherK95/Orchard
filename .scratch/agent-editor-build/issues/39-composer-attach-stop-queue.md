# 39: Composer: attach files, Stop, and queue a message

**What to build:** The rest of the design's composer toolbar:
- a paperclip to attach files or images to the next prompt
- a Stop button while the Agent works
- queuing a message sent while the Agent works ("Queued — sent after the current step")

**Blocked by:** none.

**Status:** done (Windows: core tests pass, frontend type-checked; not run in the app. Arch pending)

Attach:
- [x] The paperclip opens a file dialog. Dropping or pasting a file or image into the composer attaches it too.
- [x] Attachments show as removable chips above the input (like Edit notes). They go with the next prompt as ACP content blocks: images as images, text files as resources.
- [x] Only kinds the Agent accepts (its prompt capabilities) can be attached; anything else is refused with a reason.

Stop:
- [x] While the session is Working, a Stop button (and Esc in the composer) cancels the turn (ACP `session/cancel`). The session goes Idle and the transcript says the turn was stopped.
- [x] An open permission card is cancelled along with the turn, as when a turn ends.

Queue:
- [x] While the session is Working, the input stays enabled, and Enter (the button reads "Queue") queues the message instead of refusing it.
- [x] A queued message shows in the composer as queued and can be edited or removed until it's sent. It's sent when the turn ends Idle, not if it ends Exited or Needs you.
- [x] One queued message per session. Queuing again appends to it (a blank line between), so nothing typed is lost.

Notes:
- The core only has `send_prompt`. Stop needs a `cancel_turn` command. Queuing could live in the core, so a queued message survives switching Tabs and columns.

**Notes (done):**
- The core owns the queue (`Control::queued`), so it survives switching Tabs and columns. `queue_prompt` adds to it while a turn runs (or sends at once if the turn has just ended); `take_queued_prompt` gives it back; `QueuedPromptChanged` tells the composer. When a turn ends Idle, the queued message starts the next turn under the same lock: the session never shows Idle in between, so nothing else can claim it first.
- Not sent if the turn was stopped, failed, or the Agent exited: it stays in the core, and the composer takes it back into the input once the session isn't mid-turn.
- `cancel_turn` sends `session/cancel` and cancels open permission cards; when the turn ends the transcript says "You stopped the turn." A Stop before the prompt has gone to the Agent is sent right after it (the prompt request is now written by the call, not when awaited), so the cancel can't overtake it.
- The composer shows the queued message above the input (Edit takes it back into the input, × removes it) and the design's "Queued — will be sent after the current step" chip in the bar; Send reads "Queue" while Working. Stop (and Esc in the composer) shows while Working or Needs you. The input stays locked in Needs you, as the design has it.
- Attachments: `attach_file` (paperclip dialog, or dropped on the composer: the webview reports drops with their paths) and `attach_data` (pasted, base64) check them against the Agent's `promptCapabilities`: PNG/JPEG/GIF/WebP images up to 5 MB as `image` blocks (needs `image`), UTF-8 text files up to 1 MB as `resource` blocks (needs `embeddedContext`). Anything else is refused with a reason. They're chips above the input, kept with the draft, and the user message in the transcript lists their names.
- Not done: the Board card's "1 queued" (ticket 26's note).
