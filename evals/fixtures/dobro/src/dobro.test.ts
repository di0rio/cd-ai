import { expect, test } from "bun:test";
import { dobro } from "./dobro";

test("dobro devolve o dobro do número", () => {
  expect(dobro(3)).toBe(6);
});

test("dobro de zero é zero", () => {
  expect(dobro(0)).toBe(0);
});
