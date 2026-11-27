import { getExamResult } from './desktop';
import type { ExamSession, ExamResult } from './desktop';
import { getAssessmentView } from './assessment';
import type { AssessmentView } from './assessment';

export type MixedResult = AssessmentView & Pick<ExamSession, 'settings' | 'dataset_titles'> & { db_version: 2 };
export type ExamOutcome = ExamResult | MixedResult;
export function isMixedResult(result: ExamOutcome): result is MixedResult { return 'db_version' in result && result.db_version === 2; }
export async function getExamOutcome(session: ExamSession): Promise<ExamOutcome> {
  if (session.db_version !== 2) return getExamResult(session.exam_id);
  return { ...await getAssessmentView(session.exam_id), db_version: 2, settings: session.settings, dataset_titles: session.dataset_titles };
}
