# MVCC

As we mentioned earlier, the MVCC system is the bedrock for how CRDB provides atomicity and isolation to transactions. In this section, we will talk about how the MVCC layer is implemented.

In an MVCC system, each record contains a key, value, and timestamp.

<img src="../images/mvcc.png" width="65%">

Similarly to CRDB, my toy database also uses RocksDB as the MVCC's storage engine. RocksDB is a key-value store, so we need to figure out a way to store `key`, `value,` and `timestamp` into a key-value pair. The toy database combines the `key` and the `timestamp` of the MVCC system into a single `MVCCKey` which will be encoded into a single RocksDB key.

```rust
pub struct MVCCKey {
    pub key: Key,
    pub timestamp: Timestamp,
}
```

Each `timestamp` in an `MVCCKey` is a HLC (hybrid logical clock) time:

```rust
pub struct Timestamp {
    pub wall_time: u64,
    pub logical_time: u32,
}
```

The Key’s type is

```rust
pub type Key = Vec<u8>;
```

To store the `MVCCKey` into `RocksDB`, we need to encode the `MVCCKey` into a RocksDB key, which is just a `Vec<u8>`. Inspired by CRDB's [EncodeMVCCKey method](https://github.com/cockroachdb/cockroach/blob/530100fd39cc722bc324bfb3869a325622258fb3/pkg/storage/mvcc_key.go#L161), the MVCCKey is encoded into the following form:

`[key] [wall_time] [logical_time]`

Here is the implementation:

```rust
pub fn encode_mvcc_key(mvcc_key: &MVCCKey) -> Vec<u8> {
    let mut key_vec = mvcc_key.key.to_vec();
    let timestamp_vec = encode_timestamp(mvcc_key.timestamp);
    key_vec.extend(timestamp_vec);
    key_vec
}

pub fn encode_timestamp(timestamp: Timestamp) -> Vec<u8> {
    let mut wall_time_bytes = timestamp.wall_time.to_le_bytes().to_vec();
    let logical_time_bytes = timestamp.logical_time.to_le_bytes().to_vec();
    wall_time_bytes.extend(logical_time_bytes);
    wall_time_bytes
}
```

Earlier, we mentioned that each key can only have one write intent at a time, so we need a unique key to represent an intent key. We use the MVCCKey with the zero timestamp: `Timestamp { wall_time: 0, logical_time: 0 }` to represent the key for a write intent.

When users query for a key, the database returns the latest version of that key. To make this faster, MVCCKeys are sorted from the highest timestamp to the lowest timestamp in the storage engine. However, we need the query to return the zero timestamp, which represents the intent key, if there is one.

Luckily, RocksDB allows developers to customize the order of keys in the table through [set_comparator](https://docs.rs/rocksdb/latest/rocksdb/struct.Options.html#method.set_comparator). [Here](https://github.com/brianshih1/little-key-value-db/blob/194d3f9e65bb69d674f0217f2a02b18ace12ee7e/src/storage/storage.rs#L50) is my implementation of the custom comparator. The comparator gives the highest precedence to the zero timestamp for the same key. Otherwise, it uses the key and the timestamp to order the MVCC keys.

## Core APIs

The core API of the MVCC layer includes:

- MVCC_SCAN
- MVCC_GET
- MVCC_PUT

These are the three APIs that the other entities of the transactional layer interact with the MVCC layer.

### MVCC_SCAN

`mvcc_scan(&self,  start_key: Key, end_key: Key, timestamp: Timestamp, scan_params: MVCCScanParams) -> MVCCScanResult`

[MVCC_SCAN](https://github.com/brianshih1/little-key-value-db/blob/194d3f9e65bb69d674f0217f2a02b18ace12ee7e/src/storage/mvcc.rs#L99) takes a start key, an end key, and a timestamp as inputs and returns an array of results that contains the keys in the range [start_key, end_key). Only keys with a timestamp less than or equal to the input timestamp are added to the scan results. MVCC_SCAN also collects any write intents that it found along the way.

**Algorithm**

- It first creates an [MVCCIterator](https://github.com/brianshih1/little-key-value-db/blob/194d3f9e65bb69d674f0217f2a02b18ace12ee7e/src/storage/mvcc_iterator.rs#L24), which is a wrapper around RocksDB’s iterator
- It then creates an [MVCCScanner](https://github.com/brianshih1/little-key-value-db/blob/194d3f9e65bb69d674f0217f2a02b18ace12ee7e/src/storage/mvcc.rs#L107) with the iterator and performs the [scan](https://github.com/brianshih1/little-key-value-db/blob/194d3f9e65bb69d674f0217f2a02b18ace12ee7e/src/storage/mvcc.rs#L115)
- The idea behind the scan is to first a[dvance the iterator to the intent key of the start key](https://github.com/brianshih1/little-key-value-db/blob/194d3f9e65bb69d674f0217f2a02b18ace12ee7e/src/storage/mvcc_scanner.rs#L65). Write intents have higher precedence than other MVCC keys, so it’s guaranteed that the iterator didn’t skip any keys that may be in the output
- it then keeps [advancing the iterator](https://github.com/brianshih1/little-key-value-db/blob/194d3f9e65bb69d674f0217f2a02b18ace12ee7e/src/storage/mvcc_scanner.rs#L88) with a [loop](https://github.com/brianshih1/little-key-value-db/blob/194d3f9e65bb69d674f0217f2a02b18ace12ee7e/src/storage/mvcc_scanner.rs#L66) until the iterator’s current key [exceeds the end key](https://github.com/brianshih1/little-key-value-db/blob/194d3f9e65bb69d674f0217f2a02b18ace12ee7e/src/storage/mvcc_scanner.rs#L76)
- during each iteration, the scanner checks if the record’s key is an intent key. [If it is,](https://github.com/brianshih1/little-key-value-db/blob/194d3f9e65bb69d674f0217f2a02b18ace12ee7e/src/storage/mvcc_scanner.rs#L107) add it to found_intents. Otherwise, if the MVCC key’s timestamp is ≤ the input timestamp, [add it to the results](https://github.com/brianshih1/little-key-value-db/blob/194d3f9e65bb69d674f0217f2a02b18ace12ee7e/src/storage/mvcc_scanner.rs#L145) and advance to the next key. However, if the key’s timestamp is greater than the input timestamp, it must [seek an older version of the key](https://github.com/brianshih1/little-key-value-db/blob/194d3f9e65bb69d674f0217f2a02b18ace12ee7e/src/storage/mvcc_scanner.rs#L150)

**CockroachDB’s MVCCScan**

For reference, here is CockroachDB’s implementation of [MVCCScan](https://github.com/cockroachdb/cockroach/blob/530100fd39cc722bc324bfb3869a325622258fb3/pkg/storage/mvcc.go#L3995). The core idea is similar. It initializes a [mvccScanner](https://github.com/cockroachdb/cockroach/blob/530100fd39cc722bc324bfb3869a325622258fb3/pkg/storage/mvcc.go#LL3739C31-L3739C42), [seeks to the start of the scan](https://github.com/cockroachdb/cockroach/blob/530100fd39cc722bc324bfb3869a325622258fb3/pkg/storage/pebble_mvcc_scanner.go#L655), and [keeps looping](https://github.com/cockroachdb/cockroach/blob/530100fd39cc722bc324bfb3869a325622258fb3/pkg/storage/pebble_mvcc_scanner.go#L658) until it exceeds the range. 

On each iteration, it checks if the key is an intent key [by checking if it’s empty](https://github.com/cockroachdb/cockroach/blob/c21c90f93219b857858518d25a8bc061444d573c/pkg/storage/pebble_mvcc_scanner.go#L723). It then checks if the MVCC key’s timestamp is [equal to the input timestamp](https://github.com/cockroachdb/cockroach/blob/c21c90f93219b857858518d25a8bc061444d573c/pkg/storage/pebble_mvcc_scanner.go#L745), [greater than the input timestamp](https://github.com/cockroachdb/cockroach/blob/c21c90f93219b857858518d25a8bc061444d573c/pkg/storage/pebble_mvcc_scanner.go#L786), or less than the input timestamp, in that case, it [seeks an older version of the key](https://github.com/cockroachdb/cockroach/blob/c21c90f93219b857858518d25a8bc061444d573c/pkg/storage/pebble_mvcc_scanner.go#L829).

### MVCC_GET

`fn mvcc_get<'a>(&self, key: &'a Key, timestamp: Timestamp, params: MVCCGetParams) -> MVCCGetResult`

[MVCC_GET](https://github.com/brianshih1/little-key-value-db/blob/194d3f9e65bb69d674f0217f2a02b18ace12ee7e/src/storage/mvcc.rs#L63) takes a key, a timestamp, and an optional transaction as inputs and returns the most recent value for the specified key whose timestamp is less than or equal to the supplied timestamp. If it runs into an uncommitted value, it returns a WriteIntentError.

**Algorithm**

- MVCC_GET is implemented as an MVCC_SCAN where a single key is retrieved. This is achieved by [setting both the start key and the end key to the same key](https://github.com/brianshih1/little-key-value-db/blob/194d3f9e65bb69d674f0217f2a02b18ace12ee7e/src/storage/mvcc.rs#L78).

**CockroachDB’s MVCCGet**

For reference, [here](https://github.com/brianshih1/little-key-value-db/blob/194d3f9e65bb69d674f0217f2a02b18ace12ee7e/src/storage/mvcc.rs#L63) is CockroachDB’s implementation of MVCCGet. The idea to use MVCCScanner is inspired by them. In their implementation, they implement mvcc_get a[s a scan with start_key = end_key](https://github.com/brianshih1/little-key-value-db/blob/194d3f9e65bb69d674f0217f2a02b18ace12ee7e/src/storage/mvcc.rs#L73).

### MVCC_PUT

`fn mvcc_put<T: Serialize>(&self, key: Key, timestamp: Option<Timestamp>, txn: Option<TxnLink>, value: T) -> Result<MVCCKey, WriteIntentError>`

[MVCC_PUT](https://github.com/brianshih1/little-key-value-db/blob/194d3f9e65bb69d674f0217f2a02b18ace12ee7e/src/storage/mvcc.rs#L122) takes a key, a timestamp, and an optional transaction and tries to insert a timestamped record into the MVCC database. If a transaction is provided, a write intent is placed in the database. Otherwise, the raw value is placed into the database.

Before writing, MVCC_PUT must verify that there is no uncommitted intent for the same key. This is because there can only be a write intent for a key. If an uncommitted intent already exists for the same key, an error is returned. 

**Algorithm:**

- first, it tries to [fetch the intent record](https://github.com/brianshih1/little-key-value-db/blob/194d3f9e65bb69d674f0217f2a02b18ace12ee7e/src/storage/mvcc.rs#L151)
- if an intent is found, there are 2 scenarios
  - [the intent’s transaction is the same transaction as the current transaction](https://github.com/brianshih1/little-key-value-db/blob/194d3f9e65bb69d674f0217f2a02b18ace12ee7e/src/storage/mvcc.rs#L158). In this case, we’re overwriting our own transaction which is acceptable - we don’t do anything
  - Otherwise, we check the status of the transaction record that corresponds to the write intent. If the transaction is pending, [a WriteIntentError is returned](https://github.com/brianshih1/little-key-value-db/blob/194d3f9e65bb69d674f0217f2a02b18ace12ee7e/src/storage/mvcc.rs#L162)

- After verifying that there is no uncommitted intent for the same key, we are free to insert it into the database. 

- If a transaction is provided, [an uncommitted intent is placed into the database](https://github.com/brianshih1/little-key-value-db/blob/194d3f9e65bb69d674f0217f2a02b18ace12ee7e/src/storage/mvcc.rs#L193). An UncommittedValue contains the value, the transaction ID, and the write_timestamp of the transaction at the time of insertion.

- Otherwise, [the raw value is placed into the database](https://github.com/brianshih1/little-key-value-db/blob/194d3f9e65bb69d674f0217f2a02b18ace12ee7e/src/storage/mvcc.rs#L206).

**CockroachDB’s MVCCPut**

For reference, here is CockroachDB’s implementation of [MVCCPut](https://github.com/cockroachdb/cockroach/blob/530100fd39cc722bc324bfb3869a325622258fb3/pkg/storage/mvcc.go#L1442). The core idea is the same. It [checks if an intent was found](https://github.com/cockroachdb/cockroach/blob/530100fd39cc722bc324bfb3869a325622258fb3/pkg/storage/mvcc.go#L1859). If there exists an uncommitted write intent whose transaction ID is not the same as the MVCCPut’s Transaction ID, [a WriteIntentError is returned](https://github.com/cockroachdb/cockroach/blob/530100fd39cc722bc324bfb3869a325622258fb3/pkg/storage/mvcc.go#L1863). 

Otherwise, if the intent’s timestamp is less than the write timestamp, clear [it](https://github.com/cockroachdb/cockroach/blob/530100fd39cc722bc324bfb3869a325622258fb3/pkg/storage/mvcc.go#L2024) so that MVCCPut can overwrite it. Finally, MVCCPut [writes the mvcc value](https://github.com/cockroachdb/cockroach/blob/530100fd39cc722bc324bfb3869a325622258fb3/pkg/storage/mvcc.go#L2195).  

