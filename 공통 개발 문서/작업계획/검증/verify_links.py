"""프로젝트 설계 문서의 로컬 Markdown 링크만 검사한다."""
import json
import re
import subprocess
from pathlib import Path
from urllib.parse import unquote

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent


def main():
    # 이번 실행이 만드는 보고서로 향하는 링크도 검사할 수 있게 경로를 먼저 만든다.
    (HERE / '문서경로검사결과.json').touch(exist_ok=True)
    listed = subprocess.run(['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z'],
                            cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True)
    relative_files = set(listed.stdout.decode('utf-8').split('\0'))
    files = sorted(ROOT / name for name in relative_files if name.endswith('.md')
                   and (name in ('README.md', 'AGENTS.md') or name.split('/')[0] in ('Android', 'Window', '공통 개발 문서')))
    count = 0
    local_data_count = 0
    missing_local_data_count = 0
    errors = []
    for path in files:
        text = re.sub(r'```.*?```', '', path.read_text(encoding='utf-8'), flags=re.S)
        for match in re.finditer(r'\[[^\]]*\]\((<[^>]+>|[^\s]+?)\)', text):
            link = match.group(1).strip('<>')
            if link.startswith(('https:', 'http:', '#', 'mailto:')):
                continue
            target = unquote(link.split('#', 1)[0])
            count += 1
            resolved = (path.parent / target).resolve()
            if ROOT / 'data' in resolved.parents or ROOT / 'local-data' in resolved.parents:
                local_data_count += 1
                if not resolved.exists():
                    missing_local_data_count += 1
                continue
            if not resolved.exists():
                errors.append({'file': str(path.relative_to(ROOT)), 'target': target})
    report = {'scope': '루트 README·AGENTS와 Android·Window·공통 문서의 로컬 링크. Git 제외 자료 링크는 별도 집계',
              'document_count': len(files), 'local_link_count': count,
              'local_data_link_count': local_data_count,
              'missing_local_data_link_count': missing_local_data_count, 'errors': errors}
    (HERE / '문서경로검사결과.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(report, ensure_ascii=False))
    if errors:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
