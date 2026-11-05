/** Range de bytes pedido pelo cliente, já resolvido contra o tamanho do arquivo. */
export type ByteRange = { offset: number; length: number };

/**
 * `null` = sem Range (ou em forma que ignoramos, como múltiplos intervalos → 200 completo);
 * `"unsatisfiable"` = 416.
 */
export function parseRange(header: string | null | undefined, size: number): ByteRange | null | "unsatisfiable" {
  if (!header) return null;
  const m = /^bytes=(\d*)-(\d*)$/.exec(header.trim());
  if (!m) return null;
  const [, a, b] = m;
  if (a === "" && b === "") return null;
  if (a === "") {
    // Sufixo: últimos N bytes.
    const n = Number(b);
    if (n === 0) return "unsatisfiable";
    const length = Math.min(n, size);
    return { offset: size - length, length };
  }
  const start = Number(a);
  if (start >= size) return "unsatisfiable";
  const end = b === "" ? size - 1 : Math.min(Number(b), size - 1);
  if (end < start) return "unsatisfiable";
  return { offset: start, length: end - start + 1 };
}
