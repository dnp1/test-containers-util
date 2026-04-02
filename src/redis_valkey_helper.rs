use redis::aio::ConnectionManager;
use redis::Client;
use crate::valkey_container::{valkey_url, ValkeyTestBB8, ValkeyTestConnManager};


/// A test Valkey pool bound to a specific database index.
///
/// Implements [`Deref`] to [`bb8::Pool<redis::Client>`] so it can be used
/// transparently wherever a pool is expected. When dropped, the database index
/// is returned to the shared registry so another test can acquire it.
pub struct ValkeyTestBB8 {
    pool: bb8::Pool<Client>,
    db: u8,
}

impl ValkeyTestBB8 {
    pub fn get_pool(&self) -> bb8::Pool<redis::Client> {
        self.pool.clone()
    }
}

impl Deref for ValkeyTestBB8 {
    type Target = bb8::Pool<redis::Client>;

    fn deref(&self) -> &Self::Target {
        &self.pool
    }
}

impl Drop for ValkeyTestBB8 {
    fn drop(&mut self) {
        DB_REGISTRY.available.lock().unwrap().push_back(self.db);
        DB_REGISTRY.notify.notify_one();
    }
}

/// A test Valkey connection manager bound to a specific database index.
///
/// Implements [`Deref`] to [`ConnectionManager`] so it can be used
/// transparently wherever a manager is expected. When dropped, the database
/// index is returned to the shared registry so another test can acquire it.
pub struct ValkeyTestConnManager {
    manager: ConnectionManager,
    db: u8,
}

impl ValkeyTestConnManager {
    pub fn get_manager(&self) -> ConnectionManager {
        self.manager.clone()
    }
}

impl Deref for ValkeyTestConnManager {
    type Target = ConnectionManager;

    fn deref(&self) -> &Self::Target {
        &self.manager
    }
}

impl Drop for ValkeyTestConnManager {
    fn drop(&mut self) {
        DB_REGISTRY.available.lock().unwrap().push_back(self.db);
        DB_REGISTRY.notify.notify_one();
    }
}

fn open_client(base_url: &str, db: u8) -> Client {
    let url = format!("{base_url}/{db}");
    redis::Client::open(url).expect("Invalid Valkey URL")
}

/// Acquire a pool backed by an isolated Valkey database (0–15).
///
/// # Cross-binary isolation warning
///
/// The DB registry (0–15) isolates concurrent tests **within a single test
/// binary** via [`crate::valkey_container::DB_REGISTRY`]. It does NOT isolate across binaries: if two
/// test binaries run simultaneously against the same container and one calls
/// `FLUSHDB` on a DB index that the other currently holds, tests will corrupt
/// each other.
///
/// This is safe today because `cargo test` runs each crate's test binary
/// sequentially by default. If you ever enable `--test-threads` across crates
/// or run binaries in parallel (e.g. `cargo nextest` with default settings),
/// revisit this — the fix would be to partition the 16 DB indices per binary
/// (e.g. via an env var) or switch to per-test key namespacing.
///
/// If all 16 databases are currently in use, this function waits until one is
/// released. The selected database is flushed before the pool is returned.
/// Dropping the returned [`ValkeyTestBB8`] releases the database back to the
/// registry.
pub async fn valkey_pool() -> ValkeyTestBB8 {
    let db = crate::valkey_container::acquire_db().await;
    let client = open_client(&valkey_url().await, db);
    let pool = bb8::Pool::builder()
        .min_idle(2)
        .max_size(5)
        .build(client)
        .await
        .expect("Failed to build Valkey pool");
    let mut conn = pool.get().await.expect("Failed to get Valkey connection");
    redis::cmd("FLUSHDB")
        .query_async::<()>(&mut *conn)
        .await
        .expect("Failed to flush Valkey database");
    drop(conn);
    ValkeyTestBB8 { pool, db }
}

/// Acquire a connection manager backed by an isolated Valkey database (0–15).
///
/// If all 16 databases are currently in use, this function waits until one is
/// released. The selected database is flushed before the manager is returned.
/// Dropping the returned [`ValkeyTestConnManager`] releases the database back to
/// the registry.
pub async fn valkey_conn_manager() -> ValkeyTestConnManager {
    let db = crate::valkey_container::acquire_db().await;
    let client = open_client(&valkey_url().await, db);
    let mut manager = ConnectionManager::new(client)
        .await
        .expect("Failed to build ConnectionManager");
    redis::cmd("FLUSHDB")
        .query_async::<()>(&mut manager)
        .await
        .expect("Failed to flush Valkey database");
    ValkeyTestConnManager { manager, db }
}