import { useState } from 'react';
import { applyEditorChange, needsMultilineEditor, toEditorText } from './answerText';

export function AnswerInput({ initialValue, value, disabled, onChange, mode = 'short_answer' }: {
  initialValue: string; value: string; disabled: boolean; onChange: (value: string) => void; mode?: 'short_answer' | 'descriptive';
}) {
  // 편집 중 줄바꿈을 지워도 DOM 종류를 바꾸지 않아 포커스와 한글 조합을 유지한다.
  const [multiline] = useState(() => mode === 'descriptive' || needsMultilineEditor(initialValue));
  const shared = { id: 'exam-answer', className: 'answer-input', 'aria-label': '내 답변', disabled,
    autoComplete: 'off', autoCorrect: 'off', autoCapitalize: 'off', spellCheck: false,
    placeholder: mode === 'descriptive' ? '내 말로 설명해주세요' : '단어나 짧은 문장으로 답해주세요' };
  if (!multiline) return <input {...shared} type="text" value={value} onChange={(event) => onChange(event.target.value)} />;
  return <>
    <textarea {...shared} rows={4} aria-describedby={mode === 'descriptive' ? undefined : 'restored-answer-note'} value={toEditorText(value)} onChange={(event) => {
      const next = applyEditorChange(value, event.target.value);
      if (next !== value) onChange(next);
    }} />
    {mode !== 'descriptive' && <p id="restored-answer-note" className="exam-note">이전에 저장한 여러 줄 답변을 복원했어요. 줄바꿈도 채점에 반영되며, 수정하지 않으면 저장한 원문 그대로 제출돼요.</p>}
  </>;
}
