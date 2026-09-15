import { expect, test } from "bun:test";
import { greet } from "./greet";

test("greet cumprimenta pelo nome", () => {
  expect(greet("Ana")).toBe("Olá, Ana");
});

test("greet funciona com string vazia", () => {
  expect(greet("")).toBe("Olá, ");
});
