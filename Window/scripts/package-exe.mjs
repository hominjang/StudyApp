import { constants } from 'node:fs';
import { readFile, copyFile, mkdir, realpath, stat, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { dirname, isAbsolute, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const windowRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const source = resolve(windowRoot, '../data/initial');
const checkOnly = process.argv.slice(2);
if (checkOnly.some((arg) => arg !== '--check')) throw new Error('지원하지 않는 실행 옵션입니다.');
if (process.platform !== 'win32' || process.arch !== 'x64') {
  throw new Error('현재 실행 파일 패키징은 Windows x64 환경을 대상으로 합니다.');
}

async function readJson(path, message) {
  try { return JSON.parse(await readFile(path, 'utf8')); }
  catch { throw new Error(message); }
}

const catalog = await readJson(resolve(source, 'catalog.json'), '로컬 data/initial/catalog.json을 준비해주세요. 기존 자산 사본으로 패키징하지 않습니다.');
if (!['1.0', '2.0'].includes(catalog.schema_version) || !Array.isArray(catalog.datasets) || !catalog.datasets.length) {
  throw new Error('초기 자료 목록 형식을 확인해주세요.');
}
const sourceRoot = await realpath(source);
const names = new Set(['catalog.json']);
for (const entry of catalog.datasets) {
  if (!entry || typeof entry !== 'object') throw new Error('초기 자료 목록 형식을 확인해주세요.');
  names.add(entry.learning_file);
  if (entry.supplement_file !== undefined && entry.supplement_file !== null) names.add(entry.supplement_file);
}
const files = [];
for (const name of names) {
  if (typeof name !== 'string' || isAbsolute(name) || name.split(/[\\/]/).some((part) => !part || part === '..' || part === '.')) {
    throw new Error('초기 자료 파일 경로를 확인해주세요.');
  }
  let path;
  try { path = await realpath(resolve(sourceRoot, name)); }
  catch { throw new Error('목록에 지정된 초기 자료 파일이 없습니다.'); }
  const within = relative(sourceRoot, path);
  if (within.startsWith('..') || isAbsolute(within) || !(await stat(path)).isFile()) {
    throw new Error('초기 자료 경로가 보관 폴더 안의 파일이어야 합니다.');
  }
  files.push({ from: path, name: `initial-data/${name.replaceAll('\\', '/')}` });
}
if (checkOnly.length) {
  console.log(`실행 파일 준비 검사 통과: Windows x64, 초기 자료 파일 ${files.length}개.`);
} else {
  const executable = resolve(windowRoot, 'src-tauri/target/release/studyapp-window.exe');
  const bytes = await readFile(executable);
  // 다른 타깃 또는 디버그 실행 파일을 잘못 전달하지 않도록 PE 아키텍처를 확인한다.
  const pe = bytes.length >= 64 ? bytes.readUInt32LE(0x3c) : -1;
  if (bytes.readUInt16LE(0) !== 0x5a4d || pe < 0 || pe + 6 > bytes.length || bytes.readUInt32LE(pe) !== 0x4550 || bytes.readUInt16LE(pe + 4) !== 0x8664) {
    throw new Error('Windows x64 PE 실행 파일을 확인하지 못했습니다.');
  }
  const { version } = await readJson(resolve(windowRoot, 'package.json'), '앱 버전을 읽지 못했습니다.');
  if (typeof version !== 'string' || !/^\d+\.\d+\.\d+$/.test(version)) throw new Error('앱 버전을 확인해주세요.');
  const stamp = new Date().toISOString().replaceAll(/[-:.]/g, '');
  const artifacts = resolve(windowRoot, 'artifacts');
  await mkdir(artifacts, { recursive: true });
  // 매번 새 폴더를 만들어 이전 실행 파일과 사용자 파일을 덮어쓰지 않는다.
  const output = resolve(artifacts, `StudyApp-${version}-windows-x64-${stamp}`);
  await mkdir(output);
  const manifest = { version, target: 'windows-x64', created_at: new Date().toISOString(), files: [] };
  for (const file of [{ from: executable, name: 'StudyApp.exe' }, ...files]) {
    const to = resolve(output, file.name);
    await mkdir(dirname(to), { recursive: true });
    await copyFile(file.from, to, constants.COPYFILE_EXCL);
    manifest.files.push({ path: file.name, sha256: createHash('sha256').update(await readFile(to)).digest('hex') });
  }
  await writeFile(resolve(output, 'manifest.json'), JSON.stringify(manifest, null, 2) + '\n', { flag: 'wx' });
  await writeFile(resolve(output, '실행안내.txt'), [
    'StudyApp.exe를 더블클릭하면 실행합니다. 개발 서버나 Node/Rust 설치는 필요하지 않습니다.',
    'initial-data 폴더와 함께 이 폴더 전체를 보관하고 이동해주세요.',
    '학습 기록은 %LOCALAPPDATA%\\StudyApp\\study.db에 저장됩니다.',
    '개발 실행의 .tools/runtime 기록과는 별도이며 자동으로 이전하지 않습니다.',
    '앱 왼쪽 자료 학습에서 질문과 답을 시험 없이 함께 읽습니다. 자료 선택·질문 검색·페이지 이동을 지원하며 시험/오답 기록은 바뀌지 않습니다.',
    '앱 왼쪽 자료 추가에서 학습 JSON과 선택적인 보충 JSON을 고른 뒤 내용 확인 → 자료 추가/갱신 적용을 선택합니다.',
    '기존 저장소는 1.0 단답형 자료만, 신규 확장 저장소는 1.0·2.0 자료를 지원합니다. DB를 지우거나 바꿔서 전환하지 마세요.',
    '같은 자료 ID를 가져오면 전체 문항 목록을 갱신합니다. 빠진 문항은 비활성화하고 시험 기록은 보존합니다.',
    'Microsoft Edge WebView2 Runtime이 설치된 Windows x64 환경이 필요합니다.',
    '초기 자료와 실행 파일은 개인 자료를 포함하므로 Git에 추가하지 않습니다.',
    '',
  ].join('\r\n'), { flag: 'wx' });
  console.log(`Windows 실행 폴더 생성 완료: ${output}`);
  console.log(`실행: ${resolve(output, 'StudyApp.exe')}`);
  console.log(`실행 파일과 초기 자료 ${files.length}개를 포함했습니다. 기존 학습 기록은 복사하지 않았습니다.`);
}
