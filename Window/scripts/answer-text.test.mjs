import assert from 'node:assert/strict';
import test from 'node:test';
import { applyEditorChange, needsMultilineEditor, toEditorText } from '../src/features/exam/answerText.ts';
import { AutosaveCoordinator } from '../src/features/exam/autosave.ts';

test('CR·LF를 포함한 복원 답변만 여러 줄 입력으로 구분한다', () => {
  for (const text of ['a\nb', 'a\rb', 'a\r\nb', '\r\n']) assert.equal(needsMultilineEditor(text), true);
  for (const text of ['', '한글', ' a  b ', '\t', '\u0085', '\u2028', '\u2029', '\ufeff']) assert.equal(needsMultilineEditor(text), false);
});

test('표시만 LF로 맞추며 무수정 입력은 CR·LF·CRLF 원문을 유지한다', () => {
  for (const raw of ['a\nb', 'a\rb', 'a\r\nb', '\r\n', 'a\r\nb\rc\nd', ' 한글\r\n\ufeff ']) {
    const visible = toEditorText(raw);
    assert.equal(visible.includes('\r'), false);
    assert.equal(applyEditorChange(raw, visible), raw);
  }
});

test('문자 편집 때 바꾸지 않은 줄바꿈과 공백을 보존한다', () => {
  assert.equal(applyEditorChange('가\r\n나\r다\n라 ', '가\n나무\n다\n라 '), '가\r\n나무\r다\n라 ');
  assert.equal(applyEditorChange('a\r\nb', 'xa\nb'), 'xa\r\nb');
  assert.equal(applyEditorChange('a\r\nb', 'a\nbx'), 'a\r\nbx');
});

test('줄바꿈 삭제·구간 교체·전체 삭제가 저장 원문에 일치한다', () => {
  assert.equal(applyEditorChange('a\r\nb', 'ab'), 'ab');
  assert.equal(applyEditorChange('a\rb', 'ab'), 'ab');
  assert.equal(applyEditorChange('a\r\nb\rc', 'a!c'), 'a!c');
  assert.equal(applyEditorChange('a\r\nb', ''), '');
});

test('붙여 넣은 새 줄바꿈은 편집 구간에만 추가한다', () => {
  assert.equal(applyEditorChange('a\r\nb', 'a\nx\nyb'), 'a\r\nx\nyb');
  assert.equal(applyEditorChange('a\r\nb', 'a\n\nb'), 'a\r\n\nb');
  assert.equal(applyEditorChange('\r\n\n', '\n'), '\r\n');
});

test('한글·이모지·다른 공백은 표시와 저장 간 왕복에서 바뀌지 않는다', () => {
  const samples = ['가\r\n나', '😀\r한글\n🧩', ' a\t\r\nb\u0085\u2028\u2029\ufeff '];
  for (const raw of samples) {
    const text = toEditorText(raw);
    for (const next of [text, `앞${text}`, `${text}뒤`, text.slice(1), text.slice(0, -1), `새\n${text}\n값`, '']) {
      assert.equal(toEditorText(applyEditorChange(raw, next)), next);
    }
  }
});

test('편집 후 자동 저장·재개·제출 스냅샷에 원문 줄바꿈이 유지된다', async () => {
  const raw = '가\r\n나\r다\n라';
  const edited = applyEditorChange(raw, '가\n나무\n다\n라');
  const initial = { exam_id: 'synthetic-exam', revision: 0, current_position: 0, responses: [raw] };
  const saved = [];
  const coordinator = new AutosaveCoordinator(initial, async (snapshot) => {
    saved.push(structuredClone(snapshot));
    return { outcome: 'saved', revision: snapshot.revision, status: 'in_progress' };
  }, () => {});
  coordinator.update({ ...initial, revision: 1, responses: [edited] });
  await coordinator.flush();
  coordinator.dispose();
  const reopened = structuredClone(saved[0]);
  assert.equal(reopened.responses[0], '가\r\n나무\r다\n라');
  assert.equal(applyEditorChange(reopened.responses[0], toEditorText(reopened.responses[0])), reopened.responses[0]);
  assert.equal(JSON.parse(JSON.stringify(reopened)).responses[0], edited);
});
