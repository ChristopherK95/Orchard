import type { SessionState } from "./core";

export const STATE_LABEL: Record<SessionState, string> = {
  working: "Working",
  needsYou: "Needs you",
  idle: "Idle",
  suspended: "Suspended",
  exited: "Exited",
};
