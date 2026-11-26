import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { RefObject } from 'react';
import { completeExam, describeError, getExamResult, getExamSession, submitExam } from '../../bridge/desktop';
import type { ExamResult, ExamSession, SubmitExamRequest, ReviewMode } from '../../bridge/desktop';
import { isUnanswered, showWhitespace } from './rules';
import { useExamProgress } from './useExamProgress';

export type ExamGuard = { prepareToLeave: () => Promise<void> };
type Props = { session: ExamSession; onDone: (result: ExamResult) => void; onLeave: () => void; guardRef: RefObject<ExamGuard | null> };

export function ExamWorkspace({ session, onDone, onLeave, guardRef }: Props) {
  const progress = useExamProgress(session);
  const responses = progress.draft.responses;
  const position = progress.draft.current_position;
  const [phase, setPhase] = useState<'editing' | 'processing'>(session.status === 'in_progress' ? 'editing' : 'processing');
  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const pending = useRef<SubmitExamRequest | null>(null);
  const active = useRef(false);
  const operation = useRef<Promise<void> | null>(null);
  const phaseRef = useRef(phase);
  phaseRef.current = phase;
  const confirmationButton = useRef<HTMLButtonElement>(null);
  const unanswered = responses.flatMap((text, index) => isUnanswered(text) ? [index + 1] : []);
  const item = session.items[position];

  function finish() {
    if (active.current) return operation.current ?? Promise.resolve();
    const task = finishCore(); operation.current = task; return task;
  }
  async function finishCore() {
    active.current = true; setBusy(true); setError(null); setConfirm(false); setPhase('processing');
    phaseRef.current = 'processing';
    pending.current ??= progress.snapshot();
    try {
      const coordinator = progress.coordinator.current;
      if (!coordinator) throw new Error('시험 저장을 준비하고 있어요. 다시 시도해주세요.');
      coordinator.pause(); await coordinator.flush();
      await submitExam(pending.current);
      await completeExam(session.exam_id);
      onDone(await getExamResult(session.exam_id));
    } catch (cause) { setError(describeError(cause)); }
    finally { active.current = false; setBusy(false); }
  }

  async function returnToAnswers() {
    if (active.current) return;
    active.current = true; setBusy(true); setError(null);
    try {
      const saved = await getExamSession(session.exam_id);
      if (saved.status === 'in_progress') {
        if (saved.revision > progress.latest.current.revision) throw new Error('저장된 답변의 버전이 더 새로워요. 현재 입력을 유지한 채 상태를 확인해주세요.');
        pending.current = null; phaseRef.current = 'editing'; setPhase('editing'); progress.coordinator.current?.resume();
      } else { setError('최종 제출된 답변은 수정할 수 없어요. 채점을 다시 시도해주세요.'); }
    } catch (cause) { setError(describeError(cause)); }
    finally { active.current = false; setBusy(false); }
  }

  async function prepareToLeave() {
    setBusy(true);
    const coordinator = progress.coordinator.current;
    if (!coordinator) throw new Error('시험 저장을 준비하고 있어요. 다시 시도해주세요.');
    coordinator.pause();
    try {
      if (phaseRef.current === 'editing') await coordinator.flush();
      else {
        await operation.current;
        const saved = await getExamSession(session.exam_id);
        if (saved.status === 'in_progress') await coordinator.flush();
      }
    } catch (cause) {
      setError(describeError(cause));
      if (phaseRef.current === 'editing') coordinator.resume();
      throw cause;
    } finally { setBusy(false); }
  }

  useLayoutEffect(() => {
    const guard = { prepareToLeave }; guardRef.current = guard;
    return () => { if (guardRef.current === guard) guardRef.current = null; };
  }, [session.exam_id]);

  useEffect(() => { if (session.status === 'submitted' || session.status === 'completed') void finish(); }, [session.exam_id]);

  async function saveAndLeave() {
    if (busy || active.current) return;
    try { await prepareToLeave(); onLeave(); } catch { /* 입력을 유지하고 오류를 표시한다. */ }
  }

  async function retrySave() {
    try { setError(null); await progress.coordinator.current?.flush(); }
    catch (cause) { setError(describeError(cause)); }
  }

  if (phase === 'processing') return <section className="exam-panel processing-panel" aria-live="polite">
    <div className="eyebrow">YOUR EXAM</div><h1>{busy ? '답변을 제출하고 채점하고 있어요' : '시험 처리를 확인해주세요'}</h1>
    <p>제출과 채점이 끝나면 전체 결과를 한 번에 확인할 수 있어요.</p>
    {error && <div role="alert" className="error-banner">{error}</div>}
    {!busy && <div className="exam-actions"><button className="primary-button" onClick={finish}>제출·채점 다시 시도</button><button className="secondary-button" onClick={returnToAnswers}>제출 상태 확인 후 답변으로</button><button className="secondary-button" onClick={saveAndLeave}>저장 상태 확인 후 나가기</button></div>}
  </section>;

  return <>
    <div className="page-heading"><div className="eyebrow">YOUR EXAM</div><h1>차근차근 풀어보세요</h1><p>{session.dataset_titles.join(' · ')} · {session.items.length}문항</p></div>
    <div className={`save-status ${progress.saveState.phase === 'error' ? 'save-error' : ''}`} role="status">{progress.saveState.phase === 'saved' ? '현재 답변과 위치가 저장됐어요.' : progress.saveState.phase === 'saving' ? '최신 답변을 저장하고 있어요…' : progress.saveState.phase === 'dirty' ? '변경한 답변을 저장할 예정이에요.' : '저장하지 못했어요. 입력은 화면에 유지되어 있어요.'}</div>
    {(error || progress.saveState.error) && <div role="alert" className="error-banner">{error ?? describeError(progress.saveState.error)} <button disabled={busy} onClick={retrySave}>저장 다시 시도</button></div>}
    <div className="exam-grid">
      <section className="exam-panel" aria-labelledby="question-heading">
        <div className="section-heading"><span className="subtle-tag">문제 {position + 1} / {session.items.length}</span><span className="step-label">{responses.length - unanswered.length}문항 응답</span></div>
        <h2 id="question-heading" className="exam-question">{item.question}</h2>
        <label className="answer-label" htmlFor="exam-answer">내 답변</label>
        {/* 정답은 한 줄이므로 한 줄 입력을 사용한다. Enter가 줄바꿈으로 답에 들어가지 않고, 제출 단축키도 연결하지 않는다. */}
        <input id="exam-answer" className="answer-input" type="text" aria-label="내 답변" value={responses[position]} disabled={busy} placeholder="단어나 짧은 문장으로 답해주세요"
          autoComplete="off" autoCorrect="off" autoCapitalize="off" spellCheck={false} onChange={(event) => progress.answer(event.target.value)} />
        <p className="exam-note">정답과 허용 표기에 정확히 맞춰 답해주세요. 정답은 전체 제출 후 공개돼요.</p>
        <div className="exam-actions"><button className="secondary-button" disabled={busy || position === 0} onClick={() => progress.move(position - 1)}>이전 문제</button><button className="secondary-button" disabled={busy || position === session.items.length - 1} onClick={() => progress.move(position + 1)}>다음 문제</button></div>
      </section>
      <aside className="exam-panel question-navigation" aria-label="문항 이동">
        <h2>문항 목록</h2><p className="exam-note">번호를 눌러 이동할 수 있어요.</p>
        <div className="question-numbers">{session.items.map((_, index) => <button key={index} disabled={busy} aria-label={`${index + 1}번 문제${isUnanswered(responses[index]) ? ', 미응답' : ', 응답함'}`} aria-current={position === index ? 'step' : undefined}
          className={`${position === index ? 'current' : ''} ${isUnanswered(responses[index]) ? '' : 'answered'}`} onClick={() => progress.move(index)}>{index + 1}</button>)}</div>
        <p className="exam-note">답변과 위치를 자동 저장해요. 저장 후 나가면 이어 풀 수 있어요. 강제 종료 시에는 마지막 저장까지만 복원돼요.</p>
        <button ref={confirmationButton} disabled={busy} className="primary-button" onClick={() => setConfirm(true)}>제출 확인</button>
        <button disabled={busy} className="secondary-button leave-exam" onClick={saveAndLeave}>저장 후 나가기</button>
      </aside>
    </div>
    {confirm && <div className="dialog-backdrop"><section role="dialog" aria-modal="true" aria-labelledby="submit-title" aria-describedby="submit-description" className="confirm-dialog" onKeyDown={(event) => {
      if (event.key === 'Escape') { setConfirm(false); confirmationButton.current?.focus(); }
      if (event.key === 'Tab') { const buttons = event.currentTarget.querySelectorAll('button'); const first = buttons[0]; const last = buttons[buttons.length - 1]; if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); } else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); } }
    }}><h2 id="submit-title">모든 답변을 제출할까요?</h2><p id="submit-description">전체 {session.items.length}문항 중 {responses.length - unanswered.length}문항에 응답했어요. 최종 제출하면 답변을 수정할 수 없어요.</p>
      {unanswered.length > 0 && <div className="unanswered-warning"><strong>미응답 {unanswered.length}문항</strong><p className="unanswered-numbers">{unanswered.join(', ')}번</p><p>그대로 제출하면 미응답은 오답으로 기록돼요.</p></div>}
      <div className="exam-actions"><button autoFocus className="secondary-button" onClick={() => { setConfirm(false); confirmationButton.current?.focus(); }}>답변으로 돌아가기</button><button className="primary-button" onClick={finish}>최종 제출</button></div>
    </section></div>}
  </>;
}

export function ResultWorkspace({ result, onPrepare, onHistory }: { result: ExamResult; onPrepare: (mode: ReviewMode) => void; onHistory: () => void }) {
  const [filter, setFilter] = useState<'all' | 'incorrect' | 'unanswered'>('all');
  const [visible, setVisible] = useState(false);
  const format = (text: string) => text === '' ? '(빈 답변)' : visible ? showWhitespace(text) : text;
  const items = result.items.filter((item) => filter === 'all' || (filter === 'incorrect' ? item.judgement === 'incorrect' : item.incorrect_reason === 'unanswered'));
  return <>
    <div className="page-heading"><div className="eyebrow">YOUR RESULTS</div><h1>오늘의 시험 결과</h1><p>{result.dataset_titles.join(' · ')} · {new Date(result.completed_at).toLocaleString('ko-KR')}</p></div>
    <section className="exam-panel result-summary" aria-label="시험 결과 요약"><div><strong>{Math.round(result.correct_count / result.items.length * 100)}%</strong><span>정답 비율</span></div><p>전체 <b>{result.items.length}</b>문항</p><p>정답 <b>{result.correct_count}</b>문항</p><p>오답 <b>{result.incorrect_count}</b>문항 <small>(미응답 {result.unanswered_count}문항 포함)</small></p></section>
    <div className="result-toolbar"><div className="segmented" aria-label="결과 필터">{([{ key: 'all', title: '전체' }, { key: 'incorrect', title: '오답' }, { key: 'unanswered', title: '미응답' }] as const).map((item) => <button key={item.key} aria-pressed={filter === item.key} className={filter === item.key ? 'selected' : ''} onClick={() => setFilter(item.key)}>{item.title}</button>)}</div><label><input type="checkbox" checked={visible} onChange={(event) => setVisible(event.target.checked)} /> 공백 문자 표시</label></div>
    <div className="result-items">{items.length === 0 && <p role="status">해당하는 문항이 없어요.</p>}{items.map((item) => <article className="exam-panel result-item" key={item.position}>
      <div className="section-heading"><span className="step-label">문제 {item.position + 1}</span><span className={item.judgement === 'correct' ? 'result-correct' : 'result-incorrect'}>{item.judgement === 'correct' ? '정답' : item.incorrect_reason === 'unanswered' ? '오답 · 미응답' : '오답'}</span></div>
      <h2>{item.question}</h2><dl><dt>내 답변</dt><dd><pre>{format(item.response_text)}</pre></dd><dt>대표 정답</dt><dd><pre>{format(item.answer)}</pre></dd>{item.accepted_answers.length > 0 && <><dt>허용 표기</dt><dd>{item.accepted_answers.map((answer, index) => <pre key={index}>{format(answer)}</pre>)}</dd></>}</dl>
      {typeof item.supplement?.explanation === 'string' && <details><summary>현재 해설 보기</summary><p className="supplement-text">{item.supplement.explanation}</p>{Array.isArray(item.supplement.sources) && item.supplement.sources.every((value) => typeof value === 'string') && <p className="supplement-text">출처: {item.supplement.sources.join(' · ')}</p>}</details>}
      <p className="exam-note">현재 학습 상태: {item.is_review_target ? `오답 복습 중 · 연속 정답 ${item.correct_streak}/3회` : '오답 대상 아님'}</p>
    </article>)}</div>
    <div className="exam-actions"><button className="primary-button" onClick={() => onPrepare(result.settings.review_mode)}>같은 설정으로 다시 준비</button><button className="secondary-button" onClick={() => onPrepare('wrong_only')}>오답 대상 복습</button><button className="secondary-button" onClick={onHistory}>시험 기록 보기</button></div>
    <p className="exam-note">오답 대상 복습은 이번 시험만의 오답이 아닌, 같은 자료 범위의 현재 누적 오답 대상으로 준비해요.</p>
  </>;
}
