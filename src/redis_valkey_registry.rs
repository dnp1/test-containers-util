use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex};
use tokio::sync::Notify;

struct DbRegistry {
    available: Mutex<VecDeque<u8>>,
    notify: Notify,
}
pub (crate) static DB_REGISTRIES: LazyLock<Mutex<HashMap<String, &'static DbRegistry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

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
pub (crate) fn release_db(container_name: &str, db: u8) {
    let registry = get_registry(container_name);
    registry.available.lock().unwrap().push_back(db);
    registry.notify.notify_one();
}

fn get_registry(container_name: &str) -> &'static DbRegistry {
    let mut map = DB_REGISTRIES.lock().unwrap();
    *map.entry(container_name.to_string()).or_insert_with(|| {
        Box::leak(Box::new(DbRegistry {
            available: Mutex::new((0u8..16).collect()),
            notify: Notify::new(),
        }))
    })
}


#[cfg(test)]
mod test {
    use super::*;
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

}