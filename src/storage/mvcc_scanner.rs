use crate::{db::db::TxnLink, hlc::timestamp::Timestamp};

use super::{
    mvcc_iterator::MVCCIterator,
    mvcc_key::{create_intent_key, MVCCKey},
    txn::{TxnIntent, UncommittedValue},
    Key, Value,
};

pub struct MVCCScanner<'a> {
    pub it: MVCCIterator<'a>,

    pub txn: Option<TxnLink>,

    // TODO: lockTable

    // start of scan (doesn't contain MVCC timestamp)
    pub start_key: Key,

    // end of the scan (doesn't contain MVCC timestamp)
    pub end_key: Option<Key>,

    // Timestamp that MVCCScan/MVCCGet was called
    pub timestamp: Timestamp,

    pub found_intents: Vec<(TxnIntent, Value)>,

    // max number of tuples to add to the results
    pub max_records_count: usize,

    /**
     * CockroachDB stores it as: <valueLen:Uint32><keyLen:Uint32><Key><Value>
     * https://github.com/cockroachdb/cockroach/blob/c21c90f93219b857858518d25a8bc061444d573c/pkg/storage/pebble_mvcc_scanner.go#L148
     *
     * For now, I'll just store a vec of KV tuples (unoptimized version for the MVP)
     */
    pub results: Vec<(MVCCKey, Value)>,
    // TODO: failOnMoreRecent if we want to allow things like locked scans. But not for now.
}

impl<'a> MVCCScanner<'a> {
    pub fn new(
        it: MVCCIterator<'a>,
        start_key: Key,
        end_key: Option<Key>,
        timestamp: Timestamp,
        max_records_count: usize,
        transaction: Option<TxnLink>,
    ) -> Self {
        MVCCScanner {
            it,
            start_key: start_key,
            end_key: end_key,
            timestamp,
            found_intents: Vec::new(),
            results: Vec::new(),
            max_records_count,
            txn: transaction,
        }
    }

    pub fn scan(&mut self) -> () {
        // intent key will always be sorted before other MVCC keys
        let start_base = create_intent_key(&self.start_key);
        self.it.seek_ge(&start_base);
        loop {
            if self.results.len() == self.max_records_count {
                return;
            }
            if !self.it.valid() {
                println!("invalid!");
                return;
            }
            match &self.end_key {
                Some(end_key) => {
                    if &self.it.current_key().key > end_key {
                        return;
                    }
                }
                None => {
                    // if there is no end_key, then the end_key defaults to start_key
                    if self.it.current_key().key > self.start_key {
                        return;
                    }
                }
            }
            self.get_current_key();
            self.advance_to_next_key();
            // advance to next one
        }
    }

    /**
     * Gets the most recent key that is less than the read_timestamp and adds it to the result set.
     *
     * If there is a transaction and it notices an intent, it adds the intent.
     *
     * Attempts to add the current key to the result set. If it notices an intent,
     * it adds the intent. This function is not responsible for checking the start and end key or advances.
     * It just tries to add the current key to the result set.
     *
     * Returns whether a record was added to the result set for the current key
     *
     */
    pub fn get_current_key(&mut self) -> bool {
        let current_key = self.it.current_key();
        if current_key.is_intent_key() {
            let current_value = self.it.current_value_serialized::<UncommittedValue>();

            if let Some(scanner_transaction) = &self.txn {
                if current_value.txn_metadata.txn_id == scanner_transaction.read().unwrap().txn_id {
                    // TODO: Resolve based on epoch
                    self.found_intents.push((
                        TxnIntent {
                            txn_meta: current_value.txn_metadata,
                            key: current_key.key.clone(),
                        },
                        current_value.value,
                    ));
                } else {
                    self.found_intents.push((
                        TxnIntent {
                            txn_meta: current_value.txn_metadata,
                            key: current_key.key.clone(),
                        },
                        current_value.value,
                    ));
                }
            } else {
                self.found_intents.push((
                    TxnIntent {
                        txn_meta: current_value.txn_metadata,
                        key: current_key.key.clone(),
                    },
                    current_value.value,
                ));
            }

            return false;
        } else {
            let key_timestamp = current_key.timestamp;

            if self.timestamp > key_timestamp {
                // the scanner's timestamp is greater, so just add
                self.results
                    .push((self.it.current_key(), self.it.current_value()));
                return true;
            } else if self.timestamp < key_timestamp {
                // seek to older version
                return self.seek_older_version(current_key.key.to_owned(), self.timestamp);
            } else {
                // the scanner's timestamp is sufficient (equal), so just add
                self.results
                    .push((self.it.current_key(), self.it.current_value()));
                return true;
            }
        }
    }

    /**
     * Try to add the key <= the provided timestamp and add it to the result set.
     * Return true if added.
     */
    fn seek_older_version(&mut self, key: Key, timestamp: Timestamp) -> bool {
        let mvcc_key = MVCCKey {
            key: key.to_owned(),
            timestamp,
        };
        let is_valid = self.it.seek_ge(&mvcc_key);
        if is_valid {
            let current_key = self.it.current_key();
            if current_key.key == key {
                self.results.push((current_key, self.it.current_value()));
                return true;
            } else {
                return false;
            }
        } else {
            return false;
        }
    }

    pub fn advance_to_next_key(&mut self) -> () {
        if !self.it.valid() {
            return;
        }
        let current_key = self.it.current_key();

        loop {
            self.it.next();
            if !self.it.valid() {
                return;
            }
            let next_key = self.it.current_key();
            if current_key.key != next_key.key {
                break;
            } else {
                continue;
            }
        }
    }
}
