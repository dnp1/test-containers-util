use std::ops::Deref;
use crate::Options;
use redis::Client;
use redis::aio::ConnectionManager;
use crate::redis_valkey_registry::{acquire_db, release_db};

/// A test Redis/Valkey connection manager bound to a specific database index.
///
/// Implements [`Deref`] to [`ConnectionManager`] so it can be used
/// transparently wherever a manager is expected. When dropped, the database
/// index is returned to the shared registry so another test can acquire it.
pub struct RedisValkeyTestConnManager {
    manager: ConnectionManager,
    db: u8,
    container_name: String,
}

impl RedisValkeyTestConnManager {
    pub fn get_manager(&self) -> ConnectionManager {
        self.manager.clone()
    }
}

impl Deref for RedisValkeyTestConnManager {
    type Target = ConnectionManager;

    fn deref(&self) -> &Self::Target {
        &self.manager
    }
}

impl Drop for RedisValkeyTestConnManager {
    fn drop(&mut self) {
        release_db(&self.container_name, self.db);
    }
}

pub fn open_client(base_url: &str, db: u8) -> Client {
    let url = format!("{base_url}/{db}");
    Client::open(url).expect("Invalid Redis/Valkey URL")
}

/// Internal generic helper to acquire a connection manager.
async fn generic_conn_manager(
    container_name: &str,
    base_url: &str,
) -> RedisValkeyTestConnManager {
    let db = acquire_db(container_name).await;
    let client = open_client(base_url, db);
    let mut manager = ConnectionManager::new(client)
        .await
        .expect("Failed to build ConnectionManager");
    redis::cmd("FLUSHDB")
        .query_async::<()>(&mut manager)
        .await
        .expect("Failed to flush database");
    RedisValkeyTestConnManager { 
        manager, 
        db, 
        container_name: container_name.to_string() 
    }
}

#[cfg(any(test, feature = "valkey"))]
pub async fn valkey_conn_manager(
    container_name: &str,
    options: Option<Options>,
) -> RedisValkeyTestConnManager {
    let base_url = crate::valkey_container::valkey_url(container_name, options).await;
    generic_conn_manager(container_name, &base_url).await
}

#[cfg(any(test, feature = "redis"))]
pub async fn redis_conn_manager(
    container_name: &str,
    options: Option<Options>,
) -> RedisValkeyTestConnManager {
    let base_url = crate::redis_container::redis_url(container_name, options).await;
    generic_conn_manager(container_name, &base_url).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_valkey_conn_manager() {
        let name = "test-valkey-mgr";
        let mgr = valkey_conn_manager(name, None).await;
        let mut conn = mgr.get_manager();
        let _: () = redis::cmd("SET").arg("key").arg("val").query_async(&mut conn).await.unwrap();
        let res: String = redis::cmd("GET").arg("key").query_async(&mut conn).await.unwrap();
        assert_eq!(res, "val");
    }

    #[tokio::test]
    async fn test_redis_conn_manager() {
        let name = "test-redis-mgr";
        let mgr = redis_conn_manager(name, None).await;
        let mut conn = mgr.get_manager();
        let _: () = redis::cmd("SET").arg("key").arg("val").query_async(&mut conn).await.unwrap();
        let res: String = redis::cmd("GET").arg("key").query_async(&mut conn).await.unwrap();
        assert_eq!(res, "val");
    }

    #[tokio::test]
    async fn test_db_release() {
        let name = "test-db-release";
        {
            let _mgr = valkey_conn_manager(name, None).await;
            // Acquired 0, Registry now has [1, 2, ..., 15]
        }
        // 0 released, Registry now has [1, 2, ..., 15, 0]
        let mgr2 = valkey_conn_manager(name, None).await;
        assert_eq!(mgr2.db, 1);
    }
}

