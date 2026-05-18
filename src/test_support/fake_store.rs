//! `FakeStore` — alias for an in-memory `SqliteStore`. The real store is
//! already fast enough in `:memory:` mode (~50µs/op) and using it means
//! tests exercise the same SQL the production code runs.

use crate::store::SqliteStore;

pub type FakeStore = SqliteStore;

/// Convenience constructor for tests: `let s = make_fake_store();`.
pub fn make_fake_store() -> SqliteStore {
    SqliteStore::open_in_memory().expect("in-memory SqliteStore open should never fail")
}
