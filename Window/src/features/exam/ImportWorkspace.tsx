import { useEffect, useRef, useState } from 'react';
import type { MutableRefObject } from 'react';
import type { ExamGuard } from './ExamWorkspace';
import { describeError } from '../../bridge/desktop';
import { applyImport, chooseImportFile, learningTemplate, previewImport } from '../../bridge/transfer';
import type { ImportPreview, ImportResult, SelectedImportFile } from '../../bridge/transfer';

export function ImportSummary({ preview }: { preview: ImportPreview }) {
  return <section className="import-summary" aria-label="자료 적용 미리보기">
    <h2>{preview.title}</h2><p>{preview.existing ? '기존 자료 갱신' : '새 자료 추가'} · {preview.question_count}문항 · 형식 {preview.schema_version}</p>
    <dl><div><dt>추가·재활성화</dt><dd>{preview.added}문항</dd></div><div><dt>질문·정답 변경</dt><dd>{preview.modified}문항</dd></div><div><dt>비활성화</dt><dd>{preview.deactivated}문항</dd></div></dl>
    {preview.existing && <p className="import-warning">갱신 파일의 문항 목록이 앞으로 출제할 전체 목록이 됩니다. 빠진 문항은 비활성화하며 기존 시험·답변·학습 기록은 보존해요. 보충 정보나 순서만 바뀌어도 갱신될 수 있어요.</p>}
    {!preview.changed && <p role="status">현재 자료와 같아요. 중복으로 추가하지 않아요.</p>}
  </section>;
}

export function ImportWorkspace({ ready, onBusy, guardRef, onImported }: {
  ready: boolean; onBusy: (value: boolean) => void; guardRef: MutableRefObject<ExamGuard | null>; onImported: (result: ImportResult) => void;
}) {
  const [learning, setLearning] = useState<SelectedImportFile | null>(null);
  const [supplement, setSupplement] = useState<SelectedImportFile | null>(null);
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [uncertain, setUncertain] = useState(false);
  const active = useRef(false); const unresolved = useRef(false);
  useEffect(() => { onBusy(busy || uncertain); }, [busy, uncertain, onBusy]);
  useEffect(() => {
    const guard: ExamGuard = { prepareToLeave: async () => {
      if (active.current || unresolved.current) throw new Error('자료 적용 상태를 확인한 뒤 창을 닫아주세요. 같은 요청으로 다시 시도할 수 있어요.');
    } };
    guardRef.current = guard;
    return () => { if (guardRef.current === guard) guardRef.current = null; onBusy(false); };
  }, [guardRef, onBusy]);
  function clearPreview() { setPreview(null); setError(null); }
  async function choose(kind: 'learning' | 'supplement') {
    if (active.current || unresolved.current) return;
    active.current = true; setBusy(true); setError(null);
    try {
      const file = await chooseImportFile(kind);
      if (file) { if (kind === 'learning') setLearning(file); else setSupplement(file); clearPreview(); }
    } catch (cause) { setPreview(null); setError(describeError(cause)); }
    finally { active.current = false; setBusy(false); }
  }
  async function inspect() {
    if (!learning || active.current || unresolved.current) return;
    active.current = true; setBusy(true); setError(null); setPreview(null);
    try {
      setPreview(await previewImport(learning.content, supplement?.content ?? null));
    } catch (cause) { setError(describeError(cause)); }
    finally { active.current = false; setBusy(false); }
  }
  async function apply() {
    if (!preview || active.current) return;
    active.current = true; setBusy(true); setError(null);
    try {
      const result = await applyImport(preview.token);
      unresolved.current = false; setUncertain(false); onImported(result);
    } catch (cause) {
      setError(describeError(cause));
      const conflict = !!cause && typeof cause === 'object' && 'code' in cause && cause.code === 'ImportConflict';
      unresolved.current = !conflict; setUncertain(!conflict);
      if (conflict) setPreview(null);
    } finally { active.current = false; setBusy(false); }
  }
  function download(version: '1.0' | '2.0') {
    const url = URL.createObjectURL(new Blob([learningTemplate(version)], { type: 'application/json;charset=utf-8' }));
    const anchor = document.createElement('a'); anchor.href = url; anchor.download = `learning-${version}.json`; anchor.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
  return <>
    <div className="page-heading"><div className="eyebrow">YOUR MATERIALS</div><h1>문제를 추가해볼까요?</h1><p>준비한 학습 자료 파일을 확인한 뒤 내 PC에 저장해요.</p></div>
    <section className="import-workspace" aria-label="학습 자료 가져오기">
      <p>학습 JSON 파일 하나를 선택하세요. 보충 파일은 해설·기초 문항 연결이 있을 때만 추가해요. 원문 문서나 시험 기록은 가져올 수 없어요. 파일을 고쳤다면 다시 선택해주세요.</p>
      <p>단답형은 정답·허용 표기 각각 4단어 이하, 문자열 완전 일치로 채점해요. 서술형은 참고 답안과 핵심 내용을 준비해 제출 후 직접 평가해요. 기존 저장소는 1.0 단답형만 지원하며, 신규 확장 저장소는 1.0·2.0을 지원해요.</p>
      {!ready && <p role="status">Windows 앱의 저장소 연결을 확인한 뒤 자료를 추가할 수 있어요.</p>}
      <fieldset disabled={!ready || busy || uncertain}>
        <legend>자료 파일 선택 · 각각 UTF-8 JSON, 16 MiB 이하</legend>
        <div className="import-file"><span>학습 파일 (필수)</span><button className="secondary-button" aria-label="학습 파일 선택" onClick={() => choose('learning')}>파일 선택</button><span>{learning?.name ?? '선택된 파일 없음'}</span></div>
        <div className="import-file"><span>보충 파일 (선택)</span><button className="secondary-button" aria-label="보충 파일 선택" onClick={() => choose('supplement')}>파일 선택</button><span>{supplement?.name ?? '선택된 파일 없음'}</span>{supplement && <button className="secondary-button" onClick={() => { setSupplement(null); clearPreview(); }}>보충 파일 제외</button>}</div>
        <div className="import-actions"><button className="secondary-button" disabled={!learning} onClick={inspect}>내용 확인</button><button className="secondary-button" onClick={() => download('1.0')}>단답형 양식 받기</button><button className="secondary-button" onClick={() => download('2.0')}>서술형 양식 받기</button></div>
      </fieldset>
      <p className="section-description">양식의 자료 ID는 자료마다 다르게, 문항 ID는 같은 자료 안에서 다르게 정하세요. 소문자 영문·숫자와 사이의 하이픈만 사용하며 1~64자예요. 같은 자료 ID를 선택하면 기존 자료 갱신으로 안내해요.</p>
      {busy && <p role="status">자료를 확인하고 저장 상태를 확인하고 있어요…</p>}
      {error && <div className="error-banner" role="alert">{error}</div>}
      {uncertain && <p role="alert">저장 결과를 아직 확인하지 못했어요. 파일을 바꾸거나 이동하기 전에 같은 요청으로 다시 시도해주세요.</p>}
      {preview && <><ImportSummary preview={preview} /><div className="import-actions"><button className="primary-button" disabled={busy} onClick={apply}>{uncertain ? '같은 요청으로 다시 시도' : preview.changed ? preview.existing ? '갱신 적용' : '자료 추가' : '확인하고 돌아가기'}</button><button className="secondary-button" disabled={busy || uncertain} onClick={clearPreview}>미리보기 취소</button></div></>}
    </section>
  </>;
}
