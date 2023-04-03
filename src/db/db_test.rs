mod test {
    use crate::db::db::{Timestamp, DB};

    // A read running into an uncommitted intent with a lower timestamp will wait for the
    // earlier transaction

    /**
     * A read running into an uncommitted intent with a lower timestamp will
     * wait for the earlier transaction to finalize.
     *
     * A read running into an uncommitted intent with a higher timestamp ignores the
     * intent and does not need to wait.
     */
    mod write_read {
        mod uncommitted_intent_has_lower_timestamp {

            use std::sync::Arc;

            use tokio::time::{self, sleep, Duration};

            use crate::db::db::{Timestamp, DB};

            #[tokio::test]
            async fn read_waits_for_uncommitted_write() {
                let db = Arc::new(DB::new("./tmp/data", Timestamp::new(10)));
                let write_txn = db.begin_txn().await;
                let key = "foo";
                db.write(key, 12, write_txn).await;
                db.set_time(Timestamp::new(12));
                let read_txn = db.begin_txn().await;

                let db_1 = db.clone();
                let task_1 = tokio::spawn(async move {
                    let read_res = db_1.read::<i32>(key, read_txn).await;
                    db_1.commit_txn(write_txn).await;
                    assert_eq!(read_res, Some(12));
                });

                let db_2 = db.clone();
                let task_2 = tokio::spawn(async move {
                    db_2.commit_txn(write_txn).await;
                });
                tokio::try_join!(task_1, task_2).unwrap();
            }
        }

        // A read running into an uncommitted intent with a higher timestamp ignores the
        // intent and does not need to wait.
        mod uncommitted_intent_has_higher_timestamp {
            use std::sync::Arc;

            use crate::db::db::{Timestamp, DB};

            #[tokio::test]
            async fn ignores_intent_with_higher_timestamp() {
                let db = Arc::new(DB::new("./tmp/data", Timestamp::new(10)));
                let read_txn = db.begin_txn().await;
                let key = "foo";

                db.set_time(Timestamp::new(12));
                let write_txn = db.begin_txn().await;
                db.write(key, 12, write_txn).await;
                let read_res = db.read::<i32>(key, read_txn).await;
                assert!(read_res.is_none());
            }
        }
    }

    mod write_write {
        mod run_into_uncommitted_intent {}

        mod run_into_committed_intent {
            use std::sync::Arc;

            use crate::{
                db::db::{CommitTxnResult, Timestamp, DB},
                hlc::{
                    clock::{Clock, ManualClock},
                    timestamp::Timestamp as HLCTimestamp,
                },
            };

            #[tokio::test]
            async fn bump_write_timestamp_before_committing() {
                let db = Arc::new(DB::new("./tmp/data", Timestamp::new(10)));
                let key = "foo";

                // begin txn1
                let write_txn_1 = db.begin_txn().await;

                // time = 12
                db.set_time(Timestamp::new(12));

                // begin txn2. txn2 writes and commits
                let write_txn_2 = db.begin_txn().await;
                db.write(key, 12, write_txn_2).await;
                let txn_2_commit_res = db.commit_txn(write_txn_2).await;
                let txn_2_commit_timestamp = match txn_2_commit_res {
                    CommitTxnResult::Success(res) => {
                        assert_eq!(res.commit_timestamp.wall_time, 12);
                        res.commit_timestamp
                    }
                    CommitTxnResult::Fail => panic!("failed to commit"),
                };

                // txn1 writes
                db.write(key, 15, write_txn_1).await;

                // txn1 attempts to commit - it should advance
                let commit_res = db.commit_txn(write_txn_1).await;

                match commit_res {
                    CommitTxnResult::Success(res) => {
                        assert_eq!(
                            res.commit_timestamp,
                            txn_2_commit_timestamp.next_logical_timestamp()
                        );
                    }
                    CommitTxnResult::Fail => panic!("failed to commit"),
                }
            }
        }
    }

    mod read_write {}

    #[tokio::test]
    async fn test() {
        // let db = DB::new("./tmp/data", Timestamp::new(10));
        // let txn_1 = db.begin_txn().await;
        // db.write("foo", 12, txn_1).await;
        // let did_commit = db.commit_txn(txn_1).await;

        // println!("Result is: {}", did_commit)
    }
}
