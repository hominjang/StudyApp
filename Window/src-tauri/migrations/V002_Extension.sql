-- Executed only on a disposable v1 copy, inside a transaction with FK checks deferred to validation.
DROP TABLE item_progress;
DROP TABLE exam_responses;
DROP TABLE items;

CREATE TABLE items (
    dataset_id TEXT NOT NULL REFERENCES datasets(dataset_id) ON DELETE RESTRICT,
    item_id TEXT NOT NULL,
    position INTEGER NOT NULL CHECK(position >= 0),
    type TEXT NOT NULL CHECK(type IN ('short_answer','descriptive')),
    question TEXT NOT NULL,
    criteria_json TEXT NOT NULL CHECK(json_valid(criteria_json) AND json_extract(criteria_json,'$.type') IS type),
    is_active INTEGER NOT NULL CHECK(is_active IN (0,1)),
    PRIMARY KEY(dataset_id,item_id)
);
CREATE UNIQUE INDEX ux_items_active_position ON items(dataset_id,position) WHERE is_active=1;
CREATE TABLE item_learning_links (
    dataset_id TEXT NOT NULL REFERENCES datasets(dataset_id) ON DELETE RESTRICT,
    item_id TEXT NOT NULL,
    position INTEGER NOT NULL CHECK(position >= 0),
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json)),
    PRIMARY KEY(dataset_id,position)
);
CREATE TABLE exam_responses (
    exam_id TEXT NOT NULL REFERENCES exams(exam_id) ON DELETE RESTRICT,
    position INTEGER NOT NULL CHECK(position >= 0),
    dataset_id TEXT NOT NULL,
    item_id TEXT NOT NULL,
    type TEXT NOT NULL CHECK(type IN ('short_answer','descriptive')),
    question_snapshot TEXT NOT NULL,
    criteria_snapshot_json TEXT NOT NULL CHECK(json_valid(criteria_snapshot_json) AND json_extract(criteria_snapshot_json,'$.type') IS type),
    response_text TEXT NOT NULL,
    PRIMARY KEY(exam_id,position), UNIQUE(exam_id,dataset_id,item_id)
);
CREATE INDEX ix_response_item_history ON exam_responses(dataset_id,item_id,exam_id);
ALTER TABLE exams ADD COLUMN result_revision INTEGER NOT NULL DEFAULT 0 CHECK(result_revision >= 0);

CREATE TABLE assessment_events (
    event_id TEXT NOT NULL UNIQUE,
    exam_id TEXT NOT NULL,
    position INTEGER NOT NULL,
    assessment_revision INTEGER NOT NULL CHECK(assessment_revision >= 1),
    previous_judgement TEXT CHECK(previous_judgement IN ('ungraded','correct','partial','incorrect')),
    judgement TEXT NOT NULL CHECK(judgement IN ('correct','partial','incorrect')),
    incorrect_reason TEXT,
    method TEXT NOT NULL CHECK(method IN ('automatic','user','migration')),
    recorded_at TEXT NOT NULL,
    note TEXT,
    request_id TEXT,
    PRIMARY KEY(exam_id,position,assessment_revision),
    UNIQUE(exam_id,position,assessment_revision,event_id),
    FOREIGN KEY(exam_id,position) REFERENCES exam_responses(exam_id,position) ON DELETE RESTRICT,
    CHECK((judgement='incorrect' AND incorrect_reason IS NOT NULL AND incorrect_reason IN ('no_match','unanswered','user'))
       OR (judgement IN ('correct','partial') AND incorrect_reason IS NULL))
);
CREATE TABLE response_assessments (
    exam_id TEXT NOT NULL,
    position INTEGER NOT NULL,
    judgement TEXT NOT NULL CHECK(judgement IN ('ungraded','correct','partial','incorrect')),
    incorrect_reason TEXT,
    assessment_revision INTEGER NOT NULL CHECK(assessment_revision >= 0),
    latest_event_id TEXT,
    PRIMARY KEY(exam_id,position),
    FOREIGN KEY(exam_id,position) REFERENCES exam_responses(exam_id,position) ON DELETE RESTRICT,
    FOREIGN KEY(exam_id,position,assessment_revision,latest_event_id)
      REFERENCES assessment_events(exam_id,position,assessment_revision,event_id) ON DELETE RESTRICT,
    CHECK((judgement='ungraded' AND assessment_revision=0 AND latest_event_id IS NULL AND incorrect_reason IS NULL)
       OR (judgement IN ('correct','partial','incorrect') AND assessment_revision>=1 AND latest_event_id IS NOT NULL)),
    CHECK((judgement='incorrect' AND incorrect_reason IS NOT NULL AND incorrect_reason IN ('no_match','unanswered','user'))
       OR (judgement!='incorrect' AND incorrect_reason IS NULL))
);
CREATE TABLE learning_events (
    learner_id TEXT NOT NULL REFERENCES learners(learner_id) ON DELETE RESTRICT,
    event_seq INTEGER NOT NULL CHECK(event_seq > 0),
    event_id TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK(kind IN ('exam_attempt','manual_review_add')),
    exam_id TEXT REFERENCES exams(exam_id) ON DELETE RESTRICT,
    dataset_id TEXT,
    item_id TEXT,
    recorded_at TEXT NOT NULL,
    PRIMARY KEY(learner_id,event_seq), UNIQUE(learner_id,exam_id),
    CHECK((kind='exam_attempt' AND exam_id IS NOT NULL AND dataset_id IS NULL AND item_id IS NULL)
       OR (kind='manual_review_add' AND exam_id IS NULL AND dataset_id IS NOT NULL AND item_id IS NOT NULL))
);
CREATE TABLE item_progress (
    learner_id TEXT NOT NULL REFERENCES learners(learner_id) ON DELETE RESTRICT,
    dataset_id TEXT NOT NULL,
    item_id TEXT NOT NULL,
    is_review_target INTEGER NOT NULL CHECK(is_review_target IN (0,1)),
    correct_streak INTEGER NOT NULL CHECK(correct_streak BETWEEN 0 AND 3),
    last_result TEXT CHECK(last_result IN ('correct','partial','incorrect')),
    last_completed_exam_id TEXT REFERENCES exams(exam_id) ON DELETE RESTRICT,
    progress_revision INTEGER NOT NULL CHECK(progress_revision >= 1),
    PRIMARY KEY(learner_id,dataset_id,item_id),
    CHECK((last_result IS NULL AND last_completed_exam_id IS NULL) OR (last_result IS NOT NULL AND last_completed_exam_id IS NOT NULL)),
    CHECK((is_review_target=1 AND correct_streak<3) OR (is_review_target=0 AND correct_streak IN (0,3)))
);
CREATE TABLE mutation_requests (
    learner_id TEXT NOT NULL REFERENCES learners(learner_id) ON DELETE RESTRICT,
    request_id TEXT NOT NULL,
    command TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    result_json TEXT NOT NULL CHECK(json_valid(result_json)),
    PRIMARY KEY(learner_id,request_id)
);
CREATE TRIGGER assessment_events_no_update BEFORE UPDATE ON assessment_events BEGIN SELECT RAISE(ABORT,'immutable assessment event'); END;
CREATE TRIGGER assessment_events_no_delete BEFORE DELETE ON assessment_events BEGIN SELECT RAISE(ABORT,'immutable assessment event'); END;
CREATE TRIGGER learning_events_no_update BEFORE UPDATE ON learning_events BEGIN SELECT RAISE(ABORT,'immutable learning event'); END;
CREATE TRIGGER learning_events_no_delete BEFORE DELETE ON learning_events BEGIN SELECT RAISE(ABORT,'immutable learning event'); END;
