import { expect, test } from "bun:test";
import { soma } from "./soma";

test("soma devolve a soma dos dois números", () => {
  expect(soma(2, 3)).toBe(5);
});

test("soma funciona com zero", () => {
  expect(soma(0, 7)).toBe(7);
});
