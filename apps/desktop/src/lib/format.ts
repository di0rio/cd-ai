const compact = new Intl.NumberFormat("pt-BR", { notation: "compact", maximumFractionDigits: 1 });

export const formatTokens = (count: number) => compact.format(count);

export function formatDuration(ms: number) {
  if (ms < 1000) return `${ms} ms`;
  return `${(ms / 1000).toLocaleString("pt-BR", { maximumFractionDigits: 1 })} s`;
}

export function plural(count: number, one: string, many: string) {
  return `${count} ${count === 1 ? one : many}`;
}
