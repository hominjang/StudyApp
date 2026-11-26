import assert from 'node:assert/strict';
import test from 'node:test';
import { AutosaveCoordinator } from '../src/features/exam/autosave.ts';

class Clock {
  time = 0; next = 0; timers = new Map();
  now = () => this.time;
  set = (fn, delay) => { const id = ++this.next; this.timers.set(id, { at: this.time + delay, fn }); return id; };
  clear = (id) => { this.timers.delete(id); };
  advance(ms) {
    const until = this.time + ms;
    for (;;) {
      const due = [...this.timers].filter(([,timer]) => timer.at <= until).sort((a,b) => a[1].at - b[1].at)[0];
      if (!due) break;
      this.time = due[1].at; this.timers.delete(due[0]); due[1].fn();
    }
    this.time = until;
  }
}
const tick = async () => { for (let i=0;i<12;i++) await Promise.resolve(); };
const snapshot = (revision, text = String(revision)) => ({ exam_id: 'fixture-exam', revision, current_position: 0, responses: [text] });
const saved = (request) => ({ outcome: 'saved', revision: request.revision, status: 'in_progress' });
function harness(save) {
  const clock = new Clock(); const calls = []; const states = [];
  const coordinator = new AutosaveCoordinator(snapshot(0, ''), async (request) => { calls.push(request); return save ? save(request) : saved(request); }, (state) => states.push(state), clock);
  return { clock, calls, states, coordinator };
}

test('500ms 후 최신 답변을 저장하고 원문 스냅샷을 복사한다', async () => {
  const h = harness(); const edited = snapshot(1, ' a  b '); h.coordinator.update(edited);
  edited.responses[0] = 'mutated'; h.clock.advance(499); await tick(); assert.equal(h.calls.length, 0);
  h.clock.advance(1); await tick(); assert.equal(h.calls.length, 1); assert.equal(h.calls[0].responses[0], ' a  b ');
  assert.equal(h.states.at(-1).phase, 'saved'); h.coordinator.dispose();
});
test('계속 입력해도 2초에 저장 요청을 보낸다', async () => {
  const h = harness(); h.coordinator.update(snapshot(1));
  for (let i=2;i<=5;i++) { h.clock.advance(400); h.coordinator.update(snapshot(i)); }
  h.clock.advance(399); await tick(); assert.equal(h.calls.length, 0);
  h.clock.advance(1); await tick(); assert.equal(h.calls[0].revision, 5); h.coordinator.dispose();
});
test('저장 하나만 실행하며 지연 중 입력은 최신 것 하나로 합친다', async () => {
  let resolve; const h = harness((request) => h.calls.length === 1 ? new Promise((done) => { resolve = () => done(saved(request)); }) : saved(request));
  h.coordinator.update(snapshot(1), true); await tick();
  h.coordinator.update(snapshot(2)); h.coordinator.update(snapshot(3));
  h.clock.advance(5000); await tick(); assert.equal(h.calls.length, 1);
  const flushed = h.coordinator.flush(); resolve(); await flushed;
  assert.deepEqual(h.calls.map((r) => r.revision), [1,3]); assert.equal(h.states.at(-1).persistedRevision, 3); h.coordinator.dispose();
});
test('실패한 저장은 완료로 표시하지 않고 같은 버전으로 재시도한다', async () => {
  let fail = true; const h = harness((request) => { if (fail) throw new Error('synthetic failure'); return saved(request); });
  h.coordinator.update(snapshot(1)); await assert.rejects(h.coordinator.flush());
  assert.equal(h.states.at(-1).phase, 'error'); assert.equal(h.states.at(-1).persistedRevision, 0);
  fail = false; await h.coordinator.flush(); assert.equal(h.states.at(-1).phase, 'saved'); assert.deepEqual(h.calls.map((r) => r.revision), [1,1]); h.coordinator.dispose();
});
test('stale과 locked는 최신 입력이 저장됐다고 표시하지 않는다', async () => {
  for (const outcome of ['stale','locked']) {
    const h = harness(() => ({ outcome, revision: 10, status: outcome === 'locked' ? 'submitted' : 'in_progress' }));
    h.coordinator.update(snapshot(1)); await assert.rejects(h.coordinator.flush());
    assert.equal(h.states.at(-1).persistedRevision, 0); assert.equal(h.states.at(-1).phase, 'error'); h.coordinator.dispose();
  }
});
test('일시 정지 후에도 명시적 flush는 최신 버전을 저장한다', async () => {
  const h = harness(); h.coordinator.pause(); h.coordinator.update(snapshot(1)); h.clock.advance(5000); await tick(); assert.equal(h.calls.length, 0);
  await h.coordinator.flush(); assert.equal(h.calls.length, 1);
  h.coordinator.update(snapshot(2)); h.coordinator.resume(); h.clock.advance(500); await tick(); assert.equal(h.calls.length, 2); h.coordinator.dispose();
});
test('다른 시험과 오래된 버전은 현재 저장을 바꾸지 않는다', async () => {
  const h = harness(); h.coordinator.update(snapshot(2)); h.coordinator.update(snapshot(1)); h.coordinator.update({ ...snapshot(3), exam_id: 'other' });
  await h.coordinator.flush(); assert.equal(h.calls[0].revision, 2); h.coordinator.dispose();
});
test('폐기된 화면은 지연 응답을 반영하거나 추가 저장하지 않는다', async () => {
  let resolve; const h = harness((request) => new Promise((done) => { resolve = () => done(saved(request)); }));
  h.coordinator.update(snapshot(1), true); await tick(); h.coordinator.update(snapshot(2));
  h.coordinator.dispose(); const before = h.states.length; resolve(); await tick(); h.clock.advance(5000); await tick();
  assert.equal(h.states.length, before); assert.equal(h.calls.length, 1);
});
