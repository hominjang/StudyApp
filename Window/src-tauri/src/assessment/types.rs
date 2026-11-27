use crate::learning_format::Criteria;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Judgement {
    Correct,
    Partial,
    Incorrect,
}
impl Judgement {
    pub(super) fn text(&self) -> &'static str {
        match self {
            Self::Correct => "correct",
            Self::Partial => "partial",
            Self::Incorrect => "incorrect",
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetAssessmentRequest {
    pub exam_id: String,
    pub position: u32,
    pub expected_revision: u64,
    pub judgement: Judgement,
    pub note: Option<String>,
    pub request_id: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AddReviewRequest {
    pub dataset_id: String,
    pub item_id: String,
    pub expected_progress_revision: u64,
    pub request_id: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MutationResult {
    pub outcome: String,
    pub assessment_revision: Option<u64>,
    pub progress_revision: u64,
    pub result_revision: Option<u64>,
}
#[derive(Debug, Serialize, Default, PartialEq, Eq)]
pub struct AssessmentCounts {
    pub correct: u32,
    pub partial: u32,
    pub incorrect: u32,
    pub ungraded: u32,
}
#[derive(Debug, Serialize)]
pub struct AssessmentItem {
    pub position: u32,
    pub question: String,
    pub response_text: String,
    pub criteria: Criteria,
    pub original_judgement: Option<String>,
    pub judgement: String,
    pub incorrect_reason: Option<String>,
    pub assessment_revision: u64,
    pub is_review_target: bool,
    pub correct_streak: u8,
    pub progress_revision: u64,
}
#[derive(Debug, Serialize)]
pub struct AssessmentView {
    pub exam_id: String,
    pub result_revision: u64,
    pub assessment_status: String,
    pub counts: AssessmentCounts,
    pub items: Vec<AssessmentItem>,
}

#[derive(Debug, Serialize)]
pub struct FoundationLink {
    pub dataset_id: String,
    pub item_id: String,
    pub question: Option<String>,
    pub issue: Option<String>,
    pub is_review_target: bool,
    pub progress_revision: u64,
}
