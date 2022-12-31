use rocksdb::DBIterator;

use crate::hlc::timestamp::Timestamp;

use super::Key;

pub struct RocksMVCCScanner<'a> {
    mvcc_iterator: DBIterator<'a>,

    // TODO: lockTable

    // start of scan (doesn't contain MVCC timestamp)
    pub start: Key,

    // end of the scan (doesn't contain MVCC timestamp)
    pub end: Key,

    // Timestamp that MVCCScan/MVCCGet was called
    pub ts: Timestamp,
    // TODO: Results
}

impl<'a> RocksMVCCScanner<'a> {
    // seeks to the start key and adds one KV to the result set
    pub fn get(&mut self) {
        // self.mvcc_iterator.seek_ge(MVCCKey {});
    }

    pub fn seek_ge(&mut self) {}
}
