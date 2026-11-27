import { invoke } from '@tauri-apps/api/core';
export type StudyCriteria = { type: 'short_answer'; answer: string; accepted_answers: string[] }
  | { type: 'descriptive'; reference_answer: string; key_points: string[] };
export type StudyItem = { dataset_id: string; item_id: string; dataset_revision: number; dataset_title: string; question: string; criteria: StudyCriteria; explanation: string | null };
export type StudyPage = { total: number; offset: number; limit: number; items: StudyItem[] };
export const browseStudyMaterials = (datasetId: string | null, search: string, offset: number) => invoke<StudyPage>('browse_study_materials', {
  request: { dataset_id: datasetId, search, offset, limit: 20 },
});
export type EditMaterialRequest = { request_id: string; dataset_id: string; item_id: string; expected_revision: number; criteria: StudyCriteria; explanation: string | null };
export type EditMaterialResult = { dataset_id: string; item_id: string; dataset_revision: number; changed: boolean };
export const updateStudyMaterial = (request: EditMaterialRequest) => invoke<EditMaterialResult>('update_study_material', { request });
