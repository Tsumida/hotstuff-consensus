//! Persistent storage based on Sqlite.

/// Final table name = table_prefix + hotstuff_token
const PREFIX_LIVENESS_TABLE: &str = "liveness_table_";
const PREFIX_SAFETY_TABLE: &str = "safety_table_";
const PREFIX_STATE_TABLE: &str = "state_table_";

pub struct SqlitePersistor {}
