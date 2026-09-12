/**
 * Which model a task starts with, before the user touches the selector.
 *
 * The remembered choice only decides this default: the selector still offers everything Ollama
 * lists. A saved model that is no longer installed is ignored rather than kept, so a `ollama rm`
 * or another machine falls back instead of leaving the app without a model.
 */
export function defaultModel(preferred: string | null, loadedModels: string[], models: string[]): string {
  if (preferred && models.includes(preferred)) return preferred;
  return loadedModels[0] ?? models[0] ?? "";
}
