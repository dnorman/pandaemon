use std::collections::HashMap;
use crate::memorefhead::MemoRefHead;
use crate::slab::*;

pub struct SystemCreator;

impl SystemCreator {

    pub fn generate_root_index_seed( slab: &SlabHandle ) -> MemoRefHead {

        let mut values = HashMap::new();
        values.insert("tier".to_string(),0.to_string());

        let memoref = slab.new_memo_basic_noparent(
            Some(slab.generate_subject_id()),
            MemoBody::FullyMaterialized {v: values, r: RelationSlotSubjectHead(HashMap::new()) }
        );

        MemoRefHead::from_memoref(memoref)
    }

}
