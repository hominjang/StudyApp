import { useEffect, useRef, useState } from 'react';
import { describeError, getAppStatus, listDatasets, startExam, listPendingExams, getExamSession, getExamResult, watchCloseRequest, previewCandidates } from './bridge/desktop';
import type { AppStatus, DatasetSummary, ExamSession, ExamResult, StartExamRequest, PendingExam, CandidatePreview, HistoryFilter } from './bridge/desktop';
import { ExamWorkspace, ResultWorkspace } from './features/exam/ExamWorkspace';
import type { ExamGuard } from './features/exam/ExamWorkspace';
import { PendingExams } from './features/exam/PendingExams';
import { HistoryWorkspace } from './features/exam/HistoryWorkspace';

type Page = 'study' | 'history';
type ReviewMode = 'general' | 'wrong_first' | 'wrong_only';

export function App() {
  const [page, setPage] = useState<Page>('study');
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [connectionError, setConnectionError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [datasets, setDatasets] = useState<DatasetSummary[]>([]);
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [retry, setRetry] = useState(0);
  const [countMode, setCountMode] = useState<'count' | 'all'>('count');
  const [count, setCount] = useState('10');
  const [order, setOrder] = useState<'random' | 'ordered'>('random');
  const [review, setReview] = useState<ReviewMode>('general');
  const [exam, setExam] = useState<ExamSession | null>(null);
  const [result, setResult] = useState<ExamResult | null>(null);
  const [starting, setStarting] = useState(false);
  const [startError, setStartError] = useState<string | null>(null);
  const startRequest = useRef<{ key: string; request: StartExamRequest } | null>(null);
  const startActive = useRef(false);
  const [pendingExams, setPendingExams] = useState<PendingExam[]>([]);
  const [pendingError, setPendingError] = useState<string | null>(null);
  const [resuming, setResuming] = useState(false);
  const [closing, setClosing] = useState(false);
  const [closeError, setCloseError] = useState<string | null>(null);
  const [closeReady, setCloseReady] = useState(false);
  const examGuard = useRef<ExamGuard | null>(null);
  const resumeActive = useRef(false);
  const [historyFilter, setHistoryFilter] = useState<HistoryFilter>('all');
  const [setupNotice, setSetupNotice] = useState<string | null>(null);
  const [candidateState, setCandidateState] = useState<{ key: string; preview: CandidatePreview | null; error: string | null }>({ key: '', preview: null, error: null });
  const candidateKey = JSON.stringify([selectedIds, review, retry]);

  function navigate(next: Page) {
    if (exam || starting || resuming) return;
    if (result) { setResult(null); setRetry((value) => value + 1); }
    setPage(next); setStartError(null);
  }
  async function beginExam() {
    if (startActive.current || !closeReady) return;
    startActive.current = true; setStarting(true); setStartError(null);
    try {
      const settings = { dataset_ids: [...selectedIds], count_mode: countMode, count: countMode === 'all' ? null : Number(count), order_mode: order, review_mode: review };
      const key = JSON.stringify(settings);
      if (startRequest.current?.key !== key) startRequest.current = { key, request: { exam_id: crypto.randomUUID(), ...settings } };
      const session = await startExam(startRequest.current.request);
      setExam(session); startRequest.current = null;
    } catch (cause) { setStartError(describeError(cause)); }
    finally { startActive.current = false; setStarting(false); }
  }

  async function resumeExam(id: string) {
    if (resumeActive.current || !closeReady) return;
    resumeActive.current = true; setResuming(true); setStartError(null);
    try {
      // 이어 풀기와 결과 보기는 학습 시작 화면에 입력해 둔 설정을 바꾸지 않는다.
      const saved = await getExamSession(id);
      if (saved.status === 'completed') setResult(await getExamResult(id)); else setExam(saved);
      setPage('study');
    } catch (cause) { setStartError(describeError(cause)); }
    finally { resumeActive.current = false; setResuming(false); }
  }

  function prepareAgain(mode: ReviewMode) {
    if (!result) return;
    const ids = result.settings.dataset_ids;
    const missing = ids.flatMap((id, index) => datasets.some((item) => item.dataset_id === id) ? [] : [result.dataset_titles[index]]);
    setSelectedIds(ids.filter((id) => datasets.some((item) => item.dataset_id === id)));
    setCountMode(result.settings.count_mode); setCount(String(result.settings.count ?? result.items.length)); setOrder(result.settings.order_mode); setReview(mode);
    setSetupNotice(missing.length > 0 ? `현재 목록에 없는 자료: ${missing.join(' · ')}. 사용할 수 있는 자료와 문제 수를 다시 확인해주세요.` : mode === 'wrong_only' ? '원래 자료 범위의 현재 누적 오답 대상으로 새 시험을 준비해요.' : null);
    setResult(null); setPage('study'); setStartError(null); setRetry((value) => value + 1);
  }

  useEffect(() => {
    let disposed = false; let unlisten: (() => void) | undefined;
    watchCloseRequest(async () => { setClosing(true); setCloseError(null); await examGuard.current?.prepareToLeave(); },
      (cause) => { setClosing(false); setCloseError(`창을 닫지 않았어요. ${describeError(cause)}`); })
      .then((stop) => { if (disposed) stop(); else { unlisten = stop; setCloseReady(true); } })
      .catch((cause) => { setCloseError(`종료 전 저장 기능을 준비하지 못했어요. ${describeError(cause)}`); });
    return () => { disposed = true; unlisten?.(); };
  }, []);

  useEffect(() => { window.scrollTo(0, 0); }, [exam?.exam_id, result?.exam_id, page]);

  useEffect(() => {
    let cancelled = false;
    setConnectionError(null);
    setLoading(true);
    (async () => {
      const nextStatus = await getAppStatus();
      const ready = nextStatus.storage === 'ready';
      // 미완료 시험 조회 실패가 자료 목록과 새 시험 시작을 막지 않도록 따로 불러온다.
      const [nextDatasets, pendingResult] = ready
        ? await Promise.all([listDatasets(), listPendingExams().then((exams) => ({ exams, error: null }), (cause: unknown) => ({ exams: [] as PendingExam[], error: describeError(cause) }))])
        : [[], { exams: [] as PendingExam[], error: null }];
      if (cancelled) return;
      setStatus(nextStatus);
      setDatasets(nextDatasets);
      setPendingExams(pendingResult.exams);
      setPendingError(pendingResult.error);
      setSelectedIds((previous) => previous.filter((id) => nextDatasets.some((item) => item.dataset_id === id)));
    })().catch((error: unknown) => { if (!cancelled) setConnectionError(describeError(error)); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [retry]);

  useEffect(() => {
    let cancelled = false;
    if (status?.storage !== 'ready' || selectedIds.length === 0 || exam || result || page !== 'study') return;
    setCandidateState({ key: candidateKey, preview: null, error: null });
    previewCandidates(selectedIds, review).then((preview) => { if (!cancelled) setCandidateState({ key: candidateKey, preview, error: null }); })
      .catch((cause) => { if (!cancelled) setCandidateState({ key: candidateKey, preview: null, error: describeError(cause) }); });
    return () => { cancelled = true; };
  }, [candidateKey, status?.storage, exam, result, page]);

  const selected = selectedIds.flatMap((id) => datasets.filter((item) => item.dataset_id === id));
  const candidateInfo = candidateState.key === candidateKey ? candidateState.preview : null;
  const candidateError = candidateState.key === candidateKey ? candidateState.error : null;
  const candidateLoading = selected.length > 0 && status?.storage === 'ready' && !candidateInfo && !candidateError;
  const availableCount = candidateInfo?.candidate_count ?? 0;
  const invalidCount = countMode === 'count' && availableCount > 0 && (!/^\d+$/.test(count) || Number(count) < 1 || Number(count) > availableCount);
  function toggleDataset(id: string) {
    setSetupNotice(null);
    setSelectedIds((previous) => previous.includes(id) ? previous.filter((value) => value !== id) : [...previous, id]);
  }
  function moveDataset(index: number, delta: number) {
    setSelectedIds((previous) => {
      const next = [...previous];
      if (index + delta < 0 || index + delta >= next.length) return previous;
      [next[index], next[index + delta]] = [next[index + delta], next[index]];
      return next;
    });
  }

  return (
    <div className="app-shell">
      <aside className="sidebar" inert={closing}>
        <a className="brand" href="#" onClick={(event) => { event.preventDefault(); navigate('study'); }}>
          <span className="brand-mark" aria-hidden="true">s<span>·</span></span>
          <span>StudyApp<small>나만의 학습 공간</small></span>
        </a>
        <div className="nav-label">WORKSPACE</div>
        <nav aria-label="주 메뉴">
          <button disabled={!!exam || starting} className={page === 'study' ? 'nav-item active' : 'nav-item'} aria-current={page === 'study' ? 'page' : undefined} onClick={() => navigate('study')}><span aria-hidden="true">▦</span> 학습 시작</button>
          <button disabled={!!exam || starting} className={page === 'history' ? 'nav-item active' : 'nav-item'} aria-current={page === 'history' ? 'page' : undefined} onClick={() => navigate('history')}><span aria-hidden="true">◷</span> 시험 기록</button>
        </nav>
        <div className="sidebar-note"><span className="status-dot" /> 내 PC의 학습 공간<p>선택한 자료로 배우고,<br />나의 속도로 반복하세요.</p></div>
      </aside>

      <main>
        <header className="topbar"><span>내 학습 공간 <span className="breadcrumb">/ {result ? '시험 결과' : exam ? '문제 풀이' : page === 'study' ? '학습 시작' : '시험 기록'}</span></span><span className="preview-tag">{exam ? '시험 진행 중' : status?.storage === 'ready' ? '내 PC에 저장됨' : '화면 미리보기'}</span></header>
        {closing && <div className="close-notice" role="status">최신 저장 상태를 확인한 뒤 창을 닫고 있어요…</div>}
        {closeError && <div className="error-banner" role="alert">{closeError}</div>}
        <div className="page-content" inert={closing}>
          {connectionError && <div className="error-banner" role="alert">{connectionError} <button onClick={() => setRetry((value) => value + 1)}>다시 확인</button></div>}
          {startError && !exam && <div role="alert" className="error-banner">{startError}</div>}
          {result ? <ResultWorkspace result={result} onPrepare={prepareAgain} onHistory={() => navigate('history')} /> : exam ? <ExamWorkspace key={exam.exam_id} session={exam} guardRef={examGuard} onLeave={() => { setExam(null); setResult(null); setRetry((value) => value + 1); setPage('study'); }} onDone={(completed) => { setResult(completed); setExam(null); setRetry((value) => value + 1); }} /> : page === 'study' ? <>
            <div className="page-heading"><div className="eyebrow">YOUR NEXT STEP</div><h1>오늘은 무엇을 배워볼까요?</h1><p>학습할 자료와 출제 방식을 선택해 나만의 시험을 준비하세요.</p></div>
            {setupNotice && <div className="setup-notice" role="status">{setupNotice}</div>}
            {!loading && pendingError && <div className="error-banner" role="alert">이어 풀 시험 목록을 불러오지 못했어요. {pendingError} <button onClick={() => setRetry((value) => value + 1)}>다시 확인</button></div>}
            {!loading && pendingExams.length > 0 && <PendingExams exams={pendingExams} busy={resuming || starting || !closeReady} onResume={resumeExam} />}
            <div className="workspace-grid" aria-busy={starting || resuming} inert={starting || resuming}>
              <section className="materials" aria-labelledby="materials-title">
                <div className="section-heading"><h2 id="materials-title">학습 자료</h2><span className="subtle-tag">{loading ? '불러오는 중' : status?.storage === 'ready' ? `${datasets.length}개 자료 · ${status.question_count}문항` : '미리보기'}</span></div>
                <p className="section-description">하나 이상의 자료를 선택해 함께 학습할 수 있어요.</p>
                {loading ? <div className="empty-materials" role="status"><h3>학습 자료를 불러오고 있어요</h3><p>처음 실행하면 로컬 자료를 한 번 등록해요.</p></div>
                  : datasets.length > 0 ? <div className="dataset-list">{datasets.map((item) => <label key={item.dataset_id} className={selectedIds.includes(item.dataset_id) ? 'dataset-card checked' : 'dataset-card'}><input type="checkbox" checked={selectedIds.includes(item.dataset_id)} onChange={() => toggleDataset(item.dataset_id)} /><span className="dataset-card-copy"><strong>{item.title}</strong><small>{item.question_count}문항 · 오답 대상 {item.review_target_count}문항</small></span><span className="dataset-icon" aria-hidden="true">▤</span></label>)}
                    {selected.length > 0 && <section className="selected-range" aria-label="선택한 자료의 순서"><h3>선택한 범위 <span>{candidateLoading ? '확인 중…' : availableCount + '문항'}</span></h3><ol>{selected.map((item, index) => <li key={item.dataset_id}><span>{item.title}</span><button aria-label={`${item.title} 위로 이동`} disabled={index === 0} onClick={() => moveDataset(index, -1)}>↑</button><button aria-label={`${item.title} 아래로 이동`} disabled={index === selected.length - 1} onClick={() => moveDataset(index, 1)}>↓</button></li>)}</ol><p>순서대로 출제할 때 이 자료 순서를 따라요.</p></section>}
                  </div>
                  : <div className="empty-materials"><div className="empty-icon" aria-hidden="true"><span /><span /><span /></div><h3>{connectionError ? '자료를 불러오지 못했어요' : '로컬 자료를 연결할 공간이에요'}</h3><p>{connectionError ? '기존 자료를 지우지 않고 오류를 확인한 뒤 다시 시도해요.' : 'Windows 앱을 실행하면 내 PC의 학습 자료를 불러와요.'}</p><span className="quiet-chip">{connectionError ? '위의 오류 안내를 확인해주세요' : '브라우저에서는 화면만 미리 볼 수 있어요'}</span></div>}
                <div className="learning-tip"><span className="tip-mark" aria-hidden="true">✦</span><div><strong>짧게 답하고, 차근차근 복습해요.</strong><p>정답과 허용 표기에 정확히 맞춰 답하고, 제출 후 결과를 한 번에 확인해요.</p></div></div>
              </section>
              <section className="exam-settings" aria-labelledby="settings-title">
                <div className="section-heading"><h2 id="settings-title">시험 설정</h2><span className="step-label">준비하기</span></div>
                <fieldset><legend>문제 수</legend><div className="segmented" aria-label="문제 수 선택 방식"><button aria-pressed={countMode === 'count'} className={countMode === 'count' ? 'selected' : ''} onClick={() => setCountMode('count')}>직접 지정</button><button aria-pressed={countMode === 'all'} className={countMode === 'all' ? 'selected' : ''} onClick={() => setCountMode('all')}>전체 출제</button></div>{countMode === 'count' && <label className="count-input"><input aria-label="출제 문제 수" type="number" min="1" max={availableCount || undefined} aria-invalid={invalidCount} step="1" value={count} onChange={(event) => setCount(event.target.value)} /><span>문항</span></label>}</fieldset>
                <fieldset><legend>표시 순서</legend><div className="segmented"><button aria-pressed={order === 'random'} className={order === 'random' ? 'selected' : ''} onClick={() => setOrder('random')}>무작위</button><button aria-pressed={order === 'ordered'} className={order === 'ordered' ? 'selected' : ''} onClick={() => setOrder('ordered')}>순서대로</button></div></fieldset>
                <fieldset><legend>학습 방식</legend><div className="review-options">{([{ key: 'general', title: '일반 학습', note: '선택한 범위에서 골고루 출제' }, { key: 'wrong_first', title: '오답 우선', note: '오답을 먼저 고르고 부족하면 일반 문항 보충' }, { key: 'wrong_only', title: '오답만', note: '현재 오답 대상만 집중 학습' }] as const).map((item) => <label key={item.key} className={review === item.key ? 'review-option chosen' : 'review-option'}><input type="radio" name="review" value={item.key} checked={review === item.key} onChange={() => { setReview(item.key); setSetupNotice(null); }} /><span><strong>{item.title}</strong><small>{item.note}</small></span></label>)}</div></fieldset>
                <div className="candidate-summary" role="status">{selected.length === 0 ? '학습 자료를 선택해주세요.' : candidateLoading ? '현재 출제 가능한 문항을 확인하고 있어요…' : candidateError ? '출제 범위를 확인하지 못했어요.' : availableCount === 0 ? review === 'wrong_only' ? '현재 선택 범위에 오답 대상이 없어요. 다른 모드나 자료를 선택해주세요.' : '선택한 범위에 출제할 문항이 없어요.' : `현재 조건으로 ${availableCount}문항을 선택할 수 있어요.`}{candidateInfo && review !== 'general' && <p>현재 오답 대상 {candidateInfo.review_target_count}문항 · 범위 전체 {candidateInfo.total_count}문항</p>}{candidateInfo && review === 'wrong_first' && candidateInfo.review_target_count === 0 && candidateInfo.total_count > 0 && <p>현재 오답 대상이 없어 일반 문항으로 구성해요.</p>}{invalidCount && <p className="count-error">출제 가능한 범위 안에서 1 이상의 정수를 입력해주세요.</p>}</div>
                {candidateError && <div role="alert" className="error-banner">{candidateError} <button onClick={() => setRetry((value) => value + 1)}>범위 다시 확인</button></div>}
                {startError && <div className="error-banner" role="alert">{startError} 같은 설정으로 다시 시작하면 기존 시험을 확인해요.</div>}
                <div className="start-area"><button className="primary-button" disabled={!closeReady || starting || loading || candidateLoading || !!candidateError || !!connectionError || status?.storage !== 'ready' || selected.length === 0 || availableCount === 0 || invalidCount} onClick={beginExam}>{starting ? '시험 준비 중…' : '시험 시작'} <span aria-hidden="true">→</span></button><p>{selected.length === 0 ? '먼저 학습할 자료를 선택해주세요.' : '선택한 조건으로 새 시험을 시작해요.'}</p></div>
              </section>
            </div>
          </> : <HistoryWorkspace ready={status?.storage === 'ready' && !connectionError} refresh={retry} filter={historyFilter} onFilter={setHistoryFilter} busy={resuming || starting || !closeReady} onOpen={resumeExam} />}
          <footer>한 걸음씩, 쌓이는 학습.</footer>
        </div>
      </main>
    </div>
  );
}
