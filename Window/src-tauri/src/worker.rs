use crate::assessment::{
    AddReviewRequest, AssessmentStore, AssessmentView, FoundationLink, MutationResult,
    SetAssessmentRequest,
};
use crate::exam::{CandidatePreview, CandidateRequest, ExamHistoryRow, RecordEntry};
use crate::exam::{
    ExamResult, ExamSession, PendingExam, SaveProgressResult, StartExamRequest, SubmitExamRequest,
};
use crate::runtime_store::RuntimeStore;
use crate::study_material::{StudyPage, StudyRequest};
use crate::transfer::{
    EditMaterialRequest, EditMaterialResult, ImportPreview, ImportRequest, ImportResult,
    ImportState, MaterialEditState,
};
use crate::{
    error::{AppError, AppResult},
    storage::{AppPaths, AppStatus, DatasetSummary},
};
use std::sync::mpsc::{self, Sender};

enum Job {
    EditMaterial(EditMaterialRequest, Sender<AppResult<EditMaterialResult>>),
    StudyMaterials(StudyRequest, Sender<AppResult<StudyPage>>),
    PreviewImport(ImportRequest, Sender<AppResult<ImportPreview>>),
    ApplyImport(String, Sender<AppResult<ImportResult>>),
    Foundation(String, u32, Sender<AppResult<Vec<FoundationLink>>>),
    Assessment(String, Sender<AppResult<AssessmentView>>),
    SetAssessment(SetAssessmentRequest, Sender<AppResult<MutationResult>>),
    AddReview(AddReviewRequest, Sender<AppResult<MutationResult>>),
    Status(Sender<AppResult<AppStatus>>),
    Datasets(Sender<AppResult<Vec<DatasetSummary>>>),
    Start(StartExamRequest, Sender<AppResult<ExamSession>>),
    Session(String, Sender<AppResult<ExamSession>>),
    Submit(SubmitExamRequest, Sender<AppResult<ExamSession>>),
    Complete(String, Sender<AppResult<ExamSession>>),
    Result(String, Sender<AppResult<ExamResult>>),
    Save(SubmitExamRequest, Sender<AppResult<SaveProgressResult>>),
    Pending(Sender<AppResult<Vec<RecordEntry<PendingExam>>>>),
    Candidates(CandidateRequest, Sender<AppResult<CandidatePreview>>),
    History(String, Sender<AppResult<Vec<RecordEntry<ExamHistoryRow>>>>),
}

impl Job {
    fn fail(self, error: AppError) {
        match self {
            Self::EditMaterial(_, reply) => {
                let _ = reply.send(Err(error));
            }
            Self::StudyMaterials(_, reply) => {
                let _ = reply.send(Err(error));
            }
            Self::PreviewImport(_, reply) => {
                let _ = reply.send(Err(error));
            }
            Self::ApplyImport(_, reply) => {
                let _ = reply.send(Err(error));
            }
            Self::Foundation(_, _, reply) => {
                let _ = reply.send(Err(error));
            }
            Self::Assessment(_, reply) => {
                let _ = reply.send(Err(error));
            }
            Self::SetAssessment(_, reply) | Self::AddReview(_, reply) => {
                let _ = reply.send(Err(error));
            }
            Self::Status(reply) => {
                let _ = reply.send(Err(error));
            }
            Self::Datasets(reply) => {
                let _ = reply.send(Err(error));
            }
            Self::Start(_, reply)
            | Self::Session(_, reply)
            | Self::Submit(_, reply)
            | Self::Complete(_, reply) => {
                let _ = reply.send(Err(error));
            }
            Self::Result(_, reply) => {
                let _ = reply.send(Err(error));
            }
            Self::Save(_, reply) => {
                let _ = reply.send(Err(error));
            }
            Self::Pending(reply) => {
                let _ = reply.send(Err(error));
            }
            Self::Candidates(_, reply) => {
                let _ = reply.send(Err(error));
            }
            Self::History(_, reply) => {
                let _ = reply.send(Err(error));
            }
        }
    }
}

#[derive(Clone)]
pub struct DbWorker {
    sender: Sender<Job>,
}

impl DbWorker {
    pub fn start(paths: AppPaths) -> AppResult<Self> {
        let (sender, receiver) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("studyapp-db".into())
            .spawn(move || {
                let mut store: Option<RuntimeStore> = None;
                let mut assessments: Option<AssessmentStore> = None;
                let mut imports = ImportState::default();
                let mut edits = MaterialEditState::default();
                while let Ok(job) = receiver.recv() {
                    let job = match job {
                        Job::Foundation(id, position, reply) => {
                            let _ = reply.send(with_assessments(&mut assessments, &paths, |s| {
                                s.foundation_links(&id, position)
                            }));
                            continue;
                        }
                        Job::Assessment(id, reply) => {
                            let _ = reply.send(with_assessments(&mut assessments, &paths, |s| {
                                s.assessment_view(&id)
                            }));
                            continue;
                        }
                        Job::SetAssessment(request, reply) => {
                            let _ = reply.send(with_assessments(&mut assessments, &paths, |s| {
                                s.set_assessment(request)
                            }));
                            continue;
                        }
                        Job::AddReview(request, reply) => {
                            let _ = reply.send(with_assessments(&mut assessments, &paths, |s| {
                                s.add_review(request)
                            }));
                            continue;
                        }
                        job => job,
                    };
                    if store.is_none() {
                        match RuntimeStore::open(&paths) {
                            Ok(opened) => store = Some(opened),
                            Err(error) => {
                                job.fail(error);
                                continue;
                            }
                        }
                    }
                    let current = store.as_mut().unwrap();
                    match job {
                        Job::EditMaterial(request, reply) => {
                            let (db, version) = current.transfer_connection();
                            let _ = reply.send(edits.save(db, version, request));
                        }
                        Job::StudyMaterials(request, reply) => {
                            let _ = reply.send(current.study_materials(request));
                        }
                        Job::PreviewImport(request, reply) => {
                            let (db, version) = current.transfer_connection();
                            let _ = reply.send(imports.preview(db, version, request));
                        }
                        Job::ApplyImport(token, reply) => {
                            let (db, version) = current.transfer_connection();
                            let _ = reply.send(imports.apply(db, version, &token));
                        }
                        Job::Foundation(..)
                        | Job::Assessment(..)
                        | Job::SetAssessment(..)
                        | Job::AddReview(..) => {
                            unreachable!()
                        }
                        Job::Status(reply) => {
                            let _ = reply.send(current.status());
                        }
                        Job::Datasets(reply) => {
                            let _ = reply.send(current.list_datasets());
                        }
                        Job::Start(request, reply) => {
                            let _ = reply.send(current.start_exam(request));
                        }
                        Job::Session(id, reply) => {
                            let _ = reply.send(current.exam_session(&id));
                        }
                        Job::Submit(request, reply) => {
                            let _ = reply.send(current.submit_exam(request));
                        }
                        Job::Complete(id, reply) => {
                            let _ = reply.send(current.complete_exam(&id));
                        }
                        Job::Result(id, reply) => {
                            let _ = reply.send(current.exam_result(&id));
                        }
                        Job::Save(request, reply) => {
                            let _ = reply.send(current.save_progress(request));
                        }
                        Job::Pending(reply) => {
                            let _ = reply.send(current.pending_exams());
                        }
                        Job::Candidates(request, reply) => {
                            let _ = reply.send(current.preview_candidates(request));
                        }
                        Job::History(filter, reply) => {
                            let _ = reply.send(current.exam_history(&filter));
                        }
                    }
                }
            })
            .map_err(|_| {
                AppError::new("StorageFailure", "저장소 작업을 시작하지 못했어요.", true)
            })?;
        Ok(Self { sender })
    }

    pub fn assessment_view(&self, id: String) -> AppResult<AssessmentView> {
        self.request(|reply| Job::Assessment(id, reply))
    }
    pub fn edit_material(&self, request: EditMaterialRequest) -> AppResult<EditMaterialResult> {
        self.request(|reply| Job::EditMaterial(request, reply))
    }
    pub fn study_materials(&self, request: StudyRequest) -> AppResult<StudyPage> {
        self.request(|reply| Job::StudyMaterials(request, reply))
    }
    pub fn preview_import(&self, request: ImportRequest) -> AppResult<ImportPreview> {
        self.request(|reply| Job::PreviewImport(request, reply))
    }
    pub fn apply_import(&self, token: String) -> AppResult<ImportResult> {
        self.request(|reply| Job::ApplyImport(token, reply))
    }
    pub fn set_assessment(&self, request: SetAssessmentRequest) -> AppResult<MutationResult> {
        self.request(|reply| Job::SetAssessment(request, reply))
    }
    pub fn add_review(&self, request: AddReviewRequest) -> AppResult<MutationResult> {
        self.request(|reply| Job::AddReview(request, reply))
    }

    pub fn foundation_links(&self, id: String, position: u32) -> AppResult<Vec<FoundationLink>> {
        self.request(|reply| Job::Foundation(id, position, reply))
    }
    pub fn status(&self) -> AppResult<AppStatus> {
        let (reply, receive) = mpsc::channel();
        self.sender
            .send(Job::Status(reply))
            .map_err(|_| disconnected())?;
        receive.recv().map_err(|_| disconnected())?
    }

    pub fn datasets(&self) -> AppResult<Vec<DatasetSummary>> {
        let (reply, receive) = mpsc::channel();
        self.sender
            .send(Job::Datasets(reply))
            .map_err(|_| disconnected())?;
        receive.recv().map_err(|_| disconnected())?
    }

    fn request<T>(&self, make_job: impl FnOnce(Sender<AppResult<T>>) -> Job) -> AppResult<T> {
        let (reply, receive) = mpsc::channel();
        self.sender
            .send(make_job(reply))
            .map_err(|_| disconnected())?;
        receive.recv().map_err(|_| disconnected())?
    }
    pub fn start_exam(&self, request: StartExamRequest) -> AppResult<ExamSession> {
        self.request(|reply| Job::Start(request, reply))
    }
    pub fn exam_session(&self, id: String) -> AppResult<ExamSession> {
        self.request(|reply| Job::Session(id, reply))
    }
    pub fn submit_exam(&self, request: SubmitExamRequest) -> AppResult<ExamSession> {
        self.request(|reply| Job::Submit(request, reply))
    }
    pub fn complete_exam(&self, id: String) -> AppResult<ExamSession> {
        self.request(|reply| Job::Complete(id, reply))
    }
    pub fn exam_result(&self, id: String) -> AppResult<ExamResult> {
        self.request(|reply| Job::Result(id, reply))
    }
    pub fn save_progress(&self, request: SubmitExamRequest) -> AppResult<SaveProgressResult> {
        self.request(|reply| Job::Save(request, reply))
    }
    pub fn pending_exams(&self) -> AppResult<Vec<RecordEntry<PendingExam>>> {
        self.request(Job::Pending)
    }
    pub fn preview_candidates(&self, request: CandidateRequest) -> AppResult<CandidatePreview> {
        self.request(|reply| Job::Candidates(request, reply))
    }
    pub fn exam_history(&self, filter: String) -> AppResult<Vec<RecordEntry<ExamHistoryRow>>> {
        self.request(|reply| Job::History(filter, reply))
    }
}

fn disconnected() -> AppError {
    AppError::new(
        "StorageFailure",
        "저장소 연결을 확인하지 못했어요. 앱을 다시 실행해주세요.",
        true,
    )
}

fn with_assessments<T>(
    store: &mut Option<AssessmentStore>,
    paths: &AppPaths,
    job: impl FnOnce(&mut AssessmentStore) -> AppResult<T>,
) -> AppResult<T> {
    if store.is_none() {
        *store = Some(AssessmentStore::open(&paths.db_dir.join("study.db"))?);
    }
    job(store.as_mut().unwrap())
}
