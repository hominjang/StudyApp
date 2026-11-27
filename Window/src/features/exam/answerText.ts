/** 한 줄 input이 숨기는 CR/LF가 있는 기존 답변만 복원용 편집기를 사용한다. */
export const needsMultilineEditor = (value: string): boolean => /[\r\n]/.test(value);

/** textarea의 표시 값. 저장·제출하는 원문과 분리한다. */
export const toEditorText = (value: string): string => value.replace(/\r\n?/g, '\n');

function rawOffset(value: string, visibleOffset: number): number {
  let raw = 0;
  for (let visible = 0; visible < visibleOffset; visible++) {
    raw += value[raw] === '\r' && value[raw + 1] === '\n' ? 2 : 1;
  }
  return raw;
}

/** 편집된 구간만 바꾸어 textarea가 정규화한 기존 CR/CRLF를 조용히 덮어쓰지 않는다. */
export function applyEditorChange(raw: string, next: string): string {
  const previous = toEditorText(raw);
  if (previous === next) return raw;
  let start = 0;
  while (start < previous.length && start < next.length && previous[start] === next[start]) start++;
  let end = previous.length;
  let nextEnd = next.length;
  while (end > start && nextEnd > start && previous[end - 1] === next[nextEnd - 1]) { end--; nextEnd--; }
  return raw.slice(0, rawOffset(raw, start)) + next.slice(start, nextEnd) + raw.slice(rawOffset(raw, end));
}
