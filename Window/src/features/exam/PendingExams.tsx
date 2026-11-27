import type { PendingExam } from '../../bridge/desktop';

export function PendingExams({ exams, busy, onResume }: { exams: PendingExam[]; busy: boolean; onResume: (id: string) => void }) {
  return <section className="pending-exams" aria-label="중단한 시험 이어 풀기">
    <div className="section-heading"><h2>이어서 학습하기</h2><span className="subtle-tag">{exams.length}개 시험</span></div>
    {exams.length === 0 ? <p className="exam-note">이어 풀거나 처리할 시험이 없어요.</p> : exams.map((exam) => exam.kind === 'error' ? <article className="pending-card" key={exam.row_key}>
      <div><strong>이 시험을 불러오지 못했어요</strong><p role="alert">{exam.error} 기록은 지우지 않고 보존해요.</p></div>
    </article> : <article className="pending-card" key={exam.row_key}>
      <div><strong>{exam.dataset_titles.join(' · ')}</strong><p>{exam.status === 'submitted' ? '제출 완료 · 채점 대기' : `진행 중 · ${exam.current_position + 1}번 위치 · ${exam.answered_count}/${exam.question_count}문항 응답`}</p><small>마지막 저장: {new Date(exam.saved_at).toLocaleString('ko-KR')}</small></div>
      <button disabled={busy} className="secondary-button" onClick={() => onResume(exam.exam_id)}>{exam.status === 'submitted' ? '처리 재개' : '이어 풀기'}</button>
    </article>)}
  </section>;
}
