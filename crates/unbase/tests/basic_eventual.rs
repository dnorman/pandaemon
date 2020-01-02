extern crate unbase;
use unbase::subject::Subject;
use unbase::error::*;
use timer::Delay;
use std::time;
use futures_await_test::async_test;
use futures::future::RemoteHandle;

use tracing::debug;

#[async_test]
async fn basic_eventual() {

    let net = unbase::Network::create_new_system();
    let simulator = unbase::util::simulator::Simulator::new();
    net.add_transport( Box::new(simulator.clone()) );

    simulator.start();

    let slab_a = unbase::Slab::new(&net);
    let slab_b = unbase::Slab::new(&net);
    let slab_c = unbase::Slab::new(&net);

    assert!(slab_a.id == 0, "Slab A ID shoud be 0");
    assert!(slab_b.id == 1, "Slab B ID shoud be 1");
    assert!(slab_c.id == 2, "Slab C ID shoud be 2");

    assert!(slab_a.peer_slab_count() == 2, "Slab A Should know two peers" );
    assert!(slab_b.peer_slab_count() == 2, "Slab B Should know two peers" );
    assert!(slab_c.peer_slab_count() == 2, "Slab C Should know two peers" );


    let context_a = slab_a.create_context();
    let context_b = slab_b.create_context();
    let context_c = slab_c.create_context();

    let rec_a1 = Subject::new_kv(&context_a, "animal_sound", "Moo").await;

    assert!(rec_a1.is_ok(), "New subject should be created");
    let rec_a1 = rec_a1.unwrap();

    assert!(rec_a1.get_value("animal_sound").await.unwrap() == "Moo", "New subject should be internally consistent");

    // consolidation is necessary for eventual consistency to work
    //context_a.fully_consolidate();

    // These are going to be fairly variable now that we are using memo-based indexing
    // TODO: Find a better way to measure the intent here

    //assert_eq!(slab_a.count_of_memorefs_resident(), 2, "Slab A should have 2 memorefs resident");
    //assert_eq!(slab_b.count_of_memorefs_resident(), 0, "Slab B should have 1 memorefs resident");
    //assert_eq!(slab_c.count_of_memorefs_resident(), 0, "Slab C should have 1 memorefs resident");
    debug!("New subject ID {}", rec_a1.id );

    let record_id = rec_a1.id;
    let root_index_subject_id = if let Some(ref s) = *context_a.root_index.read().unwrap() {
        s.get_root_id()
    }else{
        panic!("sanity error - uninitialized context");
    };

    let context_b_copy = context_b.clone();
    let context_c_copy = context_c.clone();
    let handle: RemoteHandle<()> = unbase::util::task::spawn_with_handle(  (async move || {
            assert!(context_b_copy.get_subject_by_id(record_id).await.unwrap_err() == RetrieveError::NotFound, "new subject should not yet have conveyed to slab B");
            assert!(context_c_copy.get_subject_by_id(record_id).await.unwrap_err() == RetrieveError::NotFound, "new subject should not yet have conveyed to slab C");
    })());

    simulator.quiescence().await;

    handle.await;

    //assert!(slab_a.count_of_memorefs_resident() == 2, "Slab A should have 2 memorefs resident");
    //assert!(slab_b.count_of_memorefs_resident() == 2, "Slab B should have 2 memorefs resident");
    //assert!(slab_c.count_of_memorefs_resident() == 2, "Slab C should have 2 memorefs resident");

    // HERE - in the case of eventual consistency, it might take several
    // seconds for this to convey – not just a single clock tick
    // We've made the index artificially chatty for now, but this will
    // change to a timeout-based process once context::subject_graph is working
    simulator.quiescence().await;

    debug!("Root Index = {:?}", context_b.get_subject_head_memo_ids(root_index_subject_id)  );
    // Temporary way to magically, instantly send context
    debug!("Manually exchanging context from Context A to Context B - Count of MemoRefs: {}", context_a.hack_send_context(&context_b) );
    debug!("Manually exchanging context from Context A to Context C - Count of MemoRefs: {}", context_a.hack_send_context(&context_c) );
    debug!("Root Index = {:?}", context_b.get_subject_head_memo_ids(root_index_subject_id)  );


    let context_b_copy = context_b.clone();
    let context_c_copy = context_c.clone();
    let simulator_copy = simulator.clone();
    let handle: RemoteHandle<()> = unbase::util::task::spawn_with_handle(  (async move || {
        let simulator = simulator_copy;

        let rec_b1 = context_b_copy.get_subject_by_id( record_id ).await;
        let rec_c1 = context_c_copy.get_subject_by_id( record_id ).await;

        assert!(rec_b1.is_ok(), "new subject should now have conveyed to slab B");
        assert!(rec_c1.is_ok(), "new subject should now have conveyed to slab C");

        let rec_b1 = rec_b1.unwrap();
        let rec_c1 = rec_c1.unwrap();

        assert!(rec_b1.get_value("animal_sound").await.unwrap() == "Moo", "Subject read from Slab B should be internally consistent");
        assert!(rec_c1.get_value("animal_sound").await.unwrap() == "Moo", "Subject read from Slab C should be internally consistent");

        simulator.quiescence().await;

        assert_eq!(rec_a1.get_value("animal_sound").await.unwrap(), "Moo");
        assert_eq!(rec_b1.get_value("animal_sound").await.unwrap(), "Moo");
        assert_eq!(rec_c1.get_value("animal_sound").await.unwrap(), "Moo");


        rec_b1.set_value("animal_type","Bovine");
        assert_eq!(rec_b1.get_value("animal_type").await.unwrap(), "Bovine");
        assert_eq!(rec_b1.get_value("animal_sound").await.unwrap(),   "Moo");

        simulator.quiesce_and_stop().await;

        rec_b1.set_value("animal_sound","Woof");
        rec_b1.set_value("animal_type","Kanine");
        assert_eq!(rec_b1.get_value("animal_sound").await.unwrap(), "Woof");
        assert_eq!(rec_b1.get_value("animal_type").await.unwrap(),  "Kanine");

        // Should not yet have propagated to slab A
        assert_eq!(rec_a1.get_value("animal_sound").await.unwrap(),   "Moo");
        assert!(rec_a1.get_value("animal_type").await.is_none(), "Should not yet have a value on Slab A for animal_type");

        simulator.start(); // advance the simulator clock by one tick

        // Nowwww it should have propagated
        assert_eq!(rec_a1.get_value("animal_sound").await.unwrap(),   "Woof");
        assert_eq!(rec_a1.get_value("animal_type").await.unwrap(),    "Kanine");
    })());

    handle.await;
/*

    let idx_node = Subject::new_kv(&context_b, "dummy","value").unwrap();
    idx_node.set_relation( 0, rec_b1 );

    debug!("All rec_b1 MemoIds: {:?}", rec_b1_memoids);
    slab_b.remotize_memo_ids( &rec_b1_memoids ).expect("failed to remotize memos");

    if let Some(record) = idx_node.get_relation(0) {
        debug!("Retrieved record: {} - {:?}", record.id, record.get_value("animal_sound") );
    }

    let rec_b2 = Subject::new_kv(&context_a, "animal_sound","Meow");
    let rec_b3 = Subject::new_kv(&context_a, "animal_sound","Ribbit");

    rec_b2.set_relation( 1, rec_b1 );
    */

    // TODO: drop the referenced memos, ensuring we only have the remote memorefs Present
    // TODO: test relation changing/projection
    // TODO: fix/test subject reconstitution for relationship traversal (it's duping now)
    // TODO: build the index class using this primative
    // TODO: figure out how to bootstrap subject index, given that the subject index
    //       needs a (probably lesser) subject index to locate its index nodes

    //rec_b1.drop();

/*
    // Time moves forward
    net.deliver_all_memos();

    let rec_b1 = context_b.get_subject_by_id( rec_a1.id );
    assert!(rec_b1.is_ok(), "new subject should now be available on slab B");
    let rec_b1 = rec_b1.unwrap();

    assert!(rec_b1.get_value("animal_sound").unwrap() == "moo", "Transferred subject should be consistent");


    // Time moves forward
    net.deliver_all_memos();

    let rec_c1 = context_c.get_subject_by_id( rec_a1.id );
    assert!(rec_c1.is_ok(), "new subject should now be available on slab C");
    let mut rec_c1 = rec_c1.unwrap();

    assert!(rec_c1.get_value("animal_sound").unwrap() == "moo", "Transferred subject should be consistent");

    // Time moves forward
    net.deliver_all_memos();

    assert!( rec_c1.set_value("animal_sound", "woof"), "Change the value on slab C" );
    assert!( rec_c1.get_value("animal_sound").unwrap() == "woof", "Updated subject should be consistent");

    assert!( rec_a1.get_value("animal_sound").unwrap() == "moo", "Value should be unchanged on slab A" );

    // Time moves forward
    net.deliver_all_memos();

    assert!( rec_a1.get_value("animal_sound").unwrap() == "woof", "Now the value should be changed on slab A" );
*/

}
