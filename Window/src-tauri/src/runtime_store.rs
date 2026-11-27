use crate::{
    assessment::AssessmentStore,
    error::{AppError, AppResult},
    exam::*,
    storage::{AppPaths, AppStatus, DatasetSummary, Store},
};
use rusqlite::{Connection, OpenFlags};

pub(crate) enum RuntimeStore {
    Legacy(Store),
    V2(AssessmentStore),
}
macro_rules! forward {
    ($($name:ident($($arg:ident:$ty:ty),*) -> $result:ty;)*) => {$ (
        pub fn $name(&mut self,$($arg:$ty),*)->AppResult<$result> {
            match self {Self::Legacy(s)=>s.$name($($arg),*),Self::V2(s)=>s.$name($($arg),*)}
        }
    )*};
}
impl RuntimeStore {
    pub fn study_materials(
        &mut self,
        request: crate::study_material::StudyRequest,
    ) -> AppResult<crate::study_material::StudyPage> {
        let (connection, version) = self.transfer_connection();
        crate::study_material::browse(connection, version, request)
    }
    pub(crate) fn transfer_connection(&mut self) -> (&mut Connection, u8) {
        match self {
            Self::Legacy(s) => (&mut s.connection, 1),
            Self::V2(s) => (&mut s.db, 2),
        }
    }
    pub fn open(paths: &AppPaths) -> AppResult<Self> {
        let path = paths.db_dir.join("study.db");
        if !path.exists() {
            return Ok(Self::V2(AssessmentStore::initialize(paths)?));
        }
        let probe = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let version: String = probe.query_row(
            "SELECT value FROM app_metadata WHERE key='db_schema_version'",
            [],
            |r| r.get(0),
        )?;
        drop(probe);
        match version.as_str() {
            "2" => Ok(Self::V2(AssessmentStore::open(&path)?)),
            "1" => Ok(Self::Legacy(Store::open(paths)?)),
            _ => Err(AppError::new(
                "UnsupportedDbVersion",
                "지원하지 않는 저장소 버전이에요. 기존 파일은 유지됩니다.",
                false,
            )),
        }
    }
    forward! {
        status()->AppStatus;
        list_datasets()->Vec<DatasetSummary>;
        preview_candidates(request:CandidateRequest)->CandidatePreview;
        start_exam(request:StartExamRequest)->ExamSession;
        exam_session(id:&str)->ExamSession;
        save_progress(request:SubmitExamRequest)->SaveProgressResult;
        submit_exam(request:SubmitExamRequest)->ExamSession;
        complete_exam(id:&str)->ExamSession;
        exam_result(id:&str)->ExamResult;
        pending_exams()->Vec<RecordEntry<PendingExam>>;
        exam_history(filter:&str)->Vec<RecordEntry<ExamHistoryRow>>;
    }
}
