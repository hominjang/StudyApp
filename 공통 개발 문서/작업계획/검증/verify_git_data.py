"""실제 데이터와 복사·빌드 산출물의 Git 추적 제외를 읽기 전용으로 검사한다."""
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent


def git(*args, input_bytes=None):
    return subprocess.run(['git', *args], cwd=ROOT, input=input_bytes, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, check=False)


def main():
    data_files = [p.relative_to(ROOT).as_posix() for folder in ('data', 'local-data')
                  for p in (ROOT / folder).rglob('*') if p.is_file()]
    probes = ['data/initial/catalog.json', 'local-data/import.json',
              'Window/src/StudyApp.Desktop/Assets/InitialData/learning.json',
              'Android/app/src/main/assets/initial-data/learning.json',
              'Window/artifacts/publish/win-x64/package.zip', 'local-data/study.db']
    checked = sorted(set(data_files + probes))
    result = git('check-ignore', '--no-index', '-z', '--stdin', input_bytes=('\0'.join(checked)+'\0').encode('utf-8'))
    if result.returncode not in (0, 1):
        raise SystemExit(result.stderr.decode('utf-8', errors='replace'))
    ignored = set(result.stdout.decode('utf-8').strip('\0').split('\0'))
    missing = [name for name in checked if name not in ignored]
    tracked = git('ls-files', '--cached', '-z')
    if tracked.returncode:
        raise SystemExit(tracked.stderr.decode('utf-8', errors='replace'))
    def prohibited(name):
        parts = Path(name).parts
        return (name.startswith(('data/', 'local-data/'))
                or any(p in ('InitialData', 'initial-data', 'artifacts', 'backups', 'bin', 'obj', 'build') for p in parts)
                or any(name.endswith(s) or s+'-' in name for s in ('.db', '.sqlite', '.sqlite3')))
    tracked_data = [name for name in tracked.stdout.decode('utf-8').split('\0') if name and prohibited(name)]
    report = {'scope': '로컬 실제 자료의 ignore와 현재 index의 데이터·복사본·산출물 추적 여부',
              'checked_local_data_files': len(data_files),
              'ignored_local_data_files': sum(name in ignored for name in data_files),
              'ignored_path_probes': len(probes), 'tracked_data_files': len(tracked_data),
              'errors': {'not_ignored': missing, 'tracked_prohibited_paths': tracked_data}}
    (HERE / 'Git데이터제외검사결과.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print(json.dumps(report,ensure_ascii=False))
    if missing or tracked_data:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
