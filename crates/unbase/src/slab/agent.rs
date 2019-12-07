use std::sync::{Arc,RwLock,Mutex};
use std::collections::hash_map::Entry;

use crate::slab::{SlabId, MemoRef, MemoBody, Memo, MemoInner, SlabRefInner, MemoRefInner, MemoRefPtr, MemoPeerList, MemoPeeringStatus, MemoId, MemoPeer, SlabPresence};
use crate::slab::state::SlabState;
use crate::network::{SlabRef, TransmitterArgs, Transmitter, TransportAddress};
use crate::Network;
use crate::subject::SubjectId;
use crate::memorefhead::MemoRefHead;
use crate::context::WeakContext;

pub struct SlabAgent {
    pub id: SlabId,
    state: RwLock<SlabState>,
    net: Network,
    my_ref: SlabRef,
}

impl SlabAgent {
    pub fn new ( net: &Network, my_ref: SlabRef ) -> Self {
        let state = RwLock::new(SlabState::new() );

        SlabAgent {
            id: my_ref.slab_id,
            state: state,
            net: net.clone(),
            my_ref: my_ref
        }
    }
    pub fn new_memo ( &self, subject_id: Option<SubjectId>, parents: MemoRefHead, body: MemoBody) -> MemoRef {
        let memo_id = {
            let mut state = self.state.write().unwrap();
            state.counters.last_memo_id += 1;
            (self.id as u64).rotate_left(32) | state.counters.last_memo_id as u64
        };

        //println!("# Slab({}).new_memo(id: {},subject_id: {:?}, parents: {:?}, body: {:?})", self.id, memo_id, subject_id, parents.memo_ids(), body );

        let memo = Memo::new(MemoInner {
            id:    memo_id,
            owning_slab_id: self.id,
            subject_id: subject_id,
            parents: parents,
            body: body
        });

        let (memoref, _had_memoref) = self.assert_memoref(memo.id, memo.subject_id, MemoPeerList(Vec::new()), Some(memo) );
        self.consider_emit_memo(&memoref);

        memoref
    }
    pub fn consider_emit_memo(&self, memoref: &MemoRef) {
        // Emit memos for durability and notification purposes
        // At present, some memos like peering and slab presence are emitted manually.
        // TODO: This will almost certainly have to change once gossip/plumtree functionality is added

        // TODO: test each memo for durability_score and emit accordingly
        if let Some(memo) = memoref.get_memo_if_resident() {
            let needs_peers = self.check_peering_target(&memo);


            //println!("Slab({}).consider_emit_memo {} - A ({:?})", self.id, memoref.id, &*self.peer_refs.read().unwrap() );
            let state = self.state.read().unwrap();
            for peer_ref in state.peer_refs.iter().filter(|x| !memoref.is_peered_with_slabref(x) ).take( needs_peers as usize ) {

                //println!("# Slab({}).emit_memos - EMIT Memo {} to Slab {}", self.id, memo.id, peer_ref.slab_id );
                peer_ref.send( &self.my_ref, memoref );
            }
        }
    }
    fn check_peering_target( &self, memo: &Memo ) -> u8 {
        if memo.does_peering() {
            5
        }else{
            // This is necessary to prevent memo routing loops for now, as
            // memoref.is_peered_with_slabref() obviously doesn't work for non-peered memos
            // something here should change when we switch to gossip/plumtree, but
            // I'm not sufficiently clear on that at the time of this writing
            0
        }
    }
    pub fn reconstitute_memo ( &self, memo_id: MemoId, subject_id: Option<SubjectId>, parents: MemoRefHead, body: MemoBody, origin_slabref: &SlabRef, peerlist: &MemoPeerList ) -> (Memo,MemoRef,bool){
        //println!("Slab({}).reconstitute_memo({})", self.id, memo_id );
        // TODO: find a way to merge this with assert_memoref to avoid doing duplicative work with regard to peerlist application

        let memo = Memo::new(MemoInner {
            id:             memo_id,
            owning_slab_id: self.id,
            subject_id:     subject_id,
            parents:        parents,
            body:           body
        });

        let (memoref, had_memoref) = self.assert_memoref(memo.id, memo.subject_id, peerlist.clone(), Some(memo.clone()) );

        {
            let mut state = self.state.write().unwrap();
            state.counters.memos_received += 1;
            if had_memoref {
                state.counters.memos_redundantly_received += 1;
            }
        }
        //println!("Slab({}).reconstitute_memo({}) B -> {:?}", self.id, memo_id, memoref );


        self.consider_emit_memo(&memoref);

        if let Some(ref memo) = memoref.get_memo_if_resident() {

            self.check_memo_waiters(memo);
            self.handle_memo_from_other_slab(memo, &memoref, &origin_slabref);
            self.do_peering(&memoref, &origin_slabref);

        }

        if let Some(ref tx_mutex) = self.memoref_dispatch_tx_channel {
            tx_mutex.lock().unwrap().send(memoref.clone()).unwrap()
        }

        (memo, memoref, had_memoref)
    }
    pub fn recv_memoref (&self, memoref : MemoRef){
        //println!("# \t\\ Slab({}).dispatch_memoref({})", self.id, &memoref.id );

        if let Some(subject_id) = memoref.subject_id {

            let maybe_sub : Option<Vec<WeakContext>> = {
                // we want to make sure the lock is released before continuing
                if let Some(ref s) = self.subject_subscriptions.read().unwrap().get( &subject_id ) {
                    Some((*s).clone())
                }else{
                    None
                }
            };

            if let Some(subscribers) = maybe_sub {

                for weakcontext in subscribers {

                    if let Some(context) = weakcontext.upgrade() {

                        context.apply_subject_head( subject_id, &memoref.to_head(), true );
                    }
                }
            }

        }
    }
    pub fn residentize_memoref(&self, memoref: &MemoRef, memo: Memo) -> bool {
        //println!("# Slab({}).MemoRef({}).residentize()", self.id, memoref.id);

        assert!(memoref.owning_slab_id == self.id);
        assert!( memoref.id == memo.id );

        let mut ptr = memoref.ptr.write().unwrap();

        if let MemoRefPtr::Remote = *ptr {
            *ptr = MemoRefPtr::Resident( memo );

            // should this be using do_peering_for_memo?
            // doing it manually for now, because I think we might only want to do
            // a concise update to reflect our peering status change

            let peering_memoref = self.new_memo(
                None,
                MemoRefHead::from_memoref(memoref.clone()),
                MemoBody::Peering(
                    memoref.id,
                    memoref.subject_id,
                    MemoPeerList::new(vec![ MemoPeer{
                        slabref: self.my_ref.clone(),
                        status: MemoPeeringStatus::Resident
                    }])
                )
            );

            for peer in memoref.peerlist.read().unwrap().iter() {
                peer.slabref.send( &self.my_ref, &peering_memoref );
            }

            // residentized
            true
        }else{
            // already resident
            false
        }
    }
    pub fn remotize_memoref( &self, memoref: &MemoRef ) -> Result<(),String> {
        assert!(memoref.owning_slab_id == self.id);

        //println!("# Slab({}).MemoRef({}).remotize()", self.id, memoref.id );

        // TODO: check peering minimums here, and punt if we're below threshold

        let send_peers;
        {
            let mut ptr = memoref.ptr.write().unwrap();
            if let MemoRefPtr::Resident(_) = *ptr {
                let peerlist = memoref.peerlist.read().unwrap();

                if peerlist.len() == 0 {
                    return Err("Cannot remotize a zero-peer memo".to_string());
                }
                send_peers = peerlist.clone();
                *ptr = MemoRefPtr::Remote;

            }else{
                return Ok(());
            }
        }

        let peering_memoref = self.new_memo_basic(
            None,
            MemoRefHead::from_memoref(memoref.clone()),
            MemoBody::Peering(
                memoref.id,
                memoref.subject_id,
                MemoPeerList::new(vec![MemoPeer{
                    slabref: self.my_ref.clone(),
                    status: MemoPeeringStatus::Participating
                }])
            )
        );

        //self.consider_emit_memo(&memoref);

        for peer in send_peers.iter() {
            peer.slabref.send( &self.my_ref, &peering_memoref );
        }

        Ok(())
    }
    pub fn request_memo (&self, memoref: &MemoRef) -> u8 {
        //println!("Slab({}).request_memo({})", self.id, memoref.id );

        let request_memo = self.new_memo_basic(
            None,
            MemoRefHead::new(), // TODO: how should this be parented?
            MemoBody::MemoRequest(
                vec![memoref.id],
                self.my_ref.clone()
            )
        );

        let mut sent = 0u8;
        for peer in memoref.peerlist.read().unwrap().iter().take(5) {
            //println!("Slab({}).request_memo({}) from {}", self.id, memoref.id, peer.slabref.slab_id );
            peer.slabref.send( &self.my_ref, &request_memo.clone() );
            sent += 1;
        }

        sent
    }
    pub fn assert_memoref( &self, memo_id: MemoId, subject_id: Option<SubjectId>, peerlist: MemoPeerList, memo: Option<Memo>) -> (MemoRef, bool) {

        let had_memoref;
        let memoref = match self.state.write().unwrap().memorefs_by_id.entry(memo_id) {
            Entry::Vacant(o)   => {
                let mr = MemoRef(Arc::new(
                    MemoRefInner {
                        id: memo_id,
                        owning_slab_id: self.id,
                        subject_id: subject_id,
                        peerlist: RwLock::new(peerlist),
                        ptr:      RwLock::new(match memo {
                            Some(m) => {
                                assert!(self.id == m.owning_slab_id);
                                MemoRefPtr::Resident(m)
                            }
                            None    => MemoRefPtr::Remote
                        })
                    }
                ));

                had_memoref = false;
                o.insert( mr ).clone()// TODO: figure out how to prolong the borrow here & avoid clone
            }
            Entry::Occupied(o) => {
                let mr = o.get();
                had_memoref = true;
                if let Some(m) = memo {

                    let mut ptr = mr.ptr.write().unwrap();
                    if let MemoRefPtr::Remote = *ptr {
                        *ptr = MemoRefPtr::Resident(m)
                    }
                }
                mr.apply_peers( &peerlist );
                mr.clone()
            }
        };

        (memoref, had_memoref)
    }
    pub fn assert_slabref(&self, slab_id: SlabId, presence: &[SlabPresence] ) -> SlabRef {
        //println!("# Slab({}).assert_slabref({}, {:?})", self.id, slab_id, presence );

        if slab_id == self.id {
            return self.my_ref.clone();
            // don't even look it up if it's me.
            // We must not allow any third party to edit the peering.
            // Also, my ref won't appeara in the list of peer_refs, because it's not a peer
        }

        let maybe_slabref = {
            // Instead of having to scope our read lock, and getting a write lock later
            // should we be using a single write lock for the full function scope?
            if let Some(slabref) = self.peer_refs.read().expect("peer_refs.read()").iter().find(|r| r.0.slab_id == slab_id ){
                Some(slabref.clone())
            }else{
                None
            }
        };

        let slabref : SlabRef;
        if let Some(s) = maybe_slabref {
            slabref = s;
        }else{
            let inner = SlabRefInner {
                slab_id:        slab_id,
                owning_slab_id: self.id, // for assertions only?
                presence:       RwLock::new(Vec::new()),
                tx:             Mutex::new(Transmitter::new_blackhole(slab_id)),
                return_address: RwLock::new(TransportAddress::Blackhole),
            };

            slabref = SlabRef(Arc::new(inner));
            self.peer_refs.write().expect("peer_refs.write()").push(slabref.clone());
        }

        if slab_id == slabref.owning_slab_id {
            return slabref; // no funny business. You don't get to tell me how to reach me
        }

        for p in presence.iter(){
            assert!(slab_id == p.slab_id, "presence slab_id does not match the provided slab_id");

            let mut _maybe_slab = None;
            let args = if p.address.is_local() {
                // playing silly games with borrow lifetimes.
                // TODO: make this less ugly
                _maybe_slab = self.net.get_slabhandle(p.slab_id);

                if let Some(ref slab) = _maybe_slab {
                    TransmitterArgs::Local(slab)
                }else{
                    continue;
                }
            }else{
                TransmitterArgs::Remote( &p.slab_id, &p.address )
            };
             // Returns true if this presence is new to the slabref
             // False if we've seen this presence already

            if slabref.apply_presence(p) {

                let new_trans = self.net.get_transmitter( &args ).expect("assert_slabref net.get_transmitter");
                let return_address = self.net.get_return_address( &p.address ).expect("return address not found");

                *slabref.0.tx.lock().expect("tx.lock()") = new_trans;
                *slabref.0.return_address.write().expect("return_address write lock") = return_address;
            }
        }

        return slabref;

    }
}

impl Drop for SlabAgent {
    fn drop(&mut self) {
        self.net.deregister_local_slab(self.id);
    }
}