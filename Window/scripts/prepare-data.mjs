import { readFile, copyFile, mkdir, realpath } from 'node:fs/promises';
import { dirname, isAbsolute, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const windowRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const source = resolve(windowRoot, '../data/initial');
const destination = resolve(windowRoot, 'src-tauri/resources/initial-data');

let catalog;
try {
  catalog = JSON.parse(await readFile(resolve(source, 'catalog.json'), 'utf8'));
} catch (error) {
  if (error?.code === 'ENOENT') {
    console.log('로컬 초기 자료가 없습니다. 이미 등록된 DB는 그대로 사용하며, 최초 등록에는 자료 준비가 필요합니다.');
    process.exit(0);
  }
  throw new Error('초기 자료 목록을 읽지 못했습니다.');
}
if (!['1.0', '2.0'].includes(catalog.schema_version) || !Array.isArray(catalog.datasets) || !catalog.datasets.length) {
  throw new Error('초기 자료 목록 형식을 확인해주세요.');
}
const sourceRoot = await realpath(source);
const files = new Set(['catalog.json']);
for (const entry of catalog.datasets) {
  files.add(entry.learning_file);
  if (entry.supplement_file !== undefined && entry.supplement_file !== null) files.add(entry.supplement_file);
}
const copies = [];
for (const file of files) {
  if (typeof file !== 'string' || isAbsolute(file) || file.split(/[\\/]/).some((part) => !part || part === '..' || part === '.')) {
    throw new Error('초기 자료 파일 경로를 확인해주세요.');
  }
  const resolved = await realpath(resolve(sourceRoot, file));
  const within = relative(sourceRoot, resolved);
  if (within.startsWith('..') || isAbsolute(within)) throw new Error('초기 자료 경로가 보관 폴더 밖을 가리킵니다.');
  copies.push([resolved, resolve(destination, file)]);
}
// 검증이 모두 끝난 뒤 로컬 사본만 갱신한다. 원본과 문항 내용은 출력하지 않는다.
for (const [from, to] of copies) {
  await mkdir(dirname(to), { recursive: true });
  await copyFile(from, to);
}
console.log(`초기 자료 파일 ${copies.length}개를 Git 제외된 앱 자산에 준비했습니다.`);
