import assert from 'node:assert/strict';
import test from 'node:test';
import { isWhiteSpace, isUnanswered, showWhitespace } from '../src/features/exam/rules.ts';

test('고정 공백 25자와 전체 Unicode 범위의 판정', () => {
  const expected = [9,10,11,12,13,32,133,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288];
  const actual = [];
  for (let code = 0; code <= 0x10ffff; code++) if (isWhiteSpace(code)) actual.push(code);
  assert.deepEqual(actual, expected);
  assert.equal(isUnanswered(''), true);
  assert.equal(isUnanswered(expected.map((code) => String.fromCodePoint(code)).join('')), true);
});
test('BOM·제로폭 공백·정보 구분 문자는 응답으로 본다', () => {
  for (const code of [0x1c,0x1d,0x1e,0x1f,0x200b,0xfeff]) assert.equal(isUnanswered(` ${String.fromCodePoint(code)} `), false);
  assert.equal(isUnanswered('\u0085'), true);
  assert.equal(isUnanswered('한글'), false);
});
test('공백 표시에서 원문과 공백 개수를 구분한다', () => {
  assert.equal(showWhitespace('a  b'), 'a[U+0020][U+0020]b');
  assert.equal(showWhitespace('\u0085\ufeff'), '[U+0085][U+FEFF]');
});
