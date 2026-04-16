export type HighlightPart = string | { text: string; highlight: true };

/**
 * Split `text` into a sequence of plain-text and highlighted parts based on
 * case-insensitive occurrences of `query`. Empty query returns the input
 * unchanged in a single part.
 */
export function highlightText(text: string, query: string): HighlightPart[] {
  if (!query.trim()) return [text];

  const parts: HighlightPart[] = [];
  const lower = text.toLowerCase();
  const qLower = query.toLowerCase();
  let lastIdx = 0;
  let idx = lower.indexOf(qLower);

  while (idx !== -1) {
    if (idx > lastIdx) parts.push(text.slice(lastIdx, idx));
    parts.push({ text: text.slice(idx, idx + query.length), highlight: true });
    lastIdx = idx + query.length;
    idx = lower.indexOf(qLower, lastIdx);
  }
  if (lastIdx < text.length) parts.push(text.slice(lastIdx));
  return parts;
}
