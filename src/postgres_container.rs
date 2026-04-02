use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use testcontainers::ContainerAsync;
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{ImageExt, ReuseDirective, core::IntoContainerPort, runners::AsyncRunner},
};
use tokio::sync::OnceCell;

static POSTGRES_DSN_BY_CONTAINER: LazyLock<Mutex<HashMap<String, &'static OnceCell<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

const PG_PORT: u16 = 5432;

pub struct Options {
    pub tag: String,
    pub cmd: Vec<String>,
}

impl Options {
    pub fn with_tag(tag: &str) -> Self {
        Self {
            tag: tag.to_string(),
            cmd: default_cmd().into_iter().map(|x| String::from(x)).collect(),
        }
    }
}

pub fn default_cmd() -> Vec<&'static str> {
    vec![
        "-c",
        "fsync=off",
        "-c",
        "synchronous_commit=off",
        "-c",
        "shared_buffers=256MB",
        "-c",
        "work_mem=4MB",
        "-c",
        "maintenance_work_mem=128MB",
        "-c",
        "effective_cache_size=512MB",
        "-c",
        "max_connections=40",
    ]
}

/// Returns the DSN for the shared test Postgres container.
///
/// On the first call within a process the function checks whether a container
/// named as container_name input
pub async fn get_postgres_dsn(container_name: &str, options: Option<Options>) -> String {
    let cell = {
        let mut map = POSTGRES_DSN_BY_CONTAINER.lock().unwrap();
        *map.entry(container_name.to_string())
            .or_insert_with(|| Box::leak(Box::new(OnceCell::new())))
    };

    let options = options.unwrap_or(Options::with_tag("18-alpine"));

    cell.get_or_init(|| async {
        let container = get_container(container_name, &options).await;
        let port = container.get_host_port_ipv4(PG_PORT).await.unwrap();
        format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port)
    })
    .await
    .clone()
}

async fn get_container(container_name: &str, opt: &Options) -> ContainerAsync<Postgres> {
    Postgres::default()
        .with_tag(&opt.tag)
        .with_container_name(container_name)
        .with_cmd(opt.cmd.clone())
        .with_shm_size(600 * 1024 * 1024) // 600 MiB
        .with_reuse(ReuseDirective::Always)
        .start()
        .await
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_container_doest_conflict_on_port_reuse_with_same_name() {
        let options = Options::with_tag("18-alpine");
        const NAME_1: &'static str = "random-name-PG-1";
        const NAME_2: &'static str = "random-name-PG-2";

        {
            // CLEANUP to ensure the new running code
            let c0 = get_container("random-name-PG-1", &options).await;
            c0.stop().await.expect("Should stop");
            let c0 = get_container("random-name-PG-2", &options).await;
            c0.stop().await.expect("Should stop");
        }

        let c1 = get_container(NAME_1, &options).await;
        let c2 = get_container(NAME_1, &options).await;
        assert_eq!(c1.id(), c2.id());
        let c3 = get_container(NAME_2, &options).await;
        assert_ne!(c1.id(), c3.id());
    }

    #[tokio::test]
    async fn test_simple_query_works() {
        let url = get_postgres_dsn("random-name-PG-simple-query", None).await;
        let (client, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .unwrap();
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        println!("secs: {}", secs);

        tokio::spawn(async move {
            if let Err(e) = conn.await {
                eprintln!("connection error: {}", e);
            }
        });

        let rows = client
            .query_one("SELECT $1::BIGINT", &[&(secs as i64)])
            .await
            .unwrap();

        assert_eq!(secs as i64, rows.get::<usize, i64>(0));
    }
}
