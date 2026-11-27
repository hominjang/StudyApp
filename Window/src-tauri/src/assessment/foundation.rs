use super::*;
use crate::learning_format::{resolve_key_links, ItemKey, LearningLink};
use std::collections::HashMap;

impl AssessmentStore {
    pub fn foundation_links(
        &mut self,
        exam: &str,
        position: u32,
    ) -> AppResult<Vec<FoundationLink>> {
        uuid(exam)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        let (_, count) = completed_exam(&tx, &self.learner, exam)?;
        if position >= count {
            return Err(AppError::validation("position"));
        }
        let original = load_response(&tx, exam, position)?;
        let from = ItemKey {
            dataset_id: original.dataset,
            item_id: original.item,
        };
        let installed: HashMap<ItemKey, String> = tx
            .prepare("SELECT dataset_id,item_id,question FROM items WHERE is_active=1")?
            .query_map([], |r| {
                Ok((
                    ItemKey {
                        dataset_id: r.get(0)?,
                        item_id: r.get(1)?,
                    },
                    r.get(2)?,
                ))
            })?
            .collect::<Result<_, _>>()?;
        let mut refs = Vec::new();
        let mut statement=tx.prepare("SELECT dataset_id,item_id,payload_json FROM item_learning_links ORDER BY dataset_id,position")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let dataset: String = row.get(0)?;
            let item: String = row.get(1)?;
            let payload: String = row.get(2)?;
            let link: LearningLink = decode(&payload)?;
            if item != link.item_id || !is_valid_id(&dataset) || !is_valid_id(&item) {
                return Err(corrupt());
            }
            for to in link.prerequisites {
                if !is_valid_id(&to.dataset_id) || !is_valid_id(&to.item_id) {
                    return Err(corrupt());
                }
                refs.push((
                    ItemKey {
                        dataset_id: dataset.clone(),
                        item_id: item.clone(),
                    },
                    to,
                ));
            }
        }
        drop(rows);
        drop(statement);
        let keys = installed.keys().cloned().collect::<Vec<_>>();
        let edges = resolve_key_links(&keys, refs);
        let mut result = Vec::new();
        for link in edges.into_iter().filter(|e| e.from == from) {
            let issue = link.issue.map(|issue| format!("{issue:?}"));
            let p = if issue.is_none() {
                checked_progress(&tx, &self.learner, &link.to.dataset_id, &link.to.item_id)?
            } else {
                progress::Progress::default()
            };
            result.push(FoundationLink {
                question: installed.get(&link.to).cloned(),
                dataset_id: link.to.dataset_id,
                item_id: link.to.item_id,
                issue,
                is_review_target: p.target,
                progress_revision: p.revision,
            });
        }
        tx.commit()?;
        Ok(result)
    }
}
