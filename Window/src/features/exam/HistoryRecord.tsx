import type { ExamHistoryRow } from '../../bridge/desktop';

export function HistoryRecord({ row, busy, onOpen }: { row: ExamHistoryRow; busy: boolean; onOpen: (id: string) => void }) {
  if (row.kind === 'error') return <article className="exam-panel history-card">
    <div><div className="history-status">기록 오류</div><h2>이 기록을 불러오지 못했어요</h2>
      <p role="alert">{row.error} 기록은 지우지 않고 보존해요. 다른 기록은 계속 이용할 수 있어요.</p></div>
  </article>;
  return <article className="exam-panel history-card">
    <div><div className="history-status">{row.status === 'completed' ? '완료' : row.status === 'submitted' ? '처리 중' : '진행 중'}</div><h2>{row.dataset_titles.join(' · ')}</h2>
      <p>{row.question_count}문항 · {row.status === 'completed' && row.assessment_counts ? `정답 ${row.assessment_counts.correct} · 부분정답 ${row.assessment_counts.partial} · 오답 ${row.assessment_counts.incorrect} · 미평가 ${row.assessment_counts.ungraded}` : row.status === 'completed' ? `정답 ${row.correct_count}/${row.question_count} · ${Math.round(row.correct_count! / row.question_count * 100)}%` : `응답 ${row.answered_count}/${row.question_count} · ${row.current_position + 1}번 위치`}</p>
      <small>{row.status === 'completed' ? '완료' : '마지막 저장'}: {new Date(row.completed_at ?? row.saved_at).toLocaleString('ko-KR')}</small></div>
    <button className="secondary-button" disabled={busy} onClick={() => onOpen(row.exam_id)}>{row.status === 'completed' ? row.assessment_status === 'pending' ? '평가 이어 하기' : '결과 보기' : row.status === 'submitted' ? '처리 재개' : '이어 풀기'}</button>
  </article>;
}
