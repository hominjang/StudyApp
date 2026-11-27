use super::*;
use crate::{
    exam::SubmitExamRequest,
    storage::{AppPaths, Store},
};
use serde_json::json;
use std::fs;

// All DBs and questions in this module are synthetic temp fixtures.
pub(crate) fn setup() -> (tempfile::TempDir, AppPaths, Store) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("initial");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("catalog.json"),serde_json::to_vec(&json!({"schema_version":"1.0","datasets":[{"dataset_id":"fixture-a","title":"Synthetic","learning_file":"learning.json","question_count":1}]})).unwrap()).unwrap();
    fs::write(source.join("learning.json"),serde_json::to_vec(&json!({"schema_version":"1.0","dataset_id":"fixture-a","title":"Synthetic","items":[{"id":"one","question":"Synthetic question","answer":"yes","accepted_answers":["ok"]}]})).unwrap()).unwrap();
    let paths = AppPaths {
        db_dir: temp.path().join("db"),
        initial_dir: source,
    };
    let store = Store::open(&paths).unwrap();
    (temp, paths, store)
}
pub(crate) fn attempt(store: &mut Store, text: &str, phase: &str) -> String {
    let s = store
        .start_exam(StartExamRequest {
            exam_id: Uuid::new_v4().to_string(),
            dataset_ids: vec!["fixture-a".into()],
            count_mode: "all".into(),
            count: None,
            order_mode: "ordered".into(),
            review_mode: "general".into(),
        })
        .unwrap();
    let request = SubmitExamRequest {
        exam_id: s.exam_id.clone(),
        revision: 1,
        current_position: 0,
        responses: vec![text.into()],
    };
    if phase == "in_progress" {
        store.save_progress(request).unwrap();
    } else {
        store.submit_exam(request).unwrap();
        if phase == "completed" {
            store.complete_exam(&s.exam_id).unwrap();
        }
    }
    s.exam_id
}
fn count(db: &Connection, table: &str) -> i64 {
    db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
fn values(db: &Connection, sql: &str) -> Vec<Vec<rusqlite::types::Value>> {
    let mut s = db.prepare(sql).unwrap();
    let columns = s.column_count();
    let mut rows = s.query([]).unwrap();
    let mut result = Vec::new();
    while let Some(r) = rows.next().unwrap() {
        result.push((0..columns).map(|i| r.get(i).unwrap()).collect());
    }
    result
}

#[test]
fn conversion_preserves_history_order_original_text_and_draft_states() {
    let (temp, paths, mut store) = setup();
    for answer in ["no", "yes", "ok", "yes", "no"] {
        attempt(&mut store, answer, "completed");
    }
    let draft = attempt(&mut store, "first\r\nsecond\rthird\nfourth", "in_progress");
    let pending = attempt(&mut store, "\u{0085}", "submitted");
    drop(store);
    let source = paths.db_dir.join("study.db");
    let before = fs::read(&source).unwrap();
    let output = temp.path().join("candidate.db");
    let report = prepare_v2_copy(&source, &output).unwrap();
    assert_eq!(
        report,
        MigrationReport {
            item_count: 1,
            response_count: 7,
            assessment_count: 5,
            attempt_count: 5
        }
    );
    assert_eq!(fs::read(&source).unwrap(), before);
    let old = Connection::open_with_flags(&source, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let new = Connection::open(&output).unwrap();
    assert_eq!(
        metadata(&new, "db_schema_version").unwrap().as_deref(),
        Some("2")
    );
    for sql in ["SELECT exam_id,learner_id,status,revision,dataset_ids_json,dataset_titles_snapshot_json,question_count,count_mode,order_mode,review_mode,current_position,grading_rule_version,started_at,saved_at,submitted_at,completed_at,completion_order FROM exams ORDER BY exam_id",
        "SELECT learner_id,dataset_id,item_id,is_review_target,correct_streak,last_result,last_completed_exam_id FROM item_progress ORDER BY dataset_id,item_id",
        "SELECT dataset_id,title,position,revision,content_hash,metadata_json FROM datasets ORDER BY dataset_id"] {assert_eq!(values(&old,sql),values(&new,sql));}
    let criteria: String = new
        .query_row(
            "SELECT criteria_snapshot_json FROM exam_responses WHERE exam_id=?1",
            [&draft],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Criteria>(&criteria).unwrap(),
        Criteria::ShortAnswer {
            answer: "yes".into(),
            accepted_answers: vec!["ok".into()]
        }
    );
    let original: Vec<(String, String)> = old
        .prepare("SELECT exam_id,response_text FROM exam_responses ORDER BY exam_id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let migrated: Vec<(String, String)> = new
        .prepare("SELECT exam_id,response_text FROM exam_responses ORDER BY exam_id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(original, migrated);
    for id in [&draft, &pending] {
        let state:(i64,String)=new.query_row("SELECT a.assessment_revision,a.judgement FROM response_assessments a WHERE exam_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(state, (0, "ungraded".into()));
    }
    let order: Vec<i64> = new
        .prepare("SELECT event_seq FROM learning_events ORDER BY event_seq")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(order, vec![1, 2, 3, 4, 5]);
    let p: (bool, i64) = new
        .query_row(
            "SELECT is_review_target,correct_streak FROM item_progress",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(p, (true, 0));
}

#[test]
fn empty_history_and_three_correct_release_migrate_without_fabricated_attempts() {
    for answered in [false, true] {
        let (temp, paths, mut store) = setup();
        if answered {
            for text in ["wrong", "yes", "yes", "yes"] {
                attempt(&mut store, text, "completed");
            }
        }
        drop(store);
        let output = temp.path().join("candidate.db");
        let r = prepare_v2_copy(&paths.db_dir.join("study.db"), &output).unwrap();
        let db = Connection::open(output).unwrap();
        assert_eq!(r.assessment_count, if answered { 4 } else { 0 });
        assert_eq!(count(&db, "learning_events"), if answered { 4 } else { 0 });
        if answered {
            assert_eq!(
                db.query_row("SELECT correct_streak FROM item_progress", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                3
            );
        } else {
            assert_eq!(count(&db, "item_progress"), 0);
        }
    }
}

#[test]
fn inconsistent_progress_or_completed_judgement_is_rejected_without_output() {
    for damage in [
        "UPDATE item_progress SET correct_streak=1",
        "UPDATE exam_responses SET judgement='correct',incorrect_reason=NULL",
    ] {
        let (temp, paths, mut store) = setup();
        attempt(&mut store, "wrong", "completed");
        drop(store);
        let source = paths.db_dir.join("study.db");
        let db = Connection::open(&source).unwrap();
        db.execute_batch(damage).unwrap();
        drop(db);
        let before = fs::read(&source).unwrap();
        let out = temp.path().join("candidate.db");
        assert!(prepare_v2_copy(&source, &out).is_err());
        assert!(!out.exists());
        assert_eq!(fs::read(&source).unwrap(), before);
    }
}

#[test]
fn failure_after_schema_rebuild_rolls_back_all_original_tables_and_metadata() {
    let (_temp, paths, mut store) = setup();
    attempt(&mut store, "wrong", "completed");
    drop(store);
    let mut db = Connection::open(paths.db_dir.join("study.db")).unwrap();
    db.execute_batch("PRAGMA foreign_keys=OFF; CREATE TRIGGER fail_version BEFORE UPDATE ON app_metadata WHEN NEW.value='2' BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(migrate_copy(&mut db).is_err());
    assert_eq!(
        metadata(&db, "db_schema_version").unwrap().as_deref(),
        Some("1")
    );
    assert_eq!(count(&db, "items"), 1);
    assert_eq!(count(&db, "exam_responses"), 1);
    assert_eq!(
        db.query_row("SELECT answer FROM items", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "yes"
    );
    assert!(db.prepare("SELECT * FROM assessment_events").is_err());
}

#[test]
fn failed_candidate_is_removed_and_existing_destination_is_never_overwritten() {
    let (temp, paths, store) = setup();
    drop(store);
    let source = paths.db_dir.join("study.db");
    let output = temp.path().join("candidate.db");
    fs::write(&output, b"synthetic existing file").unwrap();
    assert_eq!(
        prepare_v2_copy(&source, &output).err().unwrap().code,
        "DestinationExists"
    );
    assert_eq!(fs::read(&output).unwrap(), b"synthetic existing file");
    assert_eq!(
        prepare_v2_copy(&source, &source).err().unwrap().code,
        "DestinationExists"
    );
    let db = Connection::open(&source).unwrap();
    db.execute_batch("CREATE TRIGGER fail_version BEFORE UPDATE ON app_metadata WHEN NEW.value='2' BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    drop(db);
    let failed = temp.path().join("failed.db");
    assert!(prepare_v2_copy(&source, &failed).is_err());
    assert!(!failed.exists());
    assert_eq!(
        fs::read_dir(temp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with("study.v2."))
            .count(),
        0
    );
}

#[test]
fn backup_includes_committed_wal_pages_without_checkpointing_source() {
    let (temp, paths, store) = setup();
    drop(store);
    let source = paths.db_dir.join("study.db");
    let writer = Connection::open(&source).unwrap();
    writer.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; UPDATE items SET question='Synthetic WAL question';").unwrap();
    assert!(source.with_extension("db-wal").exists());
    let output = temp.path().join("candidate.db");
    prepare_v2_copy(&source, &output).unwrap();
    let new = Connection::open(output).unwrap();
    assert_eq!(
        new.query_row("SELECT question FROM items", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "Synthetic WAL question"
    );
}

#[test]
fn unsupported_or_missing_sources_do_not_create_an_output_or_reinitialize_source() {
    let (temp, paths, store) = setup();
    drop(store);
    let source = paths.db_dir.join("study.db");
    let db = Connection::open(&source).unwrap();
    db.execute(
        "UPDATE app_metadata SET value='99' WHERE key='db_schema_version'",
        [],
    )
    .unwrap();
    drop(db);
    let before = fs::read(&source).unwrap();
    let output = temp.path().join("candidate.db");
    assert_eq!(
        prepare_v2_copy(&source, &output).err().unwrap().code,
        "UnsupportedDbVersion"
    );
    assert!(!output.exists());
    assert_eq!(fs::read(&source).unwrap(), before);
    let missing = temp.path().join("missing.db");
    assert!(prepare_v2_copy(&missing, &output).is_err());
    assert!(!missing.exists());
}

#[test]
fn v2_candidate_rejects_repeat_conversion_and_old_store_cannot_open_it() {
    let (temp, paths, store) = setup();
    drop(store);
    let folder = temp.path().join("v2");
    fs::create_dir(&folder).unwrap();
    let output = folder.join("study.db");
    prepare_v2_copy(&paths.db_dir.join("study.db"), &output).unwrap();
    let before = fs::read(&output).unwrap();
    assert_eq!(
        prepare_v2_copy(&output, &temp.path().join("again.db"))
            .err()
            .unwrap()
            .code,
        "UnsupportedDbVersion"
    );
    assert_eq!(
        Store::open(&AppPaths {
            db_dir: folder,
            initial_dir: paths.initial_dir
        })
        .err()
        .unwrap()
        .code,
        "UnsupportedDbVersion"
    );
    assert_eq!(fs::read(&output).unwrap(), before);
}

#[test]
fn v2_schema_accepts_unattempted_manual_targets_and_protects_immutable_events() {
    let (temp, paths, mut store) = setup();
    let exam = attempt(&mut store, "wrong", "completed");
    drop(store);
    let output = temp.path().join("candidate.db");
    prepare_v2_copy(&paths.db_dir.join("study.db"), &output).unwrap();
    let db = Connection::open(output).unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    let learner = metadata(&db, "local_learner_id").unwrap().unwrap();
    db.execute(
        "INSERT INTO item_progress VALUES (?1,'fixture-a','unattempted',1,0,NULL,NULL,1)",
        [&learner],
    )
    .unwrap();
    let event = Uuid::new_v4().to_string();
    db.execute("INSERT INTO learning_events VALUES (?1,2,?2,'manual_review_add',NULL,'fixture-a','unattempted','synthetic-time')",params![learner,event]).unwrap();
    assert_eq!(count(&db, "exams"), 1);
    assert_eq!(count(&db, "assessment_events"), 1);
    assert!(db
        .execute("DELETE FROM assessment_events WHERE exam_id=?1", [&exam])
        .is_err());
    assert!(db
        .execute("UPDATE learning_events SET event_seq=99", [])
        .is_err());
    assert!(db
        .execute(
            "UPDATE response_assessments SET latest_event_id='unrelated'",
            []
        )
        .is_err());
    let criteria = serde_json::to_string(&Criteria::Descriptive {
        reference_answer: "one two three four five\nnext".into(),
        key_points: vec!["point".into()],
    })
    .unwrap();
    db.execute("INSERT INTO items VALUES ('fixture-a','descriptive',1,'descriptive','Synthetic question',?1,1)",[criteria]).unwrap();
    integrity(&db).unwrap();
    assert!(db.execute("INSERT INTO items VALUES ('fixture-a','invalid',2,'descriptive','Synthetic question','{}',1)",[]).is_err());
}

#[test]
fn malformed_basic_columns_and_orphaned_response_counts_block_conversion() {
    for damage in [
        "UPDATE exams SET completion_order='bad'",
        "DELETE FROM exam_responses",
        "UPDATE datasets SET metadata_json='invalid json'",
    ] {
        let (temp, paths, mut store) = setup();
        attempt(&mut store, "yes", "completed");
        drop(store);
        let source = paths.db_dir.join("study.db");
        let db = Connection::open(&source).unwrap();
        db.execute_batch(damage).unwrap();
        drop(db);
        let output = temp.path().join("candidate.db");
        assert!(prepare_v2_copy(&source, &output).is_err());
        assert!(!output.exists());
    }
}
