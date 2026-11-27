import { HistoryRecord } from './HistoryRecord';
import { useEffect, useState } from 'react';
import { describeError, listExamHistory } from '../../bridge/desktop';
import type { ExamHistoryRow, HistoryFilter } from '../../bridge/desktop';

export function HistoryWorkspace({ ready, refresh, filter, onFilter, busy, onOpen }: {
  ready: boolean; refresh: number; filter: HistoryFilter; onFilter: (filter: HistoryFilter) => void; busy: boolean; onOpen: (id: string) => void;
}) {
  const [rows, setRows] = useState<ExamHistoryRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  const [loadedKey, setLoadedKey] = useState('');
  const requestKey = `${filter}:${refresh}:${retry}`;
  useEffect(() => {
    let cancelled = false;
    setLoading(true); setError(null);
    if (!ready) { setRows([]); setLoading(false); return; }
    listExamHistory(filter).then((next) => { if (!cancelled) { setRows(next); setLoadedKey(requestKey); } })
      .catch((cause) => { if (!cancelled) setError(describeError(cause)); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [ready, filter, refresh, retry]);

  return <>
    <div className="page-heading"><div className="eyebrow">YOUR LEARNING JOURNEY</div><h1>시험 기록</h1><p>지난 결과를 확인하고 저장한 시험을 이어 풀어보세요.</p></div>
    <div className="history-filters segmented" aria-label="시험 기록 필터">{([{ key: 'all', title: '전체' }, { key: 'in_progress', title: '진행 중' }, { key: 'submitted', title: '처리 중' }, { key: 'completed', title: '완료' }, { key: 'assessment_pending', title: '평가 대기' }] as const).map((item) => <button key={item.key} disabled={busy} aria-pressed={filter === item.key} className={filter === item.key ? 'selected' : ''} onClick={() => onFilter(item.key)}>{item.title}</button>)}</div>
    {!ready ? <p className="exam-note">Windows 앱의 저장소가 준비되면 시험 기록을 불러와요.</p>
      : error ? <div role="alert" className="error-banner">{error} <button onClick={() => setRetry((value) => value + 1)}>기록 다시 불러오기</button></div>
      : loading || loadedKey !== requestKey ? <p role="status" className="exam-note">시험 기록을 불러오고 있어요…</p>
      : rows.length === 0 ? <section className="exam-panel"><h2>해당하는 시험이 없어요</h2><p className="exam-note">학습을 시작하거나 다른 상태를 선택해보세요.</p></section>
      : <div className="history-list">{rows.map((row) => <HistoryRecord key={row.row_key} row={row} busy={busy} onOpen={onOpen} />)}</div>}
  </>;
}
