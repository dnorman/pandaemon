use crate::{
    context::Context,
    error::{
        WriteError,
        RetrieveError
    },
    memorefhead::{
        MemoRefHead,
    },
    slab::{
        RelationSlotId,
        MemoId,
    },
    subject::{
        Subject,
        SubjectId,
        SubjectType
    },
};

use std::fmt;
use std::collections::HashMap;
use futures::{
    channel::mpsc,
};

#[derive(Clone)]
pub struct SubjectHandle {
    pub id: SubjectId,
    pub (crate) subject: Subject,
    pub (crate) context: Context
}

impl SubjectHandle{
    pub async fn new ( context: &Context, vals: HashMap<String, String> ) -> Result<SubjectHandle,WriteError> {

        let subject = Subject::new(&context, SubjectType::Record, vals ).await?;

        let handle = SubjectHandle{
            id: subject.id,
            subject: subject,
            context: context.clone()
        };

        Ok(handle)
    }
    pub fn new_blank ( context: &Context ) -> Result<SubjectHandle,WriteError> {
        Self::new( context, HashMap::new() )
    }
    pub fn new_kv ( context: &Context, key: &str, value: &str ) -> Result<SubjectHandle,WriteError> {
        let mut vals = HashMap::new();
        vals.insert(key.to_string(), value.to_string());

        Self::new( context, vals )
    }
    pub fn get_value ( &self, key: &str ) -> Option<String> {
        self.subject.get_value(&self.context, key).expect("Retrieval error. TODO: Convert to Result<..,RetrieveError>")
    }
    pub fn get_relation ( &self, key: RelationSlotId ) -> Result<Option<SubjectHandle>, RetrieveError> {

        match self.subject.get_relation(&self.context, key)?{
        Some(rel_sub_subject) => {
            Ok(Some(SubjectHandle{
                id: rel_sub_subject.id,
                context: self.context.clone(),
                subject: rel_sub_subject
            }))
            },
            None => Ok(None)
        }
    }
    pub fn set_value (&self, key: &str, value: &str) -> Result<bool,WriteError> {
        self.subject.set_value(&self.context, key, value)
    }
    pub fn set_relation (&self, key: RelationSlotId, relation: &Self) -> Result<(),WriteError> {
        self.subject.set_relation(&self.context, key, &relation.subject)
    }
    pub fn get_all_memo_ids ( &self ) -> Vec<MemoId> {
        self.subject.get_all_memo_ids( self.context.slab.clone() )
    }
    pub fn observe (&self) -> mpsc::Receiver<MemoRefHead> {
        self.subject.observe(&self.context.slab)
    }
}


impl fmt::Debug for SubjectHandle {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("Subject")
            .field("subject_id", &self.subject.id)
            .field("head", &self.subject.head)
            .finish()
    }
}