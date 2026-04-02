use crate::rand::rand_str;
use crate::{Options, postgres_container};
use diesel::{prelude::*, sql_query};
use diesel_async::{
    AsyncConnection, AsyncMigrationHarness, AsyncPgConnection, RunQueryDsl,
    pooled_connection::{AsyncDieselConnectionManager, bb8::Pool},
};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::runtime::Handle;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::spawn_blocking;
use url::Url;

/// Max concurrent live `PostgresTestDb` instances per container.
/// Each instance holds a bb8 pool of up to 2 connections, so this caps
/// total connections at `MAX_CONCURRENT_DBS * 2` per container.
const MAX_CONCURRENT_DBS: usize = 30;
const MAX_POOL_SIZE: usize = 3;
const MIN_POOL_SIZE: usize = 1;

static CONTAINER_SEMAPHORES: OnceLock<Mutex<HashMap<String, Arc<Semaphore>>>> = OnceLock::new();

fn semaphore_for(container_name: &str) -> Arc<Semaphore> {
    let map = CONTAINER_SEMAPHORES.get_or_init(|| Mutex::new(HashMap::new()));
    map.lock()
        .unwrap()
        .entry(container_name.to_string())
        .or_insert_with(|| Arc::new(Semaphore::new(MAX_CONCURRENT_DBS)))
        .clone()
}

async fn run_migrations(
    mut conn: AsyncPgConnection,
    migrations: EmbeddedMigrations,
    schema_name: Option<&str>,
) {
    if let Some(schema) = schema_name {
        sql_query(format!("CREATE SCHEMA IF NOT EXISTS \"{}\";", schema))
            .execute(&mut conn)
            .await
            .unwrap();
        sql_query(format!("SET search_path to \"{}\",public;", schema))
            .execute(&mut conn)
            .await
            .unwrap();
    }

    spawn_blocking(|| {
        AsyncMigrationHarness::new(conn)
            .run_pending_migrations(migrations)
            .unwrap();
    })
    .await
    .expect("Error running migrations");
}

/// An isolated PostgreSQL test database backed by Diesel async.
///
/// Each instance creates a uniquely-named database inside the shared container,
/// runs your migrations, and provides a bb8 connection pool scoped to that
/// database. When the value is dropped the database is deleted automatically.
///
/// Up to [`MAX_CONCURRENT_DBS`] instances can be alive simultaneously per
/// named container (semaphore-guarded), keeping total connections manageable.
///
/// # Example
///
/// ```rust,ignore
/// use diesel_migrations::{embed_migrations, EmbeddedMigrations};
/// use test_containers_util::diesel_pg::PostgresTestDb;
///
/// const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./migrations");
///
/// #[tokio::test(flavor = "multi_thread")]
/// async fn my_test() {
///     let db = PostgresTestDb::create("my-pg", MIGRATIONS, None, None).await;
///     let pool = db.pool();
///     // … use pool …
/// } // `db` dropped → database deleted
/// ```
pub struct PostgresTestDb {
    default_db_dsn: String,
    tests_dsn: String,
    pool: Pool<AsyncPgConnection>,
    name: String,
    _permit: OwnedSemaphorePermit,
}

impl PostgresTestDb {
    /// Create an isolated test database inside the shared container.
    ///
    /// - `container_name` – Docker container name; the container is started and
    ///   cached on the first call with this name.
    /// - `migrations` – embedded migrations to run against the new database.
    /// - `schema_name` – if `Some`, the schema is created and set as the
    ///   default `search_path` before running migrations.
    /// - `options` – override the Docker image tag or container command; `None`
    ///   uses the defaults from [`postgres_container::default_cmd`].
    pub async fn create(
        container_name: &str,
        migrations: EmbeddedMigrations,
        schema_name: Option<&str>,
        options: Option<Options>,
    ) -> Self {
        let _permit = semaphore_for(container_name)
            .acquire_owned()
            .await
            .expect("semaphore closed");

        let name = format!("test_{}", rand_str(16));
        let dsn = postgres_container::get_postgres_dsn(container_name, options).await;
        let mut conn = AsyncPgConnection::establish(&dsn).await.unwrap();

        sql_query(format!("CREATE DATABASE \"{database}\";", database = name))
            .execute(&mut conn)
            .await
            .unwrap();

        let mut url = Url::parse(&dsn).unwrap();
        url.set_path(&name);
        let tests_dsn = url.to_string();
        println!("Test database url {url}", url = tests_dsn);

        // let mut t_conn = AsyncPgConnection::establish(&tests_dsn).await.unwrap();
        // run_migrations(&mut t_conn, migrations, schema_name).await;

        let config = AsyncDieselConnectionManager::<AsyncPgConnection>::new(&tests_dsn);
        let pool = Pool::builder()
            .min_idle(MIN_POOL_SIZE as u32)
            .max_size(MAX_POOL_SIZE as u32)
            .build(config)
            .await
            .unwrap();

        {
            let conn = AsyncPgConnection::establish(&tests_dsn).await.unwrap();
            run_migrations(conn, migrations, schema_name).await;
        }

        Self {
            default_db_dsn: dsn,
            tests_dsn,
            pool,
            name,
            _permit,
        }
    }

    /// Returns a cloned handle to the bb8 connection pool for this test database.
    pub fn pool(&self) -> Pool<AsyncPgConnection> {
        self.pool.clone()
    }

    /// Returns the DSN (connection string) for this test database.
    pub fn dsn(&self) -> &str {
        &self.tests_dsn
    }
}
impl Drop for PostgresTestDb {
    fn drop(&mut self) {
        let default_db_dsn = self.default_db_dsn.clone();
        let name = self.name.to_owned();

        let cleanup = async move {
            let mut conn = AsyncPgConnection::establish(&default_db_dsn)
                .await
                .unwrap();

            sql_query(format!(
                "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '{}'",
                name
            ))
            .execute(&mut conn)
            .await
            .unwrap();

            sql_query(format!("DROP DATABASE \"{}\"", name))
                .execute(&mut conn)
                .await
                .unwrap();
        };

        if let Ok(handle) = Handle::try_current() {
            handle.spawn(cleanup);
        } else {
            tokio::runtime::Runtime::new().unwrap().block_on(cleanup);
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use diesel_migrations::embed_migrations;

    pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./migrations");

    #[tokio::test(flavor = "multi_thread")]
    async fn test_postgres_test_db_creation_and_dropping() {
        let container_name = "test-postgres-diesel";
        let db = PostgresTestDb::create(container_name, MIGRATIONS, None, None).await;
        let dsn = db.dsn().to_string();

        {
            let mut conn = AsyncPgConnection::establish(&dsn).await.expect("Should connect to test db");
            let result = sql_query("SELECT 1").execute(&mut conn).await;
            assert!(result.is_ok());

            let result = sql_query("SELECT * FROM test").execute(&mut conn).await;
            assert!(result.is_ok(), "Table 'test' should exist after migrations");
        }

        // Test the pool as well
        {
            let pool = db.pool();
            let mut conn = pool.get().await.expect("Should get connection from pool");
            let result = sql_query("SELECT 1").execute(&mut conn).await;
            assert!(result.is_ok());
        }

        // Drop the db instance to trigger database deletion
        drop(db);

        // Give it a moment to run the cleanup task
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // After dropping, connecting to the test db should fail
        let result = AsyncPgConnection::establish(&dsn).await;
        assert!(result.is_err(), "Should not be able to connect to dropped database");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_postgres_test_db_with_schema() {
        let container_name = "test-postgres-diesel-schema";
        let schema_name = "custom_schema";
        let db = PostgresTestDb::create(container_name, MIGRATIONS, Some(schema_name), None).await;
        let dsn = db.dsn().to_string();

        {
            let mut conn = AsyncPgConnection::establish(&dsn).await.expect("Should connect to test db");
            
            // Check if table 'test' exists in the custom schema
            let query = format!(
                "SELECT table_name FROM information_schema.tables WHERE table_schema = '{}' AND table_name = 'test'",
                schema_name
            );
            let result = sql_query(query).execute(&mut conn).await;
            assert!(result.is_ok());
        }

        drop(db);
    }
}
