// 공통 명세의 고정 White_Space 25자. trim()과 기본 공백 정규식을 사용하지 않는다.
export function isWhiteSpace(code: number): boolean {
  return (code >= 0x9 && code <= 0xd) || code === 0x20 || code === 0x85 || code === 0xa0
    || code === 0x1680 || (code >= 0x2000 && code <= 0x200a) || code === 0x2028
    || code === 0x2029 || code === 0x202f || code === 0x205f || code === 0x3000;
}
export const isUnanswered = (value: string): boolean => [...value].every((char) => isWhiteSpace(char.codePointAt(0)!));
export const showWhitespace = (value: string): string => [...value].map((char) => {
  const code = char.codePointAt(0)!;
  return isWhiteSpace(code) || code <= 0x1f || code === 0x200b || code === 0xfeff
    ? `[U+${code.toString(16).toUpperCase().padStart(4, '0')}]` : char;
}).join('');
