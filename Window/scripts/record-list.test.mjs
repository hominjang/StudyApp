import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { runInNewContext } from 'node:vm';
import { build } from 'vite';
import react from '@vitejs/plugin-react';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

// Render actual pure components with synthetic DTOs; no browser or database is opened.
async function component(name, exported) {
  const entry = fileURLToPath(new URL(`../src/features/exam/${name}.tsx`, import.meta.url));
  const output = await build({ configFile: false, plugins: [react()], logLevel: 'silent', build: {
    write: false, minify: false, lib: { entry, formats: ['cjs'] },
    rollupOptions: { external: ['react', 'react/jsx-runtime'], output: { codeSplitting: false } },
  } });
  const compiled = (Array.isArray(output) ? output : [output]).flatMap((bundle) => bundle.output)
    .find((item) => item.type === 'chunk').code;
  const module = { exports: {} };
  runInNewContext(compiled, { module, exports: module.exports, require: createRequire(import.meta.url) });
  return module.exports[exported];
}
const HistoryRecord = await component('HistoryRecord', 'HistoryRecord');
const PendingExams = await component('PendingExams', 'PendingExams');
const MixedResultWorkspace = await component('MixedResultWorkspace', 'MixedResultWorkspace');
const AnswerInput = await component('AnswerInput', 'AnswerInput');
const ImportSummary = await component('ImportWorkspace', 'ImportSummary');
const StudyCard = await component('StudyWorkspace', 'StudyCard');
const MaterialEditor = await component('MaterialEditor', 'MaterialEditor');
const App = await component('../../App', 'App');
const issue = { kind: 'error', row_key: 'record-1', error: 'Synthetic error' };
const normal = { kind: 'exam', row_key: 'record-2', exam_id: 'synthetic', status: 'completed', dataset_titles: ['Synthetic'],
  question_count: 3, answered_count: 2, current_position: 0, saved_at: '2026-11-26T00:00:00Z', completed_at: '2026-11-26T00:00:00Z', completion_order: 1, correct_count: 2, error: null };

test('메뉴는 자료 학습·시험 시작·시험 기록·자료 추가 순서로 표시한다', () => {
  const html = renderToStaticMarkup(createElement(App));
  const nav = html.match(/<nav aria-label="주 메뉴">([\s\S]*?)<\/nav>/)?.[1];assert.ok(nav);
  const names = ['자료 학습', '시험 시작', '시험 기록', '자료 추가'];
  for (let i = 1; i < names.length; i++) assert.ok(nav.indexOf(names[i-1]) < nav.indexOf(names[i]));
  assert.doesNotMatch(nav, /학습 시작/);
});
test('자료 편집은 정답·허용 표기·해설 원문과 시험 기록 보존 안내를 표시한다', () => {
  const html = renderToStaticMarkup(createElement(MaterialEditor, { item: { dataset_id: 'synthetic', item_id: 'a', dataset_revision: 1, criteria: { type: 'short_answer', answer: 'alpha  beta', accepted_answers: ['alias'] }, explanation: 'Synthetic\r\nexplanation' }, onSaved() {}, onCancel() {} }));
  for (const text of ['대표 정답', 'alpha  beta', '허용 표기', '해설', '수정 저장', '이미 시작한 시험']) assert.ok(html.includes(text));
});
test('서술형 편집은 참고 답안과 핵심 내용을 별도 필드로 보존한다', () => {
  const html = renderToStaticMarkup(createElement(MaterialEditor, { item: { dataset_id: 'synthetic', item_id: 'd', dataset_revision: 1, criteria: { type: 'descriptive', reference_answer: 'Synthetic reference\r\nsecond line', key_points: ['Synthetic point'] }, explanation: null }, onSaved() {}, onCancel() {} }));
  for (const text of ['참고 답안', '핵심 내용 1', 'Synthetic reference', 'Synthetic point']) assert.ok(html.includes(text));
  assert.doesNotMatch(html, /대표 정답|정답률/);
});

test('자료 학습은 질문·대표 정답·허용 표기·해설을 처음부터 함께 표시한다', () => {
  const html = renderToStaticMarkup(createElement(StudyCard, { number: 1, item: { dataset_id: 'one', item_id: 'same', dataset_title: 'Synthetic', question: 'Synthetic question', criteria: { type: 'short_answer', answer: 'alpha  beta', accepted_answers: ['alias'] }, explanation: 'Synthetic explanation\nsecond line' } }));
  for (const text of ['Synthetic question', '정답', 'alpha  beta', '허용 표기', 'alias', '해설', 'Synthetic explanation']) assert.ok(html.includes(text));
  assert.doesNotMatch(html, /<input|<textarea|<button|<details|채점|제출/);
});
test('자료 학습의 서술형은 참고 답안과 핵심 내용을 보이며 HTML을 실행하지 않는다', () => {
  const html = renderToStaticMarkup(createElement(StudyCard, { number: 2, item: { dataset_id: 'two', item_id: 'same', dataset_title: 'Synthetic', question: '<script>synthetic</script>', criteria: { type: 'descriptive', reference_answer: 'Synthetic reference\r\nline two', key_points: ['Point one', 'Point two'] }, explanation: null } }));
  for (const text of ['참고 답안', 'Synthetic reference', '핵심 내용', 'Point one', 'Point two', '&lt;script&gt;']) assert.ok(html.includes(text));
  assert.doesNotMatch(html, /<script|정답률|<button|<input|<textarea/);
});

test('자료 갱신 미리보기는 전체 목록 교체와 비활성화·기록 보존을 안내한다', () => {
  const html = renderToStaticMarkup(createElement(ImportSummary, { preview: { title: 'Synthetic', existing: true, changed: true, question_count: 3, schema_version: '2.0', added: 1, modified: 1, deactivated: 2 } }));
  for (const text of ['기존 자료 갱신', '비활성화', '2문항', '전체 목록', '학습 기록은 보존']) assert.ok(html.includes(text));
});
test('동일 자료 미리보기는 중복 추가하지 않는다고 안내한다', () => {
  const html = renderToStaticMarkup(createElement(ImportSummary, { preview: { title: 'Synthetic', existing: true, changed: false, question_count: 2, schema_version: '1.0', added: 0, modified: 0, deactivated: 0 } }));
  assert.ok(html.includes('중복으로 추가하지 않아요'));
});

test('손상 기록은 열기 버튼과 추정 날짜·점수를 표시하지 않는다', () => {
  const html = renderToStaticMarkup(createElement(HistoryRecord, { row: issue, busy: false, onOpen() {} }));
  assert.match(html, /role="alert"/);assert.doesNotMatch(html, /<button|Invalid Date|NaN|마지막 저장|완료:|0문항/);
});
test('미완료 목록은 손상 행만 차단하고 정상 행의 이어 풀기를 유지한다', () => {
  const html = renderToStaticMarkup(createElement(PendingExams, { exams: [issue, { ...normal, status: 'in_progress' }], busy: false, onResume() {} }));
  assert.equal((html.match(/<button/g) ?? []).length, 1);assert.match(html, /이어 풀기/);assert.doesNotMatch(html, /Invalid Date|NaN/);
});
test('정상 기록은 상태별 열기 동작과 완료 점수 표시를 유지한다', () => {
  for (const [status, title] of [['completed', '결과 보기'], ['submitted', '처리 재개'], ['in_progress', '이어 풀기']]) {
    const html = renderToStaticMarkup(createElement(HistoryRecord, { row: { ...normal, status }, busy: true, onOpen() {} }));
    assert.match(html, new RegExp(title));assert.match(html, /disabled=""/);
    if (status === 'completed') assert.match(html, /정답 2\/3/);else assert.doesNotMatch(html, /정답 2\/3/);
  }
});

test('확장 완료 기록은 네 평가 개수와 평가 이어 하기를 표시하고 백분율을 만들지 않는다', () => {
  const html = renderToStaticMarkup(createElement(HistoryRecord, { row: { ...normal, assessment_status: 'pending', assessment_counts: { correct: 1, partial: 1, incorrect: 0, ungraded: 1 } }, busy: false, onOpen() {} }));
  assert.match(html, /평가 이어 하기/);assert.match(html, /부분정답 1/);assert.match(html, /미평가 1/);assert.doesNotMatch(html, /%/);
});

test('서술형은 빈 답변부터 여러 줄 입력을 사용하고 단답형 복원 안내와 구분한다', () => {
  const props = { initialValue: '', value: '', disabled: false, onChange() {} };
  const short = renderToStaticMarkup(createElement(AnswerInput, props));
  const descriptive = renderToStaticMarkup(createElement(AnswerInput, { ...props, mode: 'descriptive' }));
  assert.match(short, /type="text"/);assert.doesNotMatch(short, /<textarea/);
  assert.match(descriptive, /<textarea/);assert.doesNotMatch(descriptive, /이전에 저장한 여러 줄/);
});

test('혼합 결과는 참고 기준·미평가·정정 동작을 보이며 점수 환산을 하지 않는다', () => {
  const result = { db_version: 2, exam_id: 'synthetic', dataset_titles: ['Synthetic'], counts: { correct: 0, partial: 0, incorrect: 0, ungraded: 1 }, assessment_status: 'pending', result_revision: 0,
    items: [{ position: 0, question: 'Synthetic question', response_text: 'Synthetic original\r\nresponse', criteria: { type: 'descriptive', reference_answer: 'Synthetic reference', key_points: ['Synthetic key point'] }, original_judgement: null, judgement: 'ungraded', incorrect_reason: null, assessment_revision: 0, is_review_target: false, correct_streak: 0, progress_revision: 0 }] };
  const html = renderToStaticMarkup(createElement(MixedResultWorkspace, { result, onUpdate() {}, onPrepare() {}, onHistory() {}, onBusy() {}, guardRef: { current: null } }));
  assert.match(html, /참고 답안/);assert.match(html, /Synthetic key point/);assert.match(html, /부분정답으로 평가/);assert.match(html, /연결된 기초 문항 확인/);assert.match(html, /Synthetic original\r\nresponse/);assert.doesNotMatch(html, /%|점수 환산/);
});
