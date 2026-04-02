use crate::options::Options;
use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex};
use testcontainers::{ContainerAsync, ReuseDirective};
use testcontainers_modules::{
    testcontainers::{ImageExt, runners::AsyncRunner},
    valkey::{VALKEY_PORT, Valkey},
};
use tokio::sync::{Notify, OnceCell};

static VALKEY_CONTAINERS: LazyLock<
    Mutex<HashMap<String, &'static OnceCell<ContainerAsync<Valkey>>>>,
> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Returns a Valkey URL including the database index.
///
/// Note: This acquires a database index from the registry.
pub async fn valkey_url(container_name: &str, options: Option<Options>) -> String {
    let options = options.unwrap_or(Options {
        tag: "8.1-alpine".to_string(),
        cmd: Vec::default(),
    });
    let container = get_container(container_name, &options).await;
    let port = container.get_host_port_ipv4(VALKEY_PORT).await.unwrap();
    format!("valkey://127.0.0.1:{port}")
}

/// Returns the cached container instance for the given name, starting it if necessary.
async fn get_container(container_name: &str, options: &Options) -> &'static ContainerAsync<Valkey> {
    let cell = {
        let mut map = VALKEY_CONTAINERS.lock().unwrap();
        *map.entry(container_name.to_string())
            .or_insert_with(|| Box::leak(Box::new(OnceCell::new())))
    };

    cell.get_or_init(|| async {
        let mut builder = Valkey::default()
            .with_tag("8.1-alpine")
            .with_container_name(container_name.to_string())
            .with_reuse(ReuseDirective::Always);
        if options.cmd.len() > 0 {
            builder = builder.with_cmd(options.cmd.clone());
        }

        builder.start().await.expect("Failed to start Valkey")
    })
    .await
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncWriteExt;
    use super::*;

    fn options() -> Options {
        Options {
            tag: "8.1-alpine".to_string(),
            cmd: Vec::default(),
        }
    }

    #[tokio::test]
    async fn test_valkey_container_reuses_instance_for_same_name() {
        let options = options();
        let name = "test-valkey-reuse";
        let mut c1 = get_container(name, &options).await;
        let mut c2 = get_container(name, &options).await;
        assert_eq!(c1.id(), c2.id());
        c1.stop().await.unwrap();
        c2.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_valkey_container_different_names_different_instances() {
        let options = options();
        let name1 = "test-valkey-diff-1";
        let name2 = "test-valkey-diff-2";
        let mut c1 = get_container(name1, &options).await;
        let mut c2 = get_container(name2, &options).await;
        assert_ne!(c1.id(), c2.id());
        c1.stop().await.unwrap();
        c2.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_valkey_connection_get_set() {
        let options = options();
        use redis::AsyncCommands;
        let name = "test-valkey-conn-direct";
        let base_url = valkey_url(name, None).await;

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
