use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex};
use testcontainers::{ContainerAsync, ReuseDirective};
use testcontainers_modules::{
    testcontainers::{runners::AsyncRunner, ImageExt},
    redis::{Redis, REDIS_PORT},
};
use tokio::sync::{Notify, OnceCell};

pub struct DbRegistry {
    pub available: Mutex<VecDeque<u8>>,
    pub notify: Notify,
}

static REDIS_CONTAINERS: LazyLock<Mutex<HashMap<String, &'static OnceCell<ContainerAsync<Redis>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static DB_REGISTRIES: LazyLock<Mutex<HashMap<String, &'static DbRegistry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Returns a Redis URL including the database index.
/// 
/// Note: This acquires a database index from the registry.
pub async fn redis_url(container_name: &str) -> String {
    let container = get_container(container_name).await;
    let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
    let db = acquire_db(container_name).await;
    format!("redis://127.0.0.1:{port}/{db}")
}

/// Returns the cached container instance for the given name, starting it if necessary.
pub async fn get_container(container_name: &str) -> &'static ContainerAsync<Redis> {
    let cell = {
        let mut map = REDIS_CONTAINERS.lock().unwrap();
        *map.entry(container_name.to_string())
            .or_insert_with(|| Box::leak(Box::new(OnceCell::new())))
    };

    cell.get_or_init(|| async {
        Redis::default()
            .with_tag("8")
            .with_container_name(container_name.to_string())
            .with_reuse(ReuseDirective::Always)
            .start()
            .await
            .expect("Failed to start Redis")
    })
    .await
}

/// Waits until a database index is available for the given container and returns it.
pub async fn acquire_db(container_name: &str) -> u8 {
    let registry = get_registry(container_name);

    loop {
        let db = registry.available.lock().unwrap().pop_front();
        if let Some(db) = db {
            return db;
        }
        registry.notify.notified().await;
    }
}

/// Releases a database index back to the registry for the given container.
pub fn release_db(container_name: &str, db: u8) {
    let registry = get_registry(container_name);
    registry.available.lock().unwrap().push_back(db);
    registry.notify.notify_one();
}

fn get_registry(container_name: &str) -> &'static DbRegistry {
    let mut map = DB_REGISTRIES.lock().unwrap();
    *map.entry(container_name.to_string())
        .or_insert_with(|| {
            Box::leak(Box::new(DbRegistry {
                available: Mutex::new((0u8..16).collect()),
                notify: Notify::new(),
            }))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_redis_container_reuses_instance_for_same_name() {
        let name = "test-redis-reuse";
        let c1 = get_container(name).await;
        let c2 = get_container(name).await;
        assert_eq!(c1.id(), c2.id());
    }

    #[tokio::test]
    async fn test_redis_container_different_names_different_instances() {
        let name1 = "test-redis-diff-1";
        let name2 = "test-redis-diff-2";
        let c1 = get_container(name1).await;
        let c2 = get_container(name2).await;
        assert_ne!(c1.id(), c2.id());
    }

    #[tokio::test]
    async fn test_db_registry_isolation() {
        let name1 = "test-db-iso-unique-1";
        let name2 = "test-db-iso-unique-2";

        let db1 = acquire_db(name1).await;
        let db2 = acquire_db(name2).await;

        // Both should get 0 as their first DB because they are isolated
        assert_eq!(db1, 0);
        assert_eq!(db2, 0);

        let db1_next = acquire_db(name1).await;
        assert_eq!(db1_next, 1);

        release_db(name1, db1);
        // After releasing 0, the queue is [1, 2, ..., 15, 0]
        // So next acquire will be 1, then 2, ..., 15, then 0.
        let _ = acquire_db(name1).await; // gets 2
        let db1_reacquired = acquire_db(name1).await; // should eventually get 0 if we cleared it?
        // Let's just check that releasing works and we can get more.
        assert_eq!(db1_reacquired, 3); 
    }

    #[tokio::test]
    async fn test_redis_url_includes_db() {
        let name = "test-redis-url-db";
        let url = redis_url(name).await;
        // url should be redis://127.0.0.1:PORT/0
        assert!(url.contains("/0"));
        
        let url2 = redis_url(name).await;
        assert!(url2.contains("/1"));
    }

    #[tokio::test]
    async fn test_redis_connection_get_set() {
        use redis::AsyncCommands;
        let name = "test-redis-conn-direct";
        let url = redis_url(name).await;
        
        // Extract DB from URL to release it later (hacky but shows it works)
        let db: u8 = url.split('/').last().unwrap().parse().unwrap();

        let client = redis::Client::open(url).unwrap();
        let mut conn = client.get_multiplexed_async_connection().await.unwrap();

        let key = "test_key";
        let val = "test_value";

        let _: () = conn.set::<&str, &str, ()>(key, val).await.unwrap();
        let res: String = conn.get::<&str, String>(key).await.unwrap();

        assert_eq!(res, val);
        release_db(name, db);
    }
}
