import { useEffect, useRef, useState } from 'react';
import type { MutableRefObject } from 'react';
import type { ExamGuard } from './ExamWorkspace';
import { MaterialEditor } from './MaterialEditor';
import { describeError } from '../../bridge/desktop';
import type { DatasetSummary } from '../../bridge/desktop';
import { browseStudyMaterials } from '../../bridge/study';
import type { StudyItem, StudyPage } from '../../bridge/study';

export function StudyCard({ item, number }: { item: StudyItem; number: number }) {
  const criteria = item.criteria;
  return <article className="study-card" aria-label={`학습 항목 ${number}`}>
    <div className="study-card-heading"><span>{number}. {item.dataset_title}</span><span>{criteria.type === 'short_answer' ? '단답형' : '서술형'}</span></div>
    <h2>{item.question}</h2>
    {criteria.type === 'short_answer' ? <>
      <h3>정답</h3><p className="study-answer">{criteria.answer}</p>
      {criteria.accepted_answers.length > 0 && <><h3>허용 표기</h3><ul className="study-aliases">{criteria.accepted_answers.map((answer, index) => <li className="study-answer" key={index}>{answer}</li>)}</ul></>}
    </> : <>
      <h3>참고 답안</h3><p className="study-answer">{criteria.reference_answer}</p>
      <h3>핵심 내용</h3><ul>{criteria.key_points.map((point, index) => <li className="study-answer" key={index}>{point}</li>)}</ul>
    </>}
    {item.explanation && <><h3>해설</h3><p className="study-answer">{item.explanation}</p></>}
  </article>;
}

export function StudyWorkspace({ ready, datasets, refresh, onBusy, guardRef }: { ready: boolean; datasets: DatasetSummary[]; refresh: number; onBusy: (busy: boolean) => void; guardRef: MutableRefObject<ExamGuard | null> }) {
  const [editing, setEditing] = useState<StudyItem | null>(null);
  const editingRef = useRef(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [datasetId, setDatasetId] = useState('');
  const [input, setInput] = useState('');
  const [search, setSearch] = useState('');
  const [offset, setOffset] = useState(0);
  const [retry, setRetry] = useState(0);
  const [state, setState] = useState<{ key: string; page: StudyPage | null; error: string | null }>({ key: '', page: null, error: null });
  const scope = datasets.some((item) => item.dataset_id === datasetId) ? datasetId : '';
  const key = JSON.stringify([scope, search, offset, refresh, retry]);
  const page = state.key === key ? state.page : null;
  const error = state.key === key ? state.error : null;
  useEffect(() => {
    const guard: ExamGuard = { prepareToLeave: async () => { if (editingRef.current) throw new Error('자료 편집을 저장하거나 취소한 뒤 창을 닫아주세요.'); } };
    guardRef.current = guard;
    return () => { if (guardRef.current === guard) guardRef.current = null; onBusy(false); };
  }, [guardRef, onBusy]);
  function startEdit(item: StudyItem) { editingRef.current = true; setEditing(item); onBusy(true); setNotice(null); }
  function finishEdit(saved: boolean) { editingRef.current = false; setEditing(null); onBusy(false); setRetry((value) => value + 1); setNotice(saved ? '정답과 해설을 저장했어요. 기존 시험 기록은 그대로 유지했어요.' : null); }
  useEffect(() => {
    let cancelled = false;
    if (!ready || datasets.length === 0) return;
    setState({ key, page: null, error: null });
    browseStudyMaterials(scope || null, search, offset).then((next) => {
      if (!cancelled) setState({ key, page: next, error: null });
    }).catch((cause) => { if (!cancelled) setState({ key, page: null, error: describeError(cause) }); });
    return () => { cancelled = true; };
  }, [key, ready, datasets.length]);
  function move(next: number) { setOffset(next); window.scrollTo(0, 0); }
  return <>
    <div className="page-heading"><div className="eyebrow">READ & LEARN</div><h1>질문과 답을 함께 읽어요</h1><p>먼저 내용을 익힌 뒤, 준비가 되면 시험으로 확인해보세요.</p></div>
    <div className="setup-notice">자료를 읽어도 시험 기록이나 오답 상태는 바뀌지 않아요. 현재 등록된 질문과 답을 보여드려요.</div>
    {notice && <p role="status" className="setup-notice">{notice}</p>}
    <section className="study-controls" aria-label="자료 학습 범위">
      <label>학습할 자료<select disabled={!ready || !!editing} value={scope} onChange={(event) => { setDatasetId(event.target.value); setOffset(0); }}><option value="">전체 자료</option>{datasets.map((item) => <option key={item.dataset_id} value={item.dataset_id}>{item.title} · {item.question_count}문항</option>)}</select></label>
      <form onSubmit={(event) => { event.preventDefault(); setSearch(input); setOffset(0); }}>
        <label>질문 검색<input type="search" maxLength={128} value={input} disabled={!ready || !!editing} placeholder="찾고 싶은 질문이나 용어" onChange={(event) => setInput(event.target.value)} /></label>
        <button disabled={!ready || !!editing} type="submit">검색</button>{(search || input) && <button disabled={!!editing} type="button" onClick={() => { setInput(''); setSearch(''); setOffset(0); }}>검색 지우기</button>}
      </form>
    </section>
    {!ready ? <p role="status">Windows 앱의 학습 자료 연결을 확인해주세요.</p> : datasets.length === 0 ? <p role="status">등록된 자료가 없어요. 자료 추가 메뉴에서 학습 자료를 준비해주세요.</p> : error ? <div className="error-banner" role="alert">{error} <button onClick={() => setRetry((value) => value + 1)}>다시 불러오기</button></div> : !page ? <p role="status">질문과 답을 불러오고 있어요…</p> : <>
      <p role="status" className="study-count">{page.total === 0 ? '조건에 맞는 질문이 없어요. 자료나 검색어를 바꿔보세요.' : `총 ${page.total}문항 · ${page.offset + 1}–${page.offset + page.items.length}번`}</p>
      <div className="study-cards">{page.items.map((item, index) => <div className="study-item" key={JSON.stringify([item.dataset_id, item.item_id])}><StudyCard item={item} number={page.offset + index + 1} />{editing?.dataset_id === item.dataset_id && editing.item_id === item.item_id ? <MaterialEditor item={editing} onSaved={() => finishEdit(true)} onCancel={() => finishEdit(false)} /> : <button className="secondary-button" disabled={!!editing} onClick={() => startEdit(item)}>정답·해설 수정</button>}</div>)}</div>
      {page.total > 0 && <nav className="study-pagination" aria-label="자료 페이지"><button disabled={!!editing || page.offset === 0} onClick={() => move(Math.max(0, page.offset - page.limit))}>이전 페이지</button><span>{Math.floor(page.offset / page.limit) + 1} / {Math.ceil(page.total / page.limit)} 페이지</span><button disabled={!!editing || page.offset + page.items.length >= page.total} onClick={() => move(page.offset + page.limit)}>다음 페이지</button></nav>}
    </>}
  </>;
}
