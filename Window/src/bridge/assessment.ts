import { invoke } from '@tauri-apps/api/core';

// Backend v2 contract. The current v1 app reports UnsupportedDbVersion; no automatic migration.
export type Judgement = 'correct' | 'partial' | 'incorrect';
export type Criteria = { type: 'short_answer'; answer: string; accepted_answers: string[] }
  | { type: 'descriptive'; reference_answer: string; key_points: string[] };
export type AssessmentView = {
  exam_id: string; result_revision: number; assessment_status: 'pending' | 'complete';
  counts: { correct: number; partial: number; incorrect: number; ungraded: number };
  items: { position: number; question: string; response_text: string; criteria: Criteria;
    original_judgement: Judgement | null; judgement: Judgement | 'ungraded';
    incorrect_reason: 'no_match' | 'unanswered' | 'user' | null; assessment_revision: number;
    is_review_target: boolean; correct_streak: number; progress_revision: number }[];
};
export type SetAssessmentRequest = { exam_id: string; position: number; expected_revision: number;
  judgement: Judgement; note: string | null; request_id: string };
export type AddReviewRequest = { dataset_id: string; item_id: string; expected_progress_revision: number; request_id: string };
export type MutationResult = { outcome: 'saved' | 'unchanged' | 'already_applied';
  assessment_revision: number | null; progress_revision: number; result_revision: number | null };

export const getAssessmentView = (examId: string) => invoke<AssessmentView>('get_assessment_view', { examId });
export const setResponseAssessment = (request: SetAssessmentRequest) => invoke<MutationResult>('set_response_assessment', { request });
export const addReviewTarget = (request: AddReviewRequest) => invoke<MutationResult>('add_review_target', { request });
export type FoundationLink = { dataset_id: string; item_id: string; question: string | null;
  issue: 'Missing' | 'Duplicate' | 'SelfReference' | 'Cycle' | null; is_review_target: boolean; progress_revision: number };
export const getFoundationLinks = (examId: string, position: number) => invoke<FoundationLink[]>('get_foundation_links', { examId, position });
