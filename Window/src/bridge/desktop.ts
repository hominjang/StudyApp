import { invoke, isTauri } from '@tauri-apps/api/core';

export type AppStatus = {
  runtime: 'desktop' | 'browser_preview';
  storage: 'not_initialized' | 'ready';
  dataset_count: number;
  question_count: number;
};

export type DatasetSummary = {
  dataset_id: string;
  title: string;
  position: number;
  question_count: number;
  review_target_count: number;
};

export type AppError = { code: string; message: string; field_path?: string | null; retryable: boolean };

export function describeError(error: unknown): string {
  if (error && typeof error === 'object' && 'message' in error && typeof error.message === 'string') {
    const field = 'field_path' in error && typeof error.field_path === 'string' ? ` (${error.field_path})` : '';
    return error.message + field;
  }
  return '학습 자료를 불러오지 못했어요. 다시 시도해주세요.';
}

export async function getAppStatus(): Promise<AppStatus> {
  if (!isTauri()) return { runtime: 'browser_preview', storage: 'not_initialized', dataset_count: 0, question_count: 0 };
  return invoke<AppStatus>('get_app_status');
}

export async function listDatasets(): Promise<DatasetSummary[]> {
  if (!isTauri()) return [];
  return invoke<DatasetSummary[]>('list_datasets');
}

export type ReviewMode = 'general' | 'wrong_first' | 'wrong_only';
export type StartExamRequest = {
  exam_id: string; dataset_ids: string[]; count_mode: 'count' | 'all'; count: number | null;
  order_mode: 'random' | 'ordered'; review_mode: ReviewMode;
};
export type SessionItem = { position: number; dataset_id: string; item_id: string; question: string; response_text: string };
export type ExamSession = {
  exam_id: string; status: 'in_progress' | 'submitted' | 'completed'; revision: number;
  current_position: number; settings: StartExamRequest; dataset_titles: string[]; items: SessionItem[];
};
export type SubmitExamRequest = { exam_id: string; revision: number; current_position: number; responses: string[] };
export type ResultItem = {
  position: number; question: string; response_text: string; answer: string; accepted_answers: string[];
  judgement: 'correct' | 'incorrect'; incorrect_reason: 'unanswered' | 'no_match' | null;
  is_review_target: boolean; correct_streak: number; supplement: Record<string, unknown> | null;
};
export type ExamResult = {
  exam_id: string; completed_at: string; completion_order: number; settings: StartExamRequest;
  dataset_titles: string[]; correct_count: number; incorrect_count: number; unanswered_count: number; items: ResultItem[];
};

export const startExam = (request: StartExamRequest) => invoke<ExamSession>('start_exam', { request });
export const getExamSession = (examId: string) => invoke<ExamSession>('get_exam_session', { examId });
export const submitExam = (request: SubmitExamRequest) => invoke<ExamSession>('submit_exam', { request });
export const completeExam = (examId: string) => invoke<ExamSession>('complete_exam', { examId });
export const getExamResult = (examId: string) => invoke<ExamResult>('get_exam_result', { examId });

export type SaveProgressResult = { outcome: 'saved' | 'already_saved' | 'stale' | 'locked'; revision: number; status: ExamSession['status'] };
export type PendingExam = { exam_id: string; status: 'in_progress' | 'submitted'; dataset_titles: string[]; question_count: number; answered_count: number; current_position: number; saved_at: string; error: string | null };
export const saveProgress = (request: SubmitExamRequest) => invoke<SaveProgressResult>('save_progress', { request });
export const listPendingExams = async () => isTauri() ? invoke<PendingExam[]>('list_pending_exams') : [];

export type CandidatePreview = { candidate_count: number; total_count: number; review_target_count: number };
export type HistoryFilter = 'all' | 'in_progress' | 'submitted' | 'completed';
export type ExamHistoryRow = {
  exam_id: string; status: ExamSession['status']; dataset_titles: string[]; question_count: number;
  answered_count: number; current_position: number; saved_at: string; completed_at: string | null;
  completion_order: number | null; correct_count: number | null; error: string | null;
};
export const previewCandidates = (datasetIds: string[], reviewMode: ReviewMode) => invoke<CandidatePreview>('preview_candidates', { request: { dataset_ids: datasetIds, review_mode: reviewMode } });
export const listExamHistory = (filter: HistoryFilter) => invoke<ExamHistoryRow[]>('list_exam_history', { filter });

export async function watchCloseRequest(handler: () => Promise<void>, onError: (error: unknown) => void) {
  if (!isTauri()) return () => {};
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  const window = getCurrentWindow();
  let closing = false;
  return window.onCloseRequested(async (event) => {
    event.preventDefault();
    if (closing) return;
    closing = true;
    try { await handler(); await window.destroy(); }
    catch (error) { onError(error); }
    finally { closing = false; }
  });
}
