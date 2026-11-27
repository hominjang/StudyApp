use super::*;
use crate::{
    migration::{prepare_v2_copy, tests::setup},
    storage::AppPaths,
    worker::DbWorker,
};

// All data is synthetic and all writes use isolated temporary databases.
fn fixture() -> (tempfile::TempDir, std::path::PathBuf, AssessmentStore) {
    let (temp, paths, store) = setup();
    drop(store);
    let path = temp.path().join("candidate.db");
    prepare_v2_copy(&paths.db_dir.join("study.db"), &path).unwrap();
    let store = AssessmentStore::open(&path).unwrap();
    (temp, path, store)
}
fn review(dataset: &str, revision: u64) -> AddReviewRequest {
    AddReviewRequest {
        dataset_id: dataset.into(),
        item_id: "one".into(),
        expected_progress_revision: revision,
        request_id: Uuid::new_v4().to_string(),
    }
}
fn assessment(id: &str, revision: u64, judgement: Judgement) -> SetAssessmentRequest {
    SetAssessmentRequest {
        exam_id: id.into(),
        position: 0,
        expected_revision: revision,
        judgement,
        note: None,
        request_id: Uuid::new_v4().to_string(),
    }
}
fn count(s: &AssessmentStore, table: &str) -> i64 {
    s.db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
fn state(s: &AssessmentStore) -> progress::Progress {
    checked_progress(&s.db, &s.learner, "fixture-a", "one").unwrap()
}
fn attempt(s: &mut AssessmentStore, text: &str, descriptive: bool) -> String {
    let id = Uuid::new_v4().to_string();
    let time = Utc::now().to_rfc3339();
    let order: i64 =
        s.db.query_row(
            "SELECT COALESCE(MAX(completion_order),0)+1 FROM exams",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let seq: i64 =
        s.db.query_row(
            "SELECT COALESCE(MAX(event_seq),0)+1 FROM learning_events",
            [],
            |r| r.get(0),
        )
        .unwrap();
    s.db.execute("INSERT INTO exams(exam_id,learner_id,status,revision,dataset_ids_json,dataset_titles_snapshot_json,question_count,count_mode,order_mode,review_mode,current_position,grading_rule_version,started_at,saved_at,submitted_at,completed_at,completion_order,result_revision) VALUES (?1,?2,'completed',1,'[\"fixture-a\"]','[\"Synthetic\"]',1,'all','ordered','general',0,'exact-v1',?3,?3,?3,?3,?4,0)",params![id,s.learner,time,order]).unwrap();
    let criteria = if descriptive {
        Criteria::Descriptive {
            reference_answer: "Synthetic reference answer with several words".into(),
            key_points: vec!["Synthetic point".into()],
        }
    } else {
        Criteria::ShortAnswer {
            answer: "yes".into(),
            accepted_answers: vec!["ok".into()],
        }
    };
    let kind = if descriptive {
        "descriptive"
    } else {
        "short_answer"
    };
    s.db.execute(
        "INSERT INTO exam_responses VALUES (?1,0,'fixture-a','one',?2,'Synthetic question',?3,?4)",
        params![id, kind, json(&criteria).unwrap(), text],
    )
    .unwrap();
    s.db.execute(
        "INSERT INTO learning_events VALUES (?1,?2,?3,'exam_attempt',?4,NULL,NULL,?5)",
        params![s.learner, seq, Uuid::new_v4().to_string(), id, time],
    )
    .unwrap();
    if descriptive && !is_unanswered(text) {
        s.db.execute(
            "INSERT INTO response_assessments VALUES (?1,0,'ungraded',NULL,0,NULL)",
            [&id],
        )
        .unwrap();
    } else {
        let (j, why) = crate::exam::grade(text, "yes", &["ok".into()]);
        let event = Uuid::new_v4().to_string();
        s.db.execute(
            "INSERT INTO assessment_events VALUES (?1,?2,0,1,NULL,?3,?4,'automatic',?5,NULL,NULL)",
            params![event, id, j, why, time],
        )
        .unwrap();
        s.db.execute(
            "INSERT INTO response_assessments VALUES (?1,0,?2,?3,1,?4)",
            params![id, j, why, event],
        )
        .unwrap();
    }
    let old:Option<u64>=s.db.query_row("SELECT progress_revision FROM item_progress WHERE learner_id=?1 AND dataset_id='fixture-a' AND item_id='one'",[&s.learner],|r|sql_revision(r,0)).optional().unwrap();
    let p = compute_progress(&s.db, &s.learner, "fixture-a", "one").unwrap();
    if old.is_some() || p.last.is_some() || p.target {
        save_progress(&s.db, &s.learner, "fixture-a", "one", p, old.unwrap_or(0)).unwrap();
    }
    id
}

#[test]
fn descriptive_first_assessment_correction_and_counts_preserve_submitted_source() {
    let (_temp, _path, mut s) = fixture();
    let id = attempt(&mut s, "Synthetic original\r\nresponse", true);
    let v = s.assessment_view(&id).unwrap();
    assert_eq!(v.counts.ungraded, 1);
    assert_eq!(v.assessment_status, "pending");
    s.set_assessment(assessment(&id, 0, Judgement::Partial))
        .unwrap();
    assert!(state(&s).target);
    assert_eq!(state(&s).streak, 0);
    s.set_assessment(assessment(&id, 1, Judgement::Correct))
        .unwrap();
    let v = s.assessment_view(&id).unwrap();
    assert_eq!(v.counts.correct, 1);
    assert_eq!(v.assessment_status, "complete");
    assert_eq!(v.result_revision, 2);
    assert_eq!(v.items[0].original_judgement.as_deref(), Some("partial"));
    assert_eq!(v.items[0].response_text, "Synthetic original\r\nresponse");
    assert_eq!(count(&s, "exams"), 1);
}

#[test]
fn short_override_preserves_automatic_result_and_does_not_change_allowed_answers() {
    let (_temp, _path, mut s) = fixture();
    let id = attempt(&mut s, "wrong", false);
    s.set_assessment(assessment(&id, 1, Judgement::Correct))
        .unwrap();
    let v = s.assessment_view(&id).unwrap();
    assert_eq!(v.items[0].original_judgement.as_deref(), Some("incorrect"));
    assert_eq!(v.items[0].judgement, "correct");
    assert!(!state(&s).target);
    let criteria: String =
        s.db.query_row("SELECT criteria_json FROM items", [], |r| r.get(0))
            .unwrap();
    assert!(
        matches!(decode::<Criteria>(&criteria).unwrap(),Criteria::ShortAnswer{answer,..} if answer=="yes")
    );
    assert_eq!(count(&s, "assessment_events"), 2);
}

#[test]
fn late_assessment_uses_original_attempt_order_and_ungraded_is_neutral() {
    let (_temp, _path, mut s) = fixture();
    attempt(&mut s, "wrong", false);
    attempt(&mut s, "yes", false);
    let late = attempt(&mut s, "Synthetic description", true);
    attempt(&mut s, "yes", false);
    assert_eq!(state(&s).streak, 2);
    s.set_assessment(assessment(&late, 0, Judgement::Incorrect))
        .unwrap();
    assert_eq!(state(&s).streak, 1);
    assert!(state(&s).target);
    s.set_assessment(assessment(&late, 1, Judgement::Correct))
        .unwrap();
    assert_eq!(state(&s).streak, 3);
    assert!(!state(&s).target);
}

#[test]
fn manual_review_is_not_a_fake_attempt_and_duplicate_add_does_not_reset_streak() {
    let (_temp, path, mut s) = fixture();
    let add = review("fixture-a", 0);
    let result = s.add_review(add.clone()).unwrap();
    assert_eq!(result.progress_revision, 1);
    assert_eq!(count(&s, "exams"), 0);
    assert_eq!(count(&s, "assessment_events"), 0);
    assert_eq!(state(&s).last, None);
    assert_eq!(s.add_review(add).unwrap().outcome, "already_applied");
    attempt(&mut s, "yes", false);
    let revision = state(&s).revision;
    assert_eq!(
        s.add_review(review("fixture-a", revision)).unwrap().outcome,
        "unchanged"
    );
    assert_eq!(state(&s).streak, 1);
    attempt(&mut s, "yes", false);
    attempt(&mut s, "yes", false);
    assert!(!state(&s).target);
    let rev = state(&s).revision;
    s.add_review(review("fixture-a", rev)).unwrap();
    assert!(state(&s).target);
    assert_eq!(state(&s).streak, 0);
    drop(s);
    let s = AssessmentStore::open(&path).unwrap();
    assert!(state(&s).target);
    assert_eq!(count(&s, "exams"), 3);
}

#[test]
fn past_correction_replays_later_attempts_and_keeps_manual_event() {
    let (_temp, _path, mut s) = fixture();
    let old = attempt(&mut s, "wrong", false);
    for _ in 0..3 {
        attempt(&mut s, "yes", false);
    }
    assert!(!state(&s).target);
    let revision = state(&s).revision;
    s.add_review(review("fixture-a", revision)).unwrap();
    let n = count(&s, "learning_events");
    s.set_assessment(assessment(&old, 1, Judgement::Correct))
        .unwrap();
    assert_eq!(count(&s, "learning_events"), n);
    assert!(state(&s).target);
    assert_eq!(state(&s).streak, 0);
}

#[test]
fn request_retries_conflicts_and_same_result_are_idempotent() {
    let (_temp, _path, mut s) = fixture();
    let id = attempt(&mut s, "wrong", false);
    let request = assessment(&id, 1, Judgement::Correct);
    s.set_assessment(request.clone()).unwrap();
    assert_eq!(
        s.set_assessment(request.clone()).unwrap().outcome,
        "already_applied"
    );
    assert_eq!(count(&s, "assessment_events"), 2);
    let mut collision = request.clone();
    collision.judgement = Judgement::Partial;
    assert_eq!(
        s.set_assessment(collision).err().unwrap().code,
        "RequestConflict"
    );
    assert_eq!(
        s.set_assessment(assessment(&id, 1, Judgement::Partial))
            .err()
            .unwrap()
            .code,
        "AssessmentConflict"
    );
    assert_eq!(
        s.set_assessment(assessment(&id, 2, Judgement::Correct))
            .unwrap()
            .outcome,
        "unchanged"
    );
    assert_eq!(count(&s, "assessment_events"), 2);
    s.set_assessment(assessment(&id, 2, Judgement::Partial))
        .unwrap();
    assert_eq!(
        s.set_assessment(request).unwrap().assessment_revision,
        Some(2)
    );
    assert_eq!(
        s.assessment_view(&id).unwrap().items[0].judgement,
        "partial"
    );
}

#[test]
fn optimistic_conflict_between_connections_does_not_overwrite_new_assessment() {
    let (_temp, path, mut a) = fixture();
    let id = attempt(&mut a, "wrong", false);
    let mut b = AssessmentStore::open(&path).unwrap();
    a.set_assessment(assessment(&id, 1, Judgement::Correct))
        .unwrap();
    assert_eq!(
        b.set_assessment(assessment(&id, 1, Judgement::Partial))
            .err()
            .unwrap()
            .code,
        "AssessmentConflict"
    );
    assert_eq!(a.assessment_view(&id).unwrap().counts.correct, 1);
}

#[test]
fn mutation_record_failure_rolls_back_assessment_summary_and_progress() {
    let (_temp, _path, mut s) = fixture();
    let id = attempt(&mut s, "wrong", false);
    let before = state(&s);
    s.db.execute_batch("CREATE TRIGGER fail_request BEFORE INSERT ON mutation_requests BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(s
        .set_assessment(assessment(&id, 1, Judgement::Correct))
        .is_err());
    assert_eq!(count(&s, "assessment_events"), 1);
    assert_eq!(state(&s), before);
    assert_eq!(s.assessment_view(&id).unwrap().result_revision, 0);
    assert_eq!(count(&s, "mutation_requests"), 0);
}

#[test]
fn manual_write_failure_does_not_leave_event_or_fake_progress() {
    let (_temp, _path, mut s) = fixture();
    s.db.execute_batch("CREATE TRIGGER fail_request BEFORE INSERT ON mutation_requests BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(s.add_review(review("fixture-a", 0)).is_err());
    assert_eq!(count(&s, "learning_events"), 0);
    assert_eq!(count(&s, "item_progress"), 0);
}

#[test]
fn unanswered_is_not_correctable_but_zero_width_input_is_not_unanswered() {
    let (_temp, _path, mut s) = fixture();
    for text in ["", "\u{0085}\u{00a0}\u{3000}"] {
        let id = attempt(&mut s, text, true);
        assert_eq!(
            s.set_assessment(assessment(&id, 1, Judgement::Correct))
                .err()
                .unwrap()
                .code,
            "AssessmentNotAllowed"
        );
    }
    let id = attempt(&mut s, "\u{feff}", true);
    s.set_assessment(assessment(&id, 0, Judgement::Correct))
        .unwrap();
    assert_eq!(s.assessment_view(&id).unwrap().counts.correct, 1);
}

#[test]
fn corrupt_affected_source_blocks_writes_but_unrelated_item_can_be_added() {
    let (_temp, _path, mut s) = fixture();
    let id = attempt(&mut s, "wrong", false);
    s.db.execute("UPDATE response_assessments SET judgement='correct',incorrect_reason=NULL WHERE exam_id=?1",[&id]).unwrap();
    assert_eq!(
        s.set_assessment(assessment(&id, 1, Judgement::Correct))
            .err()
            .unwrap()
            .code,
        "ProgressSourceCorrupt"
    );
    assert!(s
        .add_review(review("fixture-a", state_revision(&s)))
        .is_err());
    let criteria = json(&Criteria::ShortAnswer {
        answer: "x".into(),
        accepted_answers: vec![],
    })
    .unwrap();
    s.db.execute(
        "INSERT INTO datasets VALUES ('fixture-b','Synthetic B',1,1,'synthetic-hash',NULL)",
        [],
    )
    .unwrap();
    s.db.execute(
        "INSERT INTO items VALUES ('fixture-b','one',0,'short_answer','Synthetic question',?1,1)",
        [criteria],
    )
    .unwrap();
    s.add_review(review("fixture-b", 0)).unwrap();
    assert_eq!(count(&s, "assessment_events"), 1);
}
fn state_revision(s: &AssessmentStore) -> u64 {
    s.db.query_row("SELECT progress_revision FROM item_progress", [], |r| {
        sql_revision(r, 0)
    })
    .unwrap()
}

#[test]
fn inactive_missing_target_stale_review_revision_and_invalid_request_are_rejected() {
    let (_temp, _path, mut s) = fixture();
    assert_eq!(
        s.add_review(review("fixture-a", 1)).err().unwrap().code,
        "ProgressConflict"
    );
    let mut req = review("fixture-a", 0);
    req.request_id = "invalid".into();
    assert!(s.add_review(req).is_err());
    s.db.execute("UPDATE items SET is_active=0", []).unwrap();
    assert_eq!(
        s.add_review(review("fixture-a", 0)).err().unwrap().code,
        "LinkedItemUnavailable"
    );
    assert_eq!(
        s.add_review(review("missing", 0)).err().unwrap().code,
        "LinkedItemUnavailable"
    );
    assert_eq!(count(&s, "learning_events"), 0);
}

#[test]
fn worker_dispatches_v2_commands_but_never_converts_v1_database() {
    let (temp, paths, store) = setup();
    drop(store);
    let source = paths.db_dir.join("study.db");
    let before = std::fs::read(&source).unwrap();
    let worker = DbWorker::start(paths.clone()).unwrap();
    assert_eq!(
        worker
            .add_review(review("fixture-a", 0))
            .err()
            .unwrap()
            .code,
        "UnsupportedDbVersion"
    );
    assert_eq!(std::fs::read(&source).unwrap(), before);
    let target = temp.path().join("v2");
    std::fs::create_dir(&target).unwrap();
    prepare_v2_copy(&source, &target.join("study.db")).unwrap();
    let mut store = AssessmentStore::open(&target.join("study.db")).unwrap();
    let exam = attempt(&mut store, "yes", false);
    drop(store);
    let worker = DbWorker::start(AppPaths {
        db_dir: target,
        initial_dir: paths.initial_dir,
    })
    .unwrap();
    assert_eq!(
        worker.add_review(review("fixture-a", 1)).unwrap().outcome,
        "saved"
    );
    assert_eq!(
        worker.assessment_view(exam.clone()).unwrap().counts.correct,
        1
    );
    worker
        .set_assessment(assessment(&exam, 1, Judgement::Partial))
        .unwrap();
    let view = worker.assessment_view(exam).unwrap();
    assert_eq!(view.counts.partial, 1);
    assert!(view.items[0].is_review_target);
}

#[test]
fn result_and_mutation_require_completed_exam_owned_by_current_learner() {
    let (_temp, _path, mut s) = fixture();
    let id = attempt(&mut s, "wrong", false);
    let other = Uuid::new_v4().to_string();
    s.db.execute(
        "INSERT INTO learners VALUES (?1,?2)",
        params![other, Utc::now().to_rfc3339()],
    )
    .unwrap();
    s.db.execute(
        "UPDATE exams SET learner_id=?1 WHERE exam_id=?2",
        params![other, id],
    )
    .unwrap();
    assert_eq!(
        s.assessment_view(&id).err().unwrap().code,
        "OwnershipMismatch"
    );
    assert_eq!(
        s.set_assessment(assessment(&id, 1, Judgement::Correct))
            .err()
            .unwrap()
            .code,
        "OwnershipMismatch"
    );
    s.db.execute("UPDATE exams SET learner_id=?1,status='submitted',completed_at=NULL,completion_order=NULL WHERE exam_id=?2",params![s.learner,id]).unwrap();
    assert_eq!(s.assessment_view(&id).err().unwrap().code, "InvalidState");
    assert_eq!(count(&s, "assessment_events"), 1);
}

#[test]
fn corrupt_cache_is_not_silently_repaired_and_other_command_request_id_cannot_be_reused() {
    let (_temp, _path, mut s) = fixture();
    let id = attempt(&mut s, "wrong", false);
    let request = assessment(&id, 1, Judgement::Correct);
    s.set_assessment(request.clone()).unwrap();
    let mut add = review("fixture-a", state(&s).revision);
    add.request_id = request.request_id;
    assert_eq!(s.add_review(add).err().unwrap().code, "RequestConflict");
    s.db.execute(
        "UPDATE item_progress SET is_review_target=1,correct_streak=0",
        [],
    )
    .unwrap();
    assert_eq!(
        s.add_review(review("fixture-a", state_revision(&s)))
            .err()
            .unwrap()
            .code,
        "ProgressSourceCorrupt"
    );
    assert_eq!(count(&s, "learning_events"), 1);
}
