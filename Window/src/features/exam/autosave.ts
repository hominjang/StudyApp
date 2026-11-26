import type { SaveProgressResult, SubmitExamRequest } from '../../bridge/desktop';

export type SaveState = { phase: 'saved' | 'dirty' | 'saving' | 'error'; persistedRevision: number; error: unknown | null };
type Clock = { now: () => number; set: (callback: () => void, delay: number) => number; clear: (id: number) => void };
const realClock: Clock = { now: () => performance.now(), set: (callback, delay) => globalThis.setTimeout(callback, delay), clear: (id) => globalThis.clearTimeout(id) };
const copy = (snapshot: SubmitExamRequest): SubmitExamRequest => ({ ...snapshot, responses: [...snapshot.responses] });

/** 실행 중인 저장 하나와 최신 미전송 스냅샷만 유지한다. */
export class AutosaveCoordinator {
  private latest: SubmitExamRequest;
  private persisted: number;
  private running: Promise<void> | null = null;
  private debounce: number | null = null;
  private deadline: number | null = null;
  private dirtySince: number | null = null;
  private lastStarted: number | null = null;
  private error: unknown | null = null;
  private paused = false;
  private disposed = false;
  private save: (snapshot: SubmitExamRequest) => Promise<SaveProgressResult>;
  private changed: (state: SaveState) => void;
  private clock: Clock;

  constructor(snapshot: SubmitExamRequest, save: (snapshot: SubmitExamRequest) => Promise<SaveProgressResult>, changed: (state: SaveState) => void, clock: Clock = realClock) {
    this.latest = copy(snapshot); this.persisted = snapshot.revision;
    this.save = save; this.changed = changed; this.clock = clock;
  }

  update(snapshot: SubmitExamRequest, immediate = false) {
    if (this.disposed || snapshot.exam_id !== this.latest.exam_id || snapshot.revision <= this.latest.revision) return;
    this.latest = copy(snapshot); this.error = null; this.dirtySince ??= this.clock.now(); this.emit();
    if (immediate && !this.paused) void this.flush().catch(() => {}); else this.schedule();
  }

  pause() { this.paused = true; this.clearTimers(); }
  resume() { if (!this.disposed) { this.paused = false; this.schedule(); } }
  dispose() { this.disposed = true; this.clearTimers(); }

  async flush(): Promise<void> {
    if (this.disposed) throw new Error('이미 닫힌 시험의 저장을 다시 시작할 수 없어요.');
    this.clearTimers(); this.error = null;
    while (this.latest.revision > this.persisted || this.running) {
      if (this.disposed) throw new Error('닫힌 화면의 저장 요청은 중단했어요.');
      if (!this.running) this.start();
      await this.running;
    }
    this.emit();
  }

  private start() {
    const snapshot = copy(this.latest); this.lastStarted = this.clock.now(); this.clearTimers();
    this.running = Promise.resolve().then(() => this.save(snapshot)).then((result) => {
      if ((result.outcome !== 'saved' && result.outcome !== 'already_saved') || result.revision !== snapshot.revision) {
        throw new Error(result.outcome === 'locked' ? '이 시험의 답변은 제출되어 더 이상 저장할 수 없어요. 입력은 화면에 유지됩니다.' : '저장된 버전과 현재 입력이 달라요. 입력을 유지한 채 저장 상태를 확인해주세요.');
      }
      this.persisted = snapshot.revision;
      if (this.latest.revision === this.persisted) this.dirtySince = null;
    }).catch((error: unknown) => { this.error = error; throw error; }).finally(() => {
      this.running = null; this.emit(); this.schedule();
    });
    this.emit();
  }

  private schedule() {
    if (this.disposed || this.paused || this.running || this.error || this.latest.revision <= this.persisted) return;
    if (this.debounce !== null) this.clock.clear(this.debounce);
    const request = () => { void this.flush().catch(() => {}); };
    this.debounce = this.clock.set(request, 500);
    if (this.deadline === null) {
      const since = Math.max(this.dirtySince ?? this.clock.now(), this.lastStarted ?? 0);
      this.deadline = this.clock.set(request, Math.max(0, 2000 - (this.clock.now() - since)));
    }
  }

  private emit() {
    if (!this.disposed) this.changed({ phase: this.error ? 'error' : this.running ? 'saving' : this.latest.revision > this.persisted ? 'dirty' : 'saved', persistedRevision: this.persisted, error: this.error });
  }
  private clearTimers() {
    if (this.debounce !== null) this.clock.clear(this.debounce);
    if (this.deadline !== null) this.clock.clear(this.deadline);
    this.debounce = null; this.deadline = null;
  }
}
