CREATE TABLE app_metadata (
    key TEXT NOT NULL PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE learners (
    learner_id TEXT NOT NULL PRIMARY KEY,
    created_at TEXT NOT NULL
);
CREATE TABLE datasets (
    dataset_id TEXT NOT NULL PRIMARY KEY,
    title TEXT NOT NULL,
    position INTEGER NOT NULL CHECK(position >= 0),
    revision INTEGER NOT NULL CHECK(revision >= 1),
    content_hash TEXT NOT NULL,
    metadata_json TEXT
);
CREATE TABLE items (
    dataset_id TEXT NOT NULL,
    item_id TEXT NOT NULL,
    position INTEGER NOT NULL CHECK(position >= 0),
    question TEXT NOT NULL,
    answer TEXT NOT NULL,
    accepted_answers_json TEXT NOT NULL,
    is_active INTEGER NOT NULL CHECK(is_active IN (0,1)),
    PRIMARY KEY(dataset_id, item_id),
    FOREIGN KEY(dataset_id) REFERENCES datasets(dataset_id) ON DELETE RESTRICT
);
CREATE UNIQUE INDEX ux_items_active_position
    ON items(dataset_id, position) WHERE is_active=1;
CREATE TABLE supplements (
    dataset_id TEXT NOT NULL,
    item_id TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    PRIMARY KEY(dataset_id, item_id),
    FOREIGN KEY(dataset_id, item_id) REFERENCES items(dataset_id, item_id) ON DELETE RESTRICT
);
CREATE TABLE exams (
    exam_id TEXT NOT NULL PRIMARY KEY,
    learner_id TEXT NOT NULL REFERENCES learners(learner_id) ON DELETE RESTRICT,
    status TEXT NOT NULL CHECK(status IN ('in_progress','submitted','completed')),
    revision INTEGER NOT NULL CHECK(revision >= 0),
    dataset_ids_json TEXT NOT NULL,
    dataset_titles_snapshot_json TEXT NOT NULL,
    question_count INTEGER NOT NULL CHECK(question_count > 0),
    count_mode TEXT NOT NULL CHECK(count_mode IN ('all','count')),
    order_mode TEXT NOT NULL CHECK(order_mode IN ('random','ordered')),
    review_mode TEXT NOT NULL CHECK(review_mode IN ('general','wrong_first','wrong_only')),
    current_position INTEGER NOT NULL CHECK(current_position >= 0 AND current_position < question_count),
    grading_rule_version TEXT NOT NULL,
    started_at TEXT NOT NULL,
    saved_at TEXT NOT NULL,
    submitted_at TEXT,
    completed_at TEXT,
    completion_order INTEGER,
    UNIQUE(learner_id, completion_order),
    CHECK (
        (status='in_progress' AND submitted_at IS NULL AND completed_at IS NULL AND completion_order IS NULL)
        OR (status='submitted' AND submitted_at IS NOT NULL AND completed_at IS NULL AND completion_order IS NULL)
        OR (status='completed' AND submitted_at IS NOT NULL AND completed_at IS NOT NULL AND completion_order > 0 AND completion_order IS NOT NULL)
    )
);
CREATE TABLE exam_responses (
    exam_id TEXT NOT NULL REFERENCES exams(exam_id) ON DELETE RESTRICT,
    position INTEGER NOT NULL CHECK(position >= 0),
    dataset_id TEXT NOT NULL,
    item_id TEXT NOT NULL,
    question_snapshot TEXT NOT NULL,
    answer_snapshot TEXT NOT NULL,
    accepted_answers_snapshot_json TEXT NOT NULL,
    response_text TEXT NOT NULL,
    judgement TEXT NOT NULL CHECK(judgement IN ('ungraded','correct','incorrect')),
    incorrect_reason TEXT,
    PRIMARY KEY(exam_id, position),
    UNIQUE(exam_id, dataset_id, item_id),
    CHECK (
        (judgement IN ('ungraded','correct') AND incorrect_reason IS NULL)
        OR (judgement='incorrect' AND incorrect_reason IN ('no_match','unanswered') AND incorrect_reason IS NOT NULL)
    )
);
CREATE TABLE item_progress (
    learner_id TEXT NOT NULL REFERENCES learners(learner_id) ON DELETE RESTRICT,
    dataset_id TEXT NOT NULL,
    item_id TEXT NOT NULL,
    is_review_target INTEGER NOT NULL CHECK(is_review_target IN (0,1)),
    correct_streak INTEGER NOT NULL CHECK(correct_streak BETWEEN 0 AND 3),
    last_result TEXT NOT NULL CHECK(last_result IN ('correct','incorrect')),
    last_completed_exam_id TEXT NOT NULL REFERENCES exams(exam_id) ON DELETE RESTRICT,
    PRIMARY KEY(learner_id, dataset_id, item_id),
    CHECK ((is_review_target=1 AND correct_streak < 3) OR (is_review_target=0 AND correct_streak IN (0,3)))
);
CREATE INDEX ix_exams_history ON exams(learner_id, status, completion_order);
CREATE INDEX ix_response_item_history ON exam_responses(dataset_id, item_id, exam_id);
