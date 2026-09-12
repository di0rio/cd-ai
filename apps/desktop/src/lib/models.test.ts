import { describe, expect, test } from "bun:test";
import { defaultModel } from "./models";

describe("defaultModel", () => {
  test("a preferência guardada vence o modelo carregado", () => {
    expect(defaultModel("modelo-b", ["modelo-a"], ["modelo-a", "modelo-b"])).toBe("modelo-b");
  });

  test("sem preferência, cai no modelo já carregado", () => {
    expect(defaultModel(null, ["modelo-a"], ["modelo-b", "modelo-a"])).toBe("modelo-a");
  });

  test("uma preferência que sumiu da listagem cai no modelo carregado", () => {
    expect(defaultModel("modelo-removido", ["modelo-a"], ["modelo-a", "modelo-b"])).toBe("modelo-a");
  });

  test("sem nada carregado, o primeiro da lista", () => {
    expect(defaultModel(null, [], ["modelo-b", "modelo-a"])).toBe("modelo-b");
    expect(defaultModel("modelo-removido", [], ["modelo-b"])).toBe("modelo-b");
  });

  test("sem Ollama não há modelo, e nada trava", () => {
    expect(defaultModel("modelo-x", [], [])).toBe("");
    expect(defaultModel(null, [], [])).toBe("");
  });
});
