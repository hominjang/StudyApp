"""구현 설계 문서의 실제 DDL을 임시 SQLite에서 검사한다. 앱 테스트가 아니다."""
import json
import re
import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
SCHEMA_FILE = ROOT / 'Window/구현설계/04_1_DB스키마.md'
SQL = re.search(r'```sql\n(.*?)```', SCHEMA_FILE.read_text(encoding='utf-8'), re.S).group(1)
if not (ROOT / 'data/initial/catalog.json').is_file():
    raise SystemExit('실제 자료는 Git에서 제외됩니다. 로컬 data/initial 자료를 준비한 뒤 다시 실행하세요.')
CATALOG = json.loads((ROOT / 'data/initial/catalog.json').read_text(encoding='utf-8'))
PACKAGES = []
for entry in CATALOG['datasets']:
    base = ROOT / 'data/initial'
    PACKAGES.append((json.loads((base / entry['learning_file']).read_text(encoding='utf-8')),
                     json.loads((base / entry['supplement_file']).read_text(encoding='utf-8'))))


def fixture():
    db = sqlite3.connect(':memory:', isolation_level=None)
    db.execute('PRAGMA foreign_keys=ON')
    db.executescript(SQL)
    db.execute("INSERT INTO learners VALUES ('local','2026-11-24T00:00:00Z')")
    for pos, (data, supplement) in enumerate(PACKAGES):
        ds = data['dataset_id']
        db.execute('INSERT INTO datasets VALUES (?,?,?,1,?,?)',
                   (ds, data['title'], pos, 'fixture-hash', json.dumps(supplement.get('dataset_metadata'))))
        for index, item in enumerate(data['items']):
            db.execute('INSERT INTO items VALUES (?,?,?,?,?,?,1)',
                       (ds, item['id'], index, item['question'], item['answer'], json.dumps(item.get('accepted_answers', []))))
        for item in supplement['items']:
            db.execute('INSERT INTO supplements VALUES (?,?,?)', (ds, item['id'], json.dumps({k:v for k,v in item.items() if k != 'id'})))
    return db


def exam(db, eid='exam-1'):
    data = PACKAGES[0][0]
    db.execute('''INSERT INTO exams VALUES
      (?, 'local','in_progress',0,?,?,2,'count','ordered','general',0,'exact-v1',?,?,NULL,NULL,NULL)''',
      (eid, json.dumps([data['dataset_id']]), json.dumps({data['dataset_id']:data['title']}), 'start', 'save'))
    for position, item in enumerate(data['items'][:2]):
        db.execute("INSERT INTO exam_responses VALUES (?,?,?,?,?,?,?,?,'ungraded',NULL)",
                   (eid, position, data['dataset_id'], item['id'], item['question'], item['answer'], json.dumps(item.get('accepted_answers', [])), ''))


def completed(db, eid='exam-1', order=1):
    db.execute("UPDATE exam_responses SET judgement='correct' WHERE exam_id=?", (eid,))
    db.execute("UPDATE exams SET status='completed',submitted_at='submit',completed_at='complete',completion_order=? WHERE exam_id=?", (order, eid))


def rejects(db, sql, params=()):
    try:
        db.execute(sql, params)
    except sqlite3.IntegrityError:
        return
    raise AssertionError('제약이 거부해야 하는 변경을 허용함')


def main():
    results = []
    def run(name, fn):
        db = fixture()
        try:
            fn(db)
            assert db.execute('PRAGMA foreign_key_check').fetchall() == []
            assert db.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
            results.append({'scenario': name, 'status': 'pass'})
        finally:
            db.close()

    def actual_data(db):
        assert db.execute('SELECT COUNT(*) FROM items').fetchone()[0] == 254
        assert db.execute('SELECT COUNT(*) FROM supplements').fetchone()[0] == sum(len(s['items']) for _,s in PACKAGES)
        for data, _ in PACKAGES:
            rows = db.execute('SELECT item_id,answer,accepted_answers_json FROM items WHERE dataset_id=? ORDER BY position', (data['dataset_id'],)).fetchall()
            assert rows == [(i['id'], i['answer'], json.dumps(i.get('accepted_answers', []))) for i in data['items']]
        assert db.execute("SELECT COUNT(*) FROM sqlite_master WHERE type='table'").fetchone()[0] == 8
    run('문서 DDL의 8개 테이블과 실제 254문항·보충·ID·정답·허용 표기 등록', actual_data)

    def duplicate_response(db):
        exam(db)
        row = db.execute('SELECT * FROM exam_responses LIMIT 1').fetchone()
        rejects(db, 'INSERT INTO exam_responses VALUES (?,?,?,?,?,?,?,?,?,?)', row)
        changed = list(row); changed[1] = 2
        rejects(db, 'INSERT INTO exam_responses VALUES (?,?,?,?,?,?,?,?,?,?)', changed)
    run('동일 시험의 중복 위치와 중복 문항 거부', duplicate_response)

    def orphan(db):
        exam(db)
        row = list(db.execute('SELECT * FROM exam_responses LIMIT 1').fetchone()); row[0] = 'missing'
        rejects(db, 'INSERT INTO exam_responses VALUES (?,?,?,?,?,?,?,?,?,?)', row)
        rejects(db, "DELETE FROM learners WHERE learner_id='local'")
        rejects(db, "DELETE FROM exams WHERE exam_id='exam-1'")
    run('응답의 부모 시험과 프로필 외래 키 및 강한 참조 보존', orphan)

    def invalid_state(db):
        exam(db)
        for sql in ("UPDATE exams SET status='invalid'", "UPDATE exams SET status='submitted'",
                    "UPDATE exams SET status='completed',submitted_at='s',completed_at='c'",
                    "UPDATE exams SET revision=-1", "UPDATE exams SET current_position=2",
                    "UPDATE exams SET question_count=0", "UPDATE exams SET review_mode='other'"):
            rejects(db, sql)
    run('상태·시각·완료 순번·위치·버전·출제 설정 제약', invalid_state)

    def reasons(db):
        exam(db)
        rejects(db, "UPDATE exam_responses SET judgement='incorrect'")
        rejects(db, "UPDATE exam_responses SET incorrect_reason='unanswered'")
        db.execute("UPDATE exam_responses SET judgement='incorrect',incorrect_reason='unanswered'")
    run('미채점·정답·오답 사유의 조합 제약', reasons)

    def unique_order(db):
        exam(db); completed(db); exam(db, 'exam-2')
        rejects(db, "UPDATE exams SET status='completed',submitted_at='s',completed_at='c',completion_order=1 WHERE exam_id='exam-2'")
    run('사용자별 완료 순번 유일성', unique_order)

    def progress(db):
        exam(db); completed(db)
        ds, item = PACKAGES[0][0]['dataset_id'], PACKAGES[0][0]['items'][0]['id']
        db.execute("INSERT INTO item_progress VALUES ('local',?,?,1,0,'incorrect','exam-1')", (ds,item))
        rejects(db, 'UPDATE item_progress SET correct_streak=3')
        rejects(db, 'UPDATE item_progress SET is_review_target=0,correct_streak=2')
        db.execute('UPDATE item_progress SET is_review_target=0,correct_streak=3')
    run('오답 대상과 연속 정답 수의 범위·조합 제약', progress)

    def reorder(db):
        ds = PACKAGES[0][0]['dataset_id']
        rejects(db, 'UPDATE items SET position=1 WHERE dataset_id=? AND position=0', (ds,))
        db.execute('BEGIN IMMEDIATE')
        db.execute('UPDATE items SET is_active=0 WHERE dataset_id=?', (ds,))
        for pos, item in enumerate(reversed(PACKAGES[0][0]['items'])):
            db.execute('UPDATE items SET position=?,is_active=1 WHERE dataset_id=? AND item_id=?', (pos,ds,item['id']))
        db.execute('COMMIT')
        assert db.execute('SELECT item_id FROM items WHERE dataset_id=? AND position=0', (ds,)).fetchone()[0] == PACKAGES[0][0]['items'][-1]['id']
    run('활성 문항 순서 중복 거부와 전체 갱신 시 순서 변경', reorder)

    def preserve_snapshots(db):
        exam(db); completed(db)
        ds, item = PACKAGES[0][0]['dataset_id'], PACKAGES[0][0]['items'][0]['id']
        db.execute("INSERT INTO item_progress VALUES ('local',?,?,0,0,'correct','exam-1')", (ds,item))
        before = db.execute('SELECT * FROM exam_responses').fetchall()
        db.execute('DELETE FROM supplements'); db.execute('DELETE FROM items'); db.execute('DELETE FROM datasets')
        assert db.execute('SELECT * FROM exam_responses').fetchall() == before
        assert db.execute('SELECT COUNT(*) FROM item_progress').fetchone()[0] == 1
    run('원본 제거와 독립된 시험 스냅샷·학습 상태 참조', preserve_snapshots)

    def rollback(db):
        exam(db)
        db.execute("UPDATE exams SET status='submitted',submitted_at='s'")
        db.execute('BEGIN IMMEDIATE')
        completed(db)
        db.execute('ROLLBACK')
        assert db.execute('SELECT status,completion_order FROM exams').fetchone() == ('submitted',None)
        assert {r[0] for r in db.execute('SELECT judgement FROM exam_responses')} == {'ungraded'}
    run('완료 저장 중 롤백의 판정·상태·순번 동시 복원', rollback)

    def raw_strings(db):
        exam(db)
        text = '  A\t B e\u0301  '
        db.execute('UPDATE exam_responses SET response_text=?', (text,))
        assert {r[0] for r in db.execute('SELECT response_text FROM exam_responses')} == {text}
    run('입력 원문의 공백·탭·Unicode 조합 보존', raw_strings)

    report = {'scope':'구현 설계 문서 DDL과 실제 자료를 사용한 임시 SQLite 검사. C# 앱 검증 아님',
              'schema_source':str(SCHEMA_FILE.relative_to(ROOT)), 'sqlite_version':sqlite3.sqlite_version,
              'table_count':8,'question_count':254,'passed_scenario_count':len(results),'scenarios':results,
              'not_tested':['C# 저장소 구현','WPF 화면','Microsoft.Data.Sqlite 연결','앱 배포 환경']}
    (HERE / '구현스키마검사결과.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print(json.dumps({k:v for k,v in report.items() if k != 'scenarios'},ensure_ascii=False))


if __name__ == '__main__':
    main()
