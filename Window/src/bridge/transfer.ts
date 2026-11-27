import { invoke } from '@tauri-apps/api/core';
export type ImportPreview = {
  token: string; dataset_id: string; title: string; schema_version: string; db_version: 1 | 2;
  existing: boolean; changed: boolean; question_count: number; added: number; modified: number; deactivated: number;
};
export type ImportResult = { dataset_id: string; changed: boolean; question_count: number };
export const previewImport = (learningJson: string, supplementJson: string | null) => invoke<ImportPreview>('preview_import', { request: { learning_json: learningJson, supplement_json: supplementJson } });
export const applyImport = (token: string) => invoke<ImportResult>('apply_import', { token });
export type SelectedImportFile = { name: string; content: string };
export const chooseImportFile = (kind: 'learning' | 'supplement') => invoke<SelectedImportFile | null>('choose_import_file', { kind });
// Directly authored synthetic forms, with no source/user data.
export function learningTemplate(version: '1.0' | '2.0') {
  const item = version === '1.0' ? { id: 'sample-1', question: '질문을 입력하세요.', answer: '정답', accepted_answers: [] }
    : { id: 'sample-1', type: 'descriptive', question: '설명할 질문을 입력하세요.', reference_answer: '참고 답안을 입력하세요.', key_points: ['핵심 내용을 입력하세요.'] };
  return JSON.stringify({ schema_version: version, dataset_id: 'my-study', title: '내 학습 자료', items: [item] }, null, 2) + '\n';
}
