import { useRef, useState } from 'react';
import { describeError } from '../../bridge/desktop';
import { updateStudyMaterial } from '../../bridge/study';
import type { EditMaterialRequest, StudyCriteria, StudyItem } from '../../bridge/study';
import { applyEditorChange, toEditorText } from './answerText';

function RawText({ label, value, disabled, maxLength, onChange }: { label: string; value: string; disabled: boolean; maxLength: number; onChange: (value: string) => void }) {
  return <label>{label}<textarea rows={4} disabled={disabled} maxLength={maxLength} value={toEditorText(value)} onChange={(event) => onChange(applyEditorChange(value, event.target.value))} /></label>;
}
export function MaterialEditor({ item, onSaved, onCancel }: { item: StudyItem; onSaved: () => void; onCancel: () => void }) {
  const [criteria, setCriteria] = useState<StudyCriteria>(item.criteria);
  const [explanation, setExplanation] = useState(item.explanation ?? '');
  const [saving, setSaving] = useState(false);
  const [uncertain, setUncertain] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const active = useRef(false);
  const pending = useRef<EditMaterialRequest | null>(null);
  const locked = saving || uncertain;
  async function save() {
    if (active.current) return;
    active.current = true; setSaving(true); setError(null);
    if (!pending.current) pending.current = { request_id: crypto.randomUUID(), dataset_id: item.dataset_id, item_id: item.item_id, expected_revision: item.dataset_revision, criteria, explanation: explanation === '' ? null : explanation };
    try { await updateStudyMaterial(pending.current); pending.current = null; setUncertain(false); onSaved(); }
    catch (cause) {
      const code = cause && typeof cause === 'object' && 'code' in cause ? cause.code : null;
      const rejected = ['ValidationError', 'MaterialConflict', 'RequestConflict'].includes(String(code));
      if (rejected) pending.current = null;
      setUncertain(!rejected); setError(describeError(cause));
    } finally { active.current = false; setSaving(false); }
  }
  return <section className="material-editor" aria-label="정답과 해설 편집">
    <h3>정답·해설 수정</h3><p>현재 자료와 앞으로 시작할 시험에 반영해요. 이미 시작한 시험의 질문·정답·판정과 학습 기록은 바꾸지 않아요.</p>
    <fieldset disabled={locked}><legend>수정할 내용</legend>
      {criteria.type === 'short_answer' ? <>
        <label>대표 정답<input type="text" maxLength={4096} value={criteria.answer} onChange={(event) => setCriteria({ ...criteria, answer: event.target.value })} /></label>
        <p>정답과 각 허용 표기는 4단어 이하예요. 공백·대소문자를 정리하지 않고 입력한 그대로 저장해요.</p>
        <h4>허용 표기</h4>{criteria.accepted_answers.map((answer, index) => <div className="material-editor-row" key={index}><label>허용 표기 {index + 1}<input type="text" maxLength={4096} value={answer} onChange={(event) => setCriteria({ ...criteria, accepted_answers: criteria.accepted_answers.map((value, i) => i === index ? event.target.value : value) })} /></label><button className="secondary-button" onClick={() => setCriteria({ ...criteria, accepted_answers: criteria.accepted_answers.filter((_, i) => i !== index) })}>표기 {index + 1} 제외</button></div>)}
        <button className="secondary-button" disabled={criteria.accepted_answers.length >= 64} onClick={() => setCriteria({ ...criteria, accepted_answers: [...criteria.accepted_answers, ''] })}>허용 표기 추가</button>
      </> : <>
        <RawText label="참고 답안" value={criteria.reference_answer} disabled={locked} maxLength={65536} onChange={(value) => setCriteria({ ...criteria, reference_answer: value })} />
        <h4>핵심 내용</h4>{criteria.key_points.map((point, index) => <div className="material-editor-row" key={index}><RawText label={`핵심 내용 ${index + 1}`} value={point} disabled={locked} maxLength={8192} onChange={(value) => setCriteria({ ...criteria, key_points: criteria.key_points.map((old, i) => i === index ? value : old) })} /><button className="secondary-button" disabled={criteria.key_points.length <= 1} onClick={() => setCriteria({ ...criteria, key_points: criteria.key_points.filter((_, i) => i !== index) })}>핵심 {index + 1} 제외</button></div>)}
        <button className="secondary-button" disabled={criteria.key_points.length >= 64} onClick={() => setCriteria({ ...criteria, key_points: [...criteria.key_points, ''] })}>핵심 내용 추가</button>
      </>}
      <RawText label="해설" value={explanation} disabled={locked} maxLength={65536} onChange={setExplanation} />
    </fieldset>
    {error && <div className="error-banner" role="alert">{error}</div>}
    {uncertain && <p role="alert">저장 결과를 확인하지 못했어요. 다른 내용을 저장하거나 이동하기 전에 같은 요청으로 재시도해주세요.</p>}
    <div className="material-editor-actions"><button className="secondary-button" disabled={saving} onClick={save}>{saving ? '저장 중…' : uncertain ? '같은 요청으로 다시 저장' : '수정 저장'}</button><button className="secondary-button" disabled={locked} onClick={onCancel}>편집 취소하고 최신 자료 보기</button></div>
  </section>;
}
