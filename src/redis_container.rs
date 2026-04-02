use crate::Options;
use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex};
use testcontainers::{ContainerAsync, ReuseDirective};
use testcontainers_modules::{
    redis::{REDIS_PORT, Redis},
    testcontainers::{ImageExt, runners::AsyncRunner},
};
use tokio::sync::{Notify, OnceCell};

static REDIS_CONTAINERS: LazyLock<
    Mutex<HashMap<String, &'static OnceCell<ContainerAsync<Redis>>>>,
> = LazyLock::new(|| Mutex::new(HashMap::new()));
/// Returns a Redis URL including the database index.
///
/// Note: This acquires a database index from the registry.
pub async fn redis_url(container_name: &str, options: Option<Options>) -> String {
    let options = options.unwrap_or(Options {
        tag: "7-alpine".to_string(),
        cmd: Vec::new(),
    });
    let container = get_container(container_name, &options).await;
    let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
    format!("redis://127.0.0.1:{port}")
}

/// Returns the cached container instance for the given name, starting it if necessary.
async fn get_container(container_name: &str, options: &Options) -> &'static ContainerAsync<Redis> {
    let cell = {
        let mut map = REDIS_CONTAINERS.lock().unwrap();
        *map.entry(container_name.to_string())
            .or_insert_with(|| Box::leak(Box::new(OnceCell::new())))
    };

    cell.get_or_init(|| async {
        let mut builder = Redis::default()
            .with_tag(options.tag.clone())
            .with_container_name(container_name.to_string())
            .with_reuse(ReuseDirective::Always);
        if options.cmd.len() > 0 {
            builder = builder.with_cmd(&options.cmd);
        }
        builder.start()
            .await
            .expect("Failed to start Redis")
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options() -> Options {
        Options {
            tag: "7-alpine".to_string(),
            cmd: Vec::new(),
        }
    }

    #[tokio::test]
    async fn test_redis_container_reuses_instance_for_same_name() {
        let options = options();
        let name = "test-redis-reuse";
        let c1 = get_container(name, &options).await;
        let c2 = get_container(name, &options).await;
        assert_eq!(c1.id(), c2.id());
        c1.stop().await.unwrap();
        c2.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_redis_container_different_names_different_instances() {
        let options = options();
        let name1 = "test-redis-diff-1";
        let name2 = "test-redis-diff-2";
        let c1 = get_container(name1, &options).await;
        let c2 = get_container(name2, &options).await;
        assert_ne!(c1.id(), c2.id());
        c1.stop().await.unwrap();
        c2.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_redis_connection_get_set() {
        use redis::AsyncCommands;
        let name = "test-redis-conn-direct";
        let base_url = redis_url(name, None).await;

        // Extract DB from URL to release it later (hacky but shows it works)
        let url = format!("{base_url}/0");

        let client = redis::Client::open(url).unwrap();
        let mut conn = client.get_multiplexed_async_connection().await.unwrap();

        let key = "test_key";
        let val = "test_value";

        let _: () = conn.set::<&str, &str, ()>(key, val).await.unwrap();
        let res: String = conn.get::<&str, String>(key).await.unwrap();

        assert_eq!(res, val);
    }
}
