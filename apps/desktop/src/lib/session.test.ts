import { describe, expect, test } from "bun:test";
import { groupActivity } from "./session";

describe("groupActivity", () => {
  test("folds consecutive reads and searches into one block", () => {
    const blocks = groupActivity([
      { kind: "read", path: "a.ts" },
      { kind: "search", query: "x", matches: 2 },
      { kind: "assistant", text: "ok" },
      { kind: "read", path: "b.ts" },
    ]);

    expect(blocks.map((block) => block.kind)).toEqual(["explore", "assistant", "explore"]);
    expect(blocks[0]).toEqual({
      kind: "explore",
      items: [
        { kind: "read", path: "a.ts" },
        { kind: "search", query: "x", matches: 2 },
      ],
    });
  });

  test("keeps failed lookups visible instead of folding them", () => {
    const blocks = groupActivity([
      { kind: "read", path: "a.ts" },
      { kind: "read", path: "missing.ts", error: "arquivo não encontrado" },
      { kind: "read", path: "b.ts" },
    ]);

    expect(blocks.map((block) => block.kind)).toEqual(["explore", "read", "explore"]);
  });

  test("never folds edits or commands", () => {
    const blocks = groupActivity([
      { kind: "edit", path: "a.ts", added: 1, removed: 0 },
      { kind: "command", command: "bun test", exitCode: 0, durationMs: 10 },
    ]);

    expect(blocks.map((block) => block.kind)).toEqual(["edit", "command"]);
  });
});
