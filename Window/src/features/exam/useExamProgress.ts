import { useLayoutEffect, useRef, useState } from 'react';
import { saveProgress } from '../../bridge/desktop';
import type { ExamSession, SubmitExamRequest } from '../../bridge/desktop';
import { AutosaveCoordinator } from './autosave';
import type { SaveState } from './autosave';

export function useExamProgress(session: ExamSession) {
  const latest = useRef<SubmitExamRequest>({ exam_id: session.exam_id, revision: session.revision, current_position: session.current_position, responses: session.items.map((item) => item.response_text) });
  const [draft, setDraft] = useState(latest.current);
  const [saveState, setSaveState] = useState<SaveState>({ phase: 'saved', persistedRevision: session.revision, error: null });
  const coordinator = useRef<AutosaveCoordinator | null>(null);

  useLayoutEffect(() => {
    const instance = new AutosaveCoordinator(latest.current, saveProgress, setSaveState);
    coordinator.current = instance;
    if (session.status !== 'in_progress') instance.pause();
    return () => { instance.dispose(); if (coordinator.current === instance) coordinator.current = null; };
  }, [session.exam_id, session.status]);

  function update(next: SubmitExamRequest, immediate = false) {
    if (!Number.isSafeInteger(next.revision)) throw new Error('저장 버전 범위를 초과했어요.');
    latest.current = next; setDraft(next); coordinator.current?.update(next, immediate);
  }
  function answer(text: string) {
    const previous = latest.current;
    update({ ...previous, revision: previous.revision + 1, responses: previous.responses.map((value, index) => index === previous.current_position ? text : value) });
  }
  function move(position: number) {
    const previous = latest.current;
    update({ ...previous, revision: previous.revision + 1, current_position: position }, true);
  }
  function snapshot() { return { ...latest.current, responses: [...latest.current.responses] }; }
  return { draft, latest, saveState, coordinator, answer, move, snapshot };
}
