use core::time;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use serde::{de::DeserializeOwned, Serialize};
use uuid::Uuid;

use crate::{
    execute::{
        executor::Executor,
        request::{
            BeginTxnRequest, CommitTxnRequest, GetRequest, GetResponse, PutRequest, Request,
            RequestMetadata, RequestUnion, ResponseUnion,
        },
    },
    hlc::timestamp::Timestamp as HLCTimestamp,
    storage::{str_to_key, txn::Txn},
};

pub type TxnLink = Arc<RwLock<Txn>>;

pub struct DB {
    executor: Executor,
    current_time: RwLock<Timestamp>,
    txns: RwLock<HashMap<Uuid, TxnLink>>,
}

#[derive(Debug, Clone, Copy)]
pub struct Timestamp {
    pub value: u64,
}

impl Timestamp {
    pub fn advance_by(&self, step: u64) -> Timestamp {
        Timestamp {
            value: self.value + step,
        }
    }

    pub fn to_hlc_timestamp(&self) -> HLCTimestamp {
        HLCTimestamp::new(self.value, self.value.try_into().unwrap())
    }
}

impl DB {
    // TODO: Should we have a new_cleaned and keep a new?
    // path example: "./tmp/data";
    pub fn new(path: &str) -> Self {
        DB {
            executor: Executor::new(path),
            current_time: RwLock::new(Timestamp { value: 10 }),
            txns: RwLock::new(HashMap::new()),
        }
    }

    pub fn set_time(&self, timestamp: Timestamp) {
        *self.current_time.write().unwrap() = timestamp;
    }

    pub fn now(&self) -> Timestamp {
        *self.current_time.read().unwrap()
    }

    // TODO: Return potential error
    pub async fn write<T: Serialize>(&self, key: &str, value: T, txn_id: Uuid) {
        let request_union = RequestUnion::Put(PutRequest {
            key: str_to_key(key),
            value: serde_json::to_string(&value).unwrap().into_bytes(),
        });
        let txn = self.get_txn(txn_id);
        let request_metadata = RequestMetadata { txn };
        let write_request = Request {
            metadata: request_metadata,
            request_union,
        };
        let response = self
            .executor
            .execute_request_with_concurrency_retries(write_request)
            .await;
        match response {
            ResponseUnion::Put(_) => {}
            _ => unreachable!(),
        };
    }

    // TODO: Result
    pub async fn read<T: DeserializeOwned>(&self, key: &str, txn_id: Uuid) -> T {
        let request_union = RequestUnion::Get(GetRequest {
            key: str_to_key(key),
        });
        let txn = self.get_txn(txn_id);
        let request_metadata = RequestMetadata { txn };
        let read_request = Request {
            metadata: request_metadata,
            request_union,
        };
        let response = self
            .executor
            .execute_request_with_concurrency_retries(read_request)
            .await;
        let (_, value) = match response {
            ResponseUnion::Get(r) => r.value,
            _ => unreachable!(),
        };

        serde_json::from_slice::<T>(&value).unwrap()
    }

    pub async fn read_without_txn<T: DeserializeOwned>(
        &self,
        key: &str,
        timestamp: Timestamp,
    ) -> T {
        todo!()
    }

    pub async fn begin_txn_with_timestamp(&self, timestamp: Timestamp) -> Uuid {
        let (txn_id, txn) = self.create_txn_internal(timestamp);
        let request_metadata = RequestMetadata { txn };
        let txn_request = RequestUnion::BeginTxn(BeginTxnRequest { txn_id });
        let request = Request {
            metadata: request_metadata,
            request_union: txn_request,
        };
        let response = self
            .executor
            .execute_request_with_concurrency_retries(request)
            .await;
        match response {
            ResponseUnion::BeginTransaction(_) => {}
            _ => unreachable!(),
        };
        txn_id
    }

    pub async fn begin_txn(&self) -> Uuid {
        self.begin_txn_with_timestamp(self.now()).await
    }

    pub async fn abort_txn(&self) {}

    pub async fn commit_txn(&self, txn_id: Uuid) {
        let txn = self.get_txn(txn_id);
        let request_metadata = RequestMetadata { txn };
        let txn_request = RequestUnion::CommitTxn(CommitTxnRequest {});
        let request = Request {
            metadata: request_metadata,
            request_union: txn_request,
        };
        let response = self
            .executor
            .execute_request_with_concurrency_retries(request)
            .await;
        match response {
            ResponseUnion::CommitTxn(_) => {}
            _ => unreachable!(),
        };
    }

    fn create_txn_internal(&self, timestamp: Timestamp) -> (Uuid, TxnLink) {
        let txn_id = Uuid::new_v4();
        let txn = Txn::new_link(txn_id, timestamp.to_hlc_timestamp());
        let mut txns = self.txns.write().unwrap();
        txns.insert(txn_id, txn.clone());
        (txn_id, txn)
    }

    fn get_txn(&self, txn_id: Uuid) -> TxnLink {
        let txns = self.txns.read().unwrap();
        let txn = txns.get(&txn_id);
        match txn {
            Some(txn_link) => txn_link.clone(),
            None => {
                panic!("No txn found for {}", txn_id)
            }
        }
    }
}
