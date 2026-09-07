const LARGE_DOCUMENT_BYTES = 256 * 1024;

export function sourceWindow(content: string, target: number) {
  const lines = content.split(/\r?\n/);
  const large =
    new TextEncoder().encode(content).byteLength > LARGE_DOCUMENT_BYTES;
  const selected = Math.min(Math.max(1, target), lines.length);
  const start = large ? Math.max(0, selected - 101) : 0;
  const end = large ? Math.min(lines.length, selected + 100) : lines.length;
  return {
    large,
    start: start + 1,
    end,
    lines: lines
      .slice(start, end)
      .map((text, index) => ({ number: start + index + 1, text })),
  };
}
