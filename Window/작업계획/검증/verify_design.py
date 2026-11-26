"""설계 검증 모델. 실제 앱/저장 계층 구현이 아니며 임시 SQLite만 사용한다."""
import copy
import json
import random
import re
import sqlite3
import tempfile
import uuid
from contextlib import contextmanager
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
if not (ROOT / 'data/initial/catalog.json').is_file():
    raise SystemExit('실제 자료는 Git에서 제외됩니다. 로컬 data/initial 자료를 준비한 뒤 다시 실행하세요.')
CATALOG = json.loads((ROOT / 'data/initial/catalog.json').read_text(encoding='utf-8'))
DATA = [json.loads((ROOT / 'data/initial' / d['learning_file']).read_text(encoding='utf-8'))
        for d in CATALOG['datasets']]
KEYS = [(d['dataset_id'], i['id']) for d in DATA for i in d['items']]
RESULTS = []
CASES = []


def check(name):
    def run(fn):
        CASES.append((name, fn))
        return fn
    return run


# 공통 자동 채점 기준의 미응답 공백 문자: Unicode White_Space 25자
WHITE_SPACE = frozenset(
    [chr(c) for c in range(0x09, 0x0E)] + [' ', '\u0085', ' ', ' ']
    + [chr(c) for c in range(0x2000, 0x200B)] + [' ', ' ', ' ', ' ', '　'])
assert len(WHITE_SPACE) == 25
# 공통 식별 기준의 자료·문항 ID 형식
ID_PATTERN = re.compile(r'[a-z0-9]+(?:-[a-z0-9]+)*')


def is_valid_id(value):
    return 1 <= len(value) <= 64 and ID_PATTERN.fullmatch(value) is not None


def grade(text, answer, aliases):
    if all(c in WHITE_SPACE for c in text):
        return 'incorrect', 'unanswered'
    return ('correct', None) if text in [answer, *aliases] else ('incorrect', 'no_match')


class Model:
    def __init__(self, seed=True):
        self.db = sqlite3.connect(':memory:', isolation_level=None)
        self.db.row_factory = sqlite3.Row
        self.db.execute('PRAGMA foreign_keys=ON')
        self.db.executescript('''
          CREATE TABLE meta(k TEXT PRIMARY KEY, v TEXT);
          CREATE TABLE learners(id TEXT PRIMARY KEY);
          CREATE TABLE datasets(id TEXT PRIMARY KEY, title TEXT NOT NULL);
          CREATE TABLE items(dataset TEXT, id TEXT, question TEXT, answer TEXT,
            aliases TEXT, active INTEGER NOT NULL, PRIMARY KEY(dataset,id),
            FOREIGN KEY(dataset) REFERENCES datasets(id));
          CREATE TABLE exams(id INTEGER PRIMARY KEY, learner TEXT REFERENCES learners(id),
            state TEXT CHECK(state IN ('in_progress','submitted','completed')),
            revision INTEGER NOT NULL, position INTEGER NOT NULL, titles TEXT,
            completion_order INTEGER, completed_at TEXT,
            UNIQUE(learner,completion_order));
          CREATE TABLE responses(exam INTEGER REFERENCES exams(id), position INTEGER,
            dataset TEXT, item TEXT, question TEXT, answer TEXT, aliases TEXT,
            response TEXT NOT NULL, judgement TEXT NOT NULL, reason TEXT,
            PRIMARY KEY(exam,position), UNIQUE(exam,dataset,item));
          CREATE TABLE progress(learner TEXT REFERENCES learners(id), dataset TEXT, item TEXT,
            target INTEGER, streak INTEGER, last_exam INTEGER REFERENCES exams(id),
            PRIMARY KEY(learner,dataset,item));
        ''')
        if seed:
            self.install()

    @contextmanager
    def tx(self):
        self.db.execute('BEGIN IMMEDIATE')
        try:
            yield
            self.db.execute('COMMIT')
        except Exception:
            self.db.execute('ROLLBACK')
            raise

    def install(self, fail=False):
        with self.tx():
            if self.db.execute("SELECT 1 FROM meta WHERE k='seed'").fetchone():
                return
            self.db.execute("INSERT INTO learners VALUES ('local')")
            for data in DATA:
                self._import(data)
                if fail:
                    raise RuntimeError('injected install failure')
            self.db.execute("INSERT INTO meta VALUES ('seed','1')")

    def _import(self, data, fail=False):
        ds = data['dataset_id']
        self.db.execute('INSERT OR REPLACE INTO datasets VALUES (?,?)', (ds, data['title']))
        self.db.execute('UPDATE items SET active=0 WHERE dataset=?', (ds,))
        for i in data['items']:
            self.db.execute('''INSERT INTO items VALUES (?,?,?,?,?,1)
                ON CONFLICT(dataset,id) DO UPDATE SET question=excluded.question,
                answer=excluded.answer,aliases=excluded.aliases,active=1''',
                (ds, i['id'], i['question'], i['answer'], json.dumps(i.get('accepted_answers', []))))
            if fail:
                raise RuntimeError('injected import failure')

    def item(self, key):
        return self.db.execute('SELECT * FROM items WHERE dataset=? AND id=?', key).fetchone()

    def create(self, keys, learner='local'):
        with self.tx():
            titles = {k[0]: self.db.execute('SELECT title FROM datasets WHERE id=?', (k[0],)).fetchone()[0]
                      for k in keys}
            exam = self.db.execute('''INSERT INTO exams
                (learner,state,revision,position,titles) VALUES (?,'in_progress',0,0,?)''',
                (learner, json.dumps(titles))).lastrowid
            for pos, key in enumerate(keys):
                i = self.item(key)
                assert i and i['active']
                self.db.execute("INSERT INTO responses VALUES (?,?,?,?,?,?,?,?,'ungraded',NULL)",
                                (exam, pos, *key, i['question'], i['answer'], i['aliases'], ''))
            return exam

    def exam(self, exam):
        return dict(self.db.execute('SELECT * FROM exams WHERE id=?', (exam,)).fetchone())

    def responses(self, exam):
        return [dict(r) for r in self.db.execute('SELECT * FROM responses WHERE exam=? ORDER BY position', (exam,))]

    def _save(self, exam, revision, answers, position):
        e, rows = self.exam(exam), self.responses(exam)
        if e['state'] != 'in_progress':
            return False
        assert len(answers) == len(rows) and 0 <= position < len(rows)
        if revision < e['revision']:
            return False
        if revision == e['revision']:
            if [r['response'] for r in rows] != answers or position != e['position']:
                raise ValueError('same revision with different payload')
            return True
        for pos, text in enumerate(answers):
            self.db.execute('UPDATE responses SET response=? WHERE exam=? AND position=?', (text, exam, pos))
        self.db.execute('UPDATE exams SET revision=?,position=? WHERE id=?', (revision, position, exam))
        return True

    def save(self, *args):
        with self.tx():
            return self._save(*args)

    def submit(self, exam, revision, answers, position=0):
        with self.tx():
            if self.exam(exam)['state'] != 'in_progress':
                return
            if not self._save(exam, revision, answers, position):
                raise ValueError('stale submission')
            self.db.execute("UPDATE exams SET state='submitted' WHERE id=?", (exam,))

    def complete(self, exam, fail=False, timestamp='2026-11-24T00:00:00Z'):
        with self.tx():
            e = self.exam(exam)
            if e['state'] == 'completed':
                return
            assert e['state'] == 'submitted'
            rows = self.responses(exam)
            for r in rows:
                verdict, reason = grade(r['response'], r['answer'], json.loads(r['aliases']))
                self.db.execute('UPDATE responses SET judgement=?,reason=? WHERE exam=? AND position=?',
                                (verdict, reason, exam, r['position']))
            order = self.db.execute('SELECT COALESCE(MAX(completion_order),0)+1 FROM exams WHERE learner=?',
                                    (e['learner'],)).fetchone()[0]
            self.db.execute("UPDATE exams SET state='completed',completion_order=?,completed_at=? WHERE id=?",
                            (order, timestamp, exam))
            for r in rows:
                history = self.db.execute('''SELECT e.id,r.judgement FROM exams e JOIN responses r ON r.exam=e.id
                    WHERE e.learner=? AND e.state='completed' AND r.dataset=? AND r.item=?
                    ORDER BY e.completion_order''', (e['learner'], r['dataset'], r['item'])).fetchall()
                target, streak = 0, 0
                for h in history:
                    if h['judgement'] == 'incorrect':
                        target, streak = 1, 0
                    elif target:
                        streak += 1
                        if streak == 3:
                            target = 0
                self.db.execute('INSERT OR REPLACE INTO progress VALUES (?,?,?,?,?,?)',
                                (e['learner'], r['dataset'], r['item'], target, streak, exam))
            if fail:
                raise RuntimeError('injected before commit')

    def state(self, key=KEYS[0], learner='local'):
        r = self.db.execute('SELECT * FROM progress WHERE learner=? AND dataset=? AND item=?', (learner, *key)).fetchone()
        return (r['target'], r['streak']) if r else (0, 0)

    def attempt(self, correct, key=KEYS[0], learner='local', **kw):
        exam = self.create([key], learner)
        self.submit(exam, 1, [self.item(key)['answer'] if correct else 'wrong-unregistered'])
        self.complete(exam, **kw)
        return exam


def rejected(fn, error):
    try:
        fn()
    except error:
        return
    raise AssertionError('expected rejection')


@check('초기 254문항·ID 보존·초기화 재실행')
def initial_seed():
    m = Model()
    assert m.db.execute('SELECT COUNT(*) FROM items').fetchone()[0] == 254
    assert set(KEYS) == set(map(tuple, m.db.execute('SELECT dataset,id FROM items')))
    assert all(is_valid_id(part) for key in KEYS for part in key)
    for bad in ('', 'Upper', 'a_b', 'a--b', '-a', 'a-', 'a b', 'x' * 65):
        assert not is_valid_id(bad)
    new_id = str(uuid.uuid4())
    assert new_id == new_id.lower() and is_valid_id(new_id)
    m.db.execute('UPDATE items SET question=? WHERE dataset=? AND id=?', ('사용자 수정', *KEYS[0]))
    m.install()
    assert m.item(KEYS[0])['question'] == '사용자 수정'


@check('최초 자료 설치 실패의 전체 롤백')
def failed_seed():
    m = Model(False)
    rejected(lambda: m.install(fail=True), RuntimeError)
    for table in ('meta', 'learners', 'datasets', 'items'):
        assert m.db.execute(f'SELECT COUNT(*) FROM {table}').fetchone()[0] == 0
    m.install()
    assert len(m.db.execute('SELECT * FROM items').fetchall()) == 254


@check('실제 자료의 채점 예시 1705건')
def actual_grading():
    total = 0
    for entry, data in zip(CATALOG['datasets'], DATA):
        examples = json.loads((ROOT / 'data/initial' / Path(entry['learning_file']).parent / 'grading_examples.json').read_text(encoding='utf-8'))
        items = {i['id']: i for i in data['items']}
        for case in examples['cases']:
            i = items[case['item_id']]
            assert (grade(case['input'], i['answer'], i.get('accepted_answers', []))[0] == 'correct') == case['expected_correct']
            total += 1
    assert total == 1705


@check('공백 미응답과 문자열 완전 일치·유니코드 비정규화')
def exact_strings():
    assert grade(' \t\n', 'ABC', []) == ('incorrect', 'unanswered')
    for text in ('', '　', ' ', '\u0085', ' ', ''.join(sorted(WHITE_SPACE))):
        assert grade(text, 'ABC', []) == ('incorrect', 'unanswered')
    for text in ('\x1c', '\x1f', '​', '﻿', ' ​ '):
        assert grade(text, 'ABC', []) == ('incorrect', 'no_match')
    for text in ('abc', ' ABC', 'ABC ', 'ABC.', 'AB C'):
        assert grade(text, 'ABC', []) == ('incorrect', 'no_match')
    assert grade('Alias', 'ABC', ['Alias'])[0] == 'correct'
    assert grade('é', 'e\u0301', [])[0] == 'incorrect'


@check('늦은 자동 저장 무시·동일 버전 재시도·충돌 검출')
def autosave():
    m = Model(); e = m.create(KEYS[:2])
    assert m.save(e, 2, ['최신', '  raw  '], 1)
    assert not m.save(e, 1, ['과거', ''], 0)
    assert m.save(e, 2, ['최신', '  raw  '], 1)
    rejected(lambda: m.save(e, 2, ['충돌', ''], 0), ValueError)
    assert [r['response'] for r in m.responses(e)] == ['최신', '  raw  ']
    assert m.exam(e)['position'] == 1


@check('제출 최신 답 확정·입력 잠금·완료 전 미채점')
def submission():
    m = Model(); e = m.create(KEYS[:2])
    m.save(e, 1, ['old', ''], 0)
    m.submit(e, 2, ['final', ' \t'], 1)
    assert m.exam(e)['state'] == 'submitted'
    assert not m.save(e, 3, ['late', 'late'], 0)
    assert all(r['judgement'] == 'ungraded' for r in m.responses(e))
    assert m.state() == (0, 0)
    m.complete(e)
    assert m.responses(e)[1]['reason'] == 'unanswered'
    assert m.responses(e)[1]['response'] == ' \t'


@check('완료 직전 장애의 전체 롤백·같은 시험 재시도')
def atomic_completion():
    m = Model(); e = m.create(KEYS[:2]); m.submit(e, 1, ['wrong', ''])
    rejected(lambda: m.complete(e, fail=True), RuntimeError)
    assert m.exam(e)['state'] == 'submitted' and m.exam(e)['completion_order'] is None
    assert all(r['judgement'] == 'ungraded' for r in m.responses(e))
    assert m.db.execute('SELECT COUNT(*) FROM progress').fetchone()[0] == 0
    m.complete(e)
    assert m.exam(e)['state'] == 'completed' and m.exam(e)['completion_order'] == 1
    assert m.state() == (1, 0)


@check('완료 반복 요청과 재조회가 집계를 늘리지 않음')
def idempotency():
    m = Model(); m.attempt(False); e = m.attempt(True)
    before = m.exam(e)
    for _ in range(3):
        m.complete(e); m.responses(e)
    assert m.exam(e) == before and m.state() == (1, 1)


@check('오답 후 3회 연속 정답·재오답 편입·중간 오답 초기화')
def review_streak():
    m = Model()
    for correct, expected in [(False,(1,0)),(True,(1,1)),(True,(1,2)),(False,(1,0)),
                              (True,(1,1)),(True,(1,2)),(True,(0,3)),(False,(1,0))]:
        m.attempt(correct)
        assert m.state() == expected


@check('다른 문항 시험과 첫 정답은 오답 연속 횟수에 영향 없음')
def unrelated_attempts():
    m = Model(); m.attempt(True)
    assert m.state() == (0, 0)
    m.attempt(False); m.attempt(True); m.attempt(True, KEYS[1])
    assert m.state() == (1, 1)


@check('학습 상태는 사용자별로 분리')
def learner_isolation():
    m = Model(); m.db.execute("INSERT INTO learners VALUES ('second')")
    m.attempt(False)
    m.attempt(True, learner='second')
    assert m.state() == (1, 0) and m.state(learner='second') == (0, 0)


@check('완료 순번은 시계 역행과 시험 시작 순서에 독립')
def completion_order():
    m = Model(); first = m.create([KEYS[0]]); second = m.create([KEYS[0]])
    m.submit(first, 1, [m.item(KEYS[0])['answer']]); m.submit(second, 1, ['wrong'])
    m.complete(second, timestamp='2030-01-01T00:00:00Z')
    m.complete(first, timestamp='2020-01-01T00:00:00Z')
    assert m.exam(second)['completion_order'] == 1 and m.exam(first)['completion_order'] == 2
    assert m.state() == (1, 1)


@check('원문 변경·삭제 후 질문·답·순서·입력 위치 복원과 과거 채점 유지')
def snapshot_preservation():
    m = Model(); e = m.create(KEYS[:2]); before = m.responses(e)
    m.save(e, 1, [before[0]['answer'], before[1]['answer']], 1)
    m.db.execute('UPDATE items SET question=?,answer=?,aliases=? WHERE dataset=? AND id=?',
                 ('changed', 'changed', '[]', *KEYS[0]))
    m.db.execute('DELETE FROM items')
    assert m.exam(e)['position'] == 1
    assert [r['item'] for r in m.responses(e)] == [r['item'] for r in before]
    assert m.responses(e)[0]['question'] == before[0]['question']
    m.submit(e, 1, [before[0]['answer'], before[1]['answer']], 1); m.complete(e)
    assert all(r['judgement'] == 'correct' for r in m.responses(e))
    assert m.exam(e)['titles']


@check('자료 갱신 중 실패 롤백·빠진 문항 비활성·재가져오기 재연결')
def import_update():
    m = Model(); key = KEYS[0]; m.attempt(False)
    changed = copy.deepcopy(DATA[0]); changed['items'] = changed['items'][1:]
    rejected(lambda: import_transaction(m, changed, True), RuntimeError)
    assert m.item(key)['active'] == 1
    import_transaction(m, changed)
    assert m.item(key)['active'] == 0 and m.state(key) == (1, 0)
    rejected(lambda: m.create([key]), AssertionError)
    import_transaction(m, DATA[0])
    assert m.item(key)['active'] == 1 and m.state(key) == (1, 0)


def import_transaction(m, data, fail=False):
    with m.tx():
        m._import(data, fail)


@check('응답의 부모 시험·중복 위치·중복 문항 제약')
def integrity_constraints():
    m = Model(); e = m.create(KEYS[:2]); r = m.responses(e)[0]
    values = tuple(r.values())
    rejected(lambda: m.db.execute('INSERT INTO responses VALUES (?,?,?,?,?,?,?,?,?,?)', values), sqlite3.IntegrityError)
    changed = list(values); changed[0] = 999999
    rejected(lambda: m.db.execute('INSERT INTO responses VALUES (?,?,?,?,?,?,?,?,?,?)', changed), sqlite3.IntegrityError)
    changed[0] = e; changed[1] = 99
    rejected(lambda: m.db.execute('INSERT INTO responses VALUES (?,?,?,?,?,?,?,?,?,?)', changed), sqlite3.IntegrityError)


def select(pool, targets, mode, count=None, ordered=True):
    rng = random.Random(42)
    def take(group, size):
        return group[:size] if ordered else rng.sample(group, size)
    candidates = [k for k in pool if k in targets] if mode == 'wrong_only' else list(pool)
    n = len(candidates) if count is None else count
    if n < 1 or n > len(candidates):
        raise ValueError('invalid count or empty candidate')
    if mode == 'wrong_first':
        wrong = [k for k in candidates if k in targets]
        rest = [k for k in candidates if k not in targets]
        chosen = take(wrong, min(n, len(wrong)))
        chosen += take(rest, n - len(chosen))
    else:
        chosen = take(candidates, n)
    if ordered:
        return sorted(chosen, key=pool.index)
    rng.shuffle(chosen)
    return chosen


@check('출제 모드·선택 범위·전체·중복 방지·후보 부족')
def selection_rules():
    pool = list(KEYS); targets = {pool[2], pool[9]}
    assert len(select(pool, targets, 'general')) == 254
    assert select(pool, targets, 'wrong_only') == [pool[2], pool[9]]
    chosen = select(pool, targets, 'wrong_first', 3)
    assert chosen == [pool[0], pool[2], pool[9]]
    assert select(pool, set(), 'wrong_first', 3) == pool[:3]
    for mode in ('general', 'wrong_first', 'wrong_only'):
        chosen = select(pool, targets, mode, ordered=False)
        assert len(chosen) == len(set(chosen))
    rejected(lambda: select(pool, set(), 'wrong_only'), ValueError)
    rejected(lambda: select(pool, targets, 'wrong_only', 3), ValueError)
    rejected(lambda: select(pool, targets, 'general', 0), ValueError)
    subset = KEYS[107:]
    assert set(select(subset, targets, 'wrong_first')).issubset(subset)


@check('SQLite Backup API의 임시 백업·재연결 복원')
def backup_roundtrip():
    m = Model(); e = m.attempt(False)
    with tempfile.TemporaryDirectory(prefix='study-design-') as path:
        backup = sqlite3.connect(str(Path(path) / 'backup.db'))
        m.db.backup(backup)
        assert backup.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
        backup.close()
        restored = sqlite3.connect(str(Path(path) / 'backup.db'))
        assert restored.execute('SELECT COUNT(*) FROM items').fetchone()[0] == 254
        assert restored.execute('SELECT state,completion_order FROM exams WHERE id=?', (e,)).fetchone() == ('completed', 1)
        assert restored.execute('SELECT target,streak FROM progress').fetchone() == (1, 0)
        restored.close()


def main():
    for name, fn in CASES:
        fn()
        RESULTS.append({'scenario': name, 'status': 'pass'})
    report = {'scope': '문서의 규칙을 구현한 독립 설계 모델. 실제 앱 테스트가 아님',
          'sqlite_version': sqlite3.sqlite_version,
          'dataset_count': len(DATA), 'question_count': len(KEYS), 'grading_case_count': 1705,
          'passed_scenario_count': len(RESULTS), 'scenarios': RESULTS,
          'not_tested': ['실제 UI', '프레임워크·SQLite 연결 라이브러리', '파일 잠금과 다중 프로세스',
                         '실제 디스크 장애·강제 전원 종료', 'JSON 가져오기·내보내기 UI', '앱 DB 마이그레이션 구현']}
    (HERE / '설계시나리오결과.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    print(json.dumps({k: v for k, v in report.items() if k != 'scenarios'}, ensure_ascii=False))


if __name__ == '__main__':
    main()
