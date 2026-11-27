import { useLayoutEffect, useRef, useState } from 'react';
import type { RefObject } from 'react';
import { addReviewTarget, getAssessmentView, getFoundationLinks, setResponseAssessment } from '../../bridge/assessment';
import type { AddReviewRequest, FoundationLink, SetAssessmentRequest } from '../../bridge/assessment';
import { describeError } from '../../bridge/desktop';
import type { ReviewMode } from '../../bridge/desktop';
import type { MixedResult } from '../../bridge/outcome';
import type { ExamGuard } from './ExamWorkspace';

const titles = { correct: '정답', partial: '부분정답', incorrect: '오답', ungraded: '미평가' };
type Mutation = { kind: 'grade'; request: SetAssessmentRequest } | { kind: 'review'; request: AddReviewRequest };

export function MixedResultWorkspace({ result, onUpdate, onPrepare, onHistory, onBusy, guardRef }: {
  result: MixedResult; onUpdate: (result: MixedResult) => void; onPrepare: (mode: ReviewMode) => void;
  onHistory: () => void; onBusy: (busy: boolean) => void; guardRef: RefObject<ExamGuard | null>;
}) {
  const [busy, setBusy] = useState(false);
  const [blocked, setBlocked] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<'all' | 'ungraded' | 'partial' | 'incorrect'>('all');
  const [links, setLinks] = useState<Record<number, FoundationLink[]>>({});
  const pending = useRef<Mutation | null>(null);
  const active = useRef(false);
  const operation = useRef<Promise<void> | null>(null);

  async function reload() {
    const next = await getAssessmentView(result.exam_id);
    onUpdate({ ...result, ...next }); setLinks({}); pending.current = null; setBlocked(false);
  }
  function run(task: () => Promise<void>) {
    if (active.current) return;
    active.current = true; setBusy(true); onBusy(true); setError(null);
    const promise = (async () => {
      try { await task(); }
      catch (cause) { setError(describeError(cause)); throw cause; }
      finally { active.current = false; setBusy(false); onBusy(pending.current !== null); }
    })();
    operation.current = promise; void promise.catch(() => {});
  }
  function mutate(mutation: Mutation) {
    if (active.current || (pending.current && pending.current !== mutation)) return;
    pending.current = mutation; setBlocked(true);
    run(async () => {
      if (mutation.kind === 'grade') await setResponseAssessment(mutation.request); else await addReviewTarget(mutation.request);
      await reload();
    });
  }
  useLayoutEffect(() => {
    const guard = { prepareToLeave: async () => {
      try { await operation.current; } catch (cause) { if (pending.current) throw cause; }
      if (pending.current) throw new Error('평가 저장 상태를 먼저 확인해주세요.');
    } };
    guardRef.current = guard;
    return () => { if (guardRef.current === guard) guardRef.current = null; };
  }, [result.exam_id, guardRef]);

  const disabled = busy || blocked;
  return <>
    <div className="page-heading"><div className="eyebrow">YOUR RESULTS</div><h1>결과와 회고</h1><p>{result.dataset_titles.join(' · ')} · {result.assessment_status === 'pending' ? '나중에 평가를 이어갈 수 있어요' : '모든 문항을 평가했어요'}</p></div>
    <section className="exam-panel result-summary" aria-label="평가 개수">{(['correct', 'partial', 'incorrect', 'ungraded'] as const).map((key) => <p key={key}>{titles[key]} <b>{result.counts[key]}</b>문항</p>)}</section>
    {error && <div role="alert" className="error-banner">{error}{blocked && <p>저장 결과를 확인할 때까지 다른 평가를 보류해요.</p>}</div>}
    <div className="exam-actions">
      {blocked && <button disabled={busy} className="primary-button" onClick={() => { if (pending.current) mutate(pending.current); }}>저장 상태 확인·다시 시도</button>}
      <button disabled={busy} className="secondary-button" onClick={() => run(reload)}>최신 결과 불러오기</button>
      <button disabled={disabled} className="secondary-button" onClick={onHistory}>나중에 평가·기록으로</button>
    </div>
    <div className="result-toolbar segmented" aria-label="평가 결과 필터">{(['all', 'ungraded', 'partial', 'incorrect'] as const).map((key) => <button key={key} disabled={disabled} aria-pressed={filter === key} onClick={() => setFilter(key)}>{key === 'all' ? '전체' : titles[key]}</button>)}</div>
    <div className="result-items">{result.items.filter((item) => filter === 'all' || item.judgement === filter).map((item) => <article className="exam-panel" key={item.position}>
      <div className="section-heading"><span className="subtle-tag">문제 {item.position + 1} · {item.criteria.type === 'descriptive' ? '서술형' : '단답형'}</span><strong>{titles[item.judgement]}</strong></div>
      <h2 className="exam-question">{item.question}</h2><h3>내 답변</h3><p className="answer-original">{item.response_text || '(빈 답변)'}</p>
      {item.criteria.type === 'short_answer' ? <><h3>대표 정답</h3><p className="answer-original">{item.criteria.answer}</p>{item.criteria.accepted_answers.length > 0 && <p>허용 표기: {item.criteria.accepted_answers.join(' / ')}</p>}</> : <><h3>참고 답안</h3><p className="answer-original">{item.criteria.reference_answer}</p><h3>확인할 핵심 내용</h3><ul>{item.criteria.key_points.map((point, i) => <li className="answer-original" key={i}>{point}</li>)}</ul></>}
      {item.original_judgement && item.original_judgement !== item.judgement && <p className="exam-note">최초 평가: {titles[item.original_judgement]} · 현재 평가: {titles[item.judgement]}</p>}
      <p className="exam-note">{item.is_review_target ? `현재 복습 대상 · 연속 정답 ${item.correct_streak}/3` : '현재 복습 대상에서 제외'}</p>
      {item.incorrect_reason === 'unanswered' ? <p className="exam-note">미응답은 오답으로 기록하며 평가를 정정할 수 없어요.</p> : <div className="exam-actions" aria-label={`${item.position + 1}번 평가`}>{(['correct', 'partial', 'incorrect'] as const).map((judgement) => <button key={judgement} disabled={disabled} className="secondary-button" onClick={() => mutate({ kind: 'grade', request: { exam_id: result.exam_id, position: item.position, expected_revision: item.assessment_revision, judgement, note: null, request_id: crypto.randomUUID() } })}>{titles[judgement]}으로 평가</button>)}</div>}
      <button disabled={disabled} className="secondary-button" onClick={() => run(async () => { const next = await getFoundationLinks(result.exam_id, item.position); setLinks((old) => ({ ...old, [item.position]: next })); })}>연결된 기초 문항 확인</button>
      {links[item.position] && <section className="foundation-links" aria-label="기초 문항">{links[item.position].length === 0 ? <p className="exam-note">연결된 기초 문항이 없어요.</p> : links[item.position].map((link, index) => <div key={`${link.dataset_id}:${link.item_id}:${index}`}>
        <p>{link.question ?? '현재 자료에서 찾을 수 없는 기초 문항'}</p>
        {link.issue ? <p className="exam-note">연결을 사용할 수 없어요: {({ Missing: '자료 없음 또는 비활성', Duplicate: '중복 연결', SelfReference: '자기 참조', Cycle: '순환 연결' })[link.issue]}</p> : <button disabled={disabled || link.is_review_target} className="secondary-button" onClick={() => mutate({ kind: 'review', request: { dataset_id: link.dataset_id, item_id: link.item_id, expected_progress_revision: link.progress_revision, request_id: crypto.randomUUID() } })}>{link.is_review_target ? '이미 복습 대상이에요' : '모르는 기초 문항 · 오답노트에 추가'}</button>}
      </div>)}</section>}
    </article>)}</div>
    <div className="exam-actions"><button disabled={disabled} className="secondary-button" onClick={() => onPrepare('general')}>같은 설정으로 학습 준비</button><button disabled={disabled} className="primary-button" onClick={() => onPrepare('wrong_only')}>현재 오답으로 학습 준비</button></div>
  </>;
}
