import { describe, expect, it } from "vitest";

import {
  admitAgentUserInput,
  MAX_AGENT_USER_INPUT_BYTES,
} from "./userInput";

describe("desktop agent user input admission", () => {
  it("accepts normal multiline and Unicode text without rewriting its contents", () => {
    expect(admitAgentUserInput("  第一行🙂\nsecond line\n  ")).toEqual({
      status: "accepted",
      text: "第一行🙂\nsecond line",
    });
  });

  it("counts the bound in UTF-8 bytes and fails closed for non-string input", () => {
    expect(admitAgentUserInput("x".repeat(MAX_AGENT_USER_INPUT_BYTES))).toMatchObject({
      status: "accepted",
    });
    expect(admitAgentUserInput("🙂".repeat(MAX_AGENT_USER_INPUT_BYTES / 4))).toMatchObject({
      status: "accepted",
    });
    expect(admitAgentUserInput(`x${"🙂".repeat(MAX_AGENT_USER_INPUT_BYTES / 4)}`)).toMatchObject({
      status: "rejected",
    });
    expect(admitAgentUserInput({ text: "hello" })).toEqual({
      status: "rejected",
      error: "agent.inputPlainText",
    });
  });
});
