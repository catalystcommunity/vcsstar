use std::time::Duration;

use vcsstar_core::{queue::Queue, App, Code, Config};

#[test]
#[ignore = "requires a test database and a Corndogs server with keyed submissions and task guards"]
fn submission_conflicts_and_lost_claims() {
    let config = Config::from_env().unwrap();
    let db = vcsstar_db::pool(&config.database_url, 1).unwrap();
    let queue = Queue::connect(&config).unwrap();
    let app = App::new(config, db);
    let queue_name = format!("test-{}", uuid::Uuid::now_v7());
    let key = "one-task";

    app.submit_task(&queue_name, key, vec![1], 0).unwrap();
    app.submit_task(&queue_name, key, vec![1], 0).unwrap();
    let conflict = app.submit_task(&queue_name, key, vec![2], 0).unwrap_err();
    assert_eq!(conflict.code, Code::AlreadyExists);

    let claim = queue
        .claim(&[queue_name], Duration::from_secs(60))
        .unwrap()
        .unwrap();
    assert_eq!(claim.payload, [1]);
    let mut stale = claim.clone();
    stale.revision -= 1;
    assert!(!queue.park(&stale, Duration::from_secs(1)).unwrap());
    assert!(queue.complete(&claim).unwrap());
    assert!(!queue.complete(&claim).unwrap());
}
