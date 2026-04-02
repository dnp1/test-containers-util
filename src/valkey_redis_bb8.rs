use std::ops::Deref;
use redis::Client;
use crate::Options;
use crate::redis_valkey_registry::{acquire_db, release_db};
use bb8::{ManageConnection, Pool};

/// A test Redis/Valkey pool bound to a specific database index.
///
/// Implements [`Deref`] to [`bb8::Pool<redis::Client>`] so it can be used
/// transparently wherever a pool is expected. When dropped, the database index
/// is returned to the shared registry so another test can acquire it.
pub struct RedisValkeyTestBB8 {
    pool: Pool<RedisConnectionManager>,
    db: u8,
    container_name: String,
}

impl RedisValkeyTestBB8 {
    pub fn get_pool(&self) -> Pool<RedisConnectionManager> {
        self.pool.clone()
    }
}

impl Deref for RedisValkeyTestBB8 {
    type Target = Pool<RedisConnectionManager>;

    fn deref(&self) -> &Self::Target {
        &self.pool
    }
}

impl Drop for RedisValkeyTestBB8 {
    fn drop(&mut self) {
        release_db(&self.container_name, self.db);
    }
}

/// A simple BB8 connection manager for Redis/Valkey Client.
pub struct RedisConnectionManager {
    client: Client,
}

impl RedisConnectionManager {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

impl ManageConnection for RedisConnectionManager {
    type Connection = redis::aio::MultiplexedConnection;
    type Error = redis::RedisError;

    async fn connect(&self) -> Result<Self::Connection, Self::Error> {
        self.client.get_multiplexed_async_connection().await
    }

    async fn is_valid(&self, conn: &mut Self::Connection) -> Result<(), Self::Error> {
        redis::cmd("PING").query_async(conn).await
    }

    fn has_broken(&self, _conn: &mut Self::Connection) -> bool {
        false
    }
}

/// Internal generic helper to acquire a pool.
async fn generic_pool(container_name: &str, base_url: &str) -> RedisValkeyTestBB8 {
    let db = acquire_db(container_name).await;
    let client = crate::redis_valkey_connection_manager::open_client(base_url, db);
    let manager = RedisConnectionManager::new(client);
    let pool = Pool::builder()
        .min_idle(1)
        .max_size(4)
        .build(manager)
        .await
        .expect("Failed to build pool");
    
    let mut conn = pool.get().await.expect("Failed to get connection");
    redis::cmd("FLUSHDB")
        .query_async::<()>(&mut *conn)
        .await
        .expect("Failed to flush database");
    drop(conn);

    RedisValkeyTestBB8 { 
        pool, 
        db, 
        container_name: container_name.to_string() 
    }
}

#[cfg(any(test, all(feature = "valkey", feature = "bb8")))]
pub async fn valkey_pool(container_name: &str, options: Option<Options>) -> RedisValkeyTestBB8 {
    let base_url = crate::valkey_container::valkey_url(container_name, options).await;
    generic_pool(container_name, &base_url).await
}

#[cfg(any(test, all(feature = "redis", feature = "bb8")))]
pub async fn redis_pool(container_name: &str, options: Option<Options>) -> RedisValkeyTestBB8 {
    let base_url = crate::redis_container::redis_url(container_name, options).await;
    generic_pool(container_name, &base_url).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_valkey_pool() {
        let name = "test-valkey-pool";
        let pool = valkey_pool(name, None).await;
        let mut conn = pool.get().await.unwrap();
        let _: () = redis::cmd("SET").arg("key").arg("val").query_async(&mut *conn).await.unwrap();
        let res: String = redis::cmd("GET").arg("key").query_async(&mut *conn).await.unwrap();
        assert_eq!(res, "val");
    }

    #[tokio::test]
    async fn test_redis_pool() {
        let name = "test-redis-pool";
        let pool = redis_pool(name, None).await;
        let mut conn = pool.get().await.unwrap();
        let _: () = redis::cmd("SET").arg("key").arg("val").query_async(&mut *conn).await.unwrap();
        let res: String = redis::cmd("GET").arg("key").query_async(&mut *conn).await.unwrap();
        assert_eq!(res, "val");
    }
}

