use crate::rand::rand_str;
use crate::{Options, postgres_container};
use sqlx::postgres::{PgPoolOptions, PgPool};
use sqlx::{Connection, migrate::Migrator};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::runtime::Handle;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use url::Url;

/// Max concurrent live `PostgresTestDb` instances per container.
/// Each instance holds a sqlx pool of up to 3 connections, so this caps
/// total connections at `MAX_CONCURRENT_DBS * 3` per container.
const MAX_CONCURRENT_DBS: usize = 30;
const MAX_POOL_SIZE: u32 = 3;
const MIN_POOL_SIZE: u32 = 1;

static CONTAINER_SEMAPHORES: OnceLock<Mutex<HashMap<String, Arc<Semaphore>>>> = OnceLock::new();

fn semaphore_for(container_name: &str) -> Arc<Semaphore> {
    let map = CONTAINER_SEMAPHORES.get_or_init(|| Mutex::new(HashMap::new()));
    map.lock()
        .unwrap()
        .entry(container_name.to_string())
        .or_insert_with(|| Arc::new(Semaphore::new(MAX_CONCURRENT_DBS)))
        .clone()
}

pub struct PostgresTestDb {
    default_db_dsn: String,
    tests_dsn: String,
    pool: PgPool,
    name: String,
    _permit: OwnedSemaphorePermit,
}

impl PostgresTestDb {
    pub async fn create(
        container_name: &str,
        migrator: &Migrator,
        schema_name: Option<&str>,
        options: Option<Options>,
    ) -> Self {
        let _permit = semaphore_for(container_name)
            .acquire_owned()
            .await
            .expect("semaphore closed");

        let name = format!("test_{}", rand_str(16));
        let dsn = postgres_container::get_postgres_dsn(container_name, options).await;
        
        {
            let mut conn = sqlx::PgConnection::connect(&dsn).await.unwrap();
            sqlx::query(&format!("CREATE DATABASE \"{database}\";", database = name))
                .execute(&mut conn)
                .await
                .unwrap();
        }

        let mut url = Url::parse(&dsn).unwrap();
        url.set_path(&name);
        let tests_dsn = url.to_string();
        println!("Test database url {url}", url = tests_dsn);

        let mut pool_options = PgPoolOptions::new()
            .min_connections(MIN_POOL_SIZE)
            .max_connections(MAX_POOL_SIZE);

        if let Some(schema) = schema_name {
            let schema = schema.to_string();
            pool_options = pool_options.after_connect(move |conn, _meta| {
                let schema = schema.clone();
                Box::pin(async move {
                    sqlx::query(&format!("SET search_path TO \"{}\", public", schema))
                        .execute(conn)
                        .await?;
                    Ok(())
                })
            });
        }

        let pool = pool_options.connect(&tests_dsn).await.unwrap();

        if let Some(schema) = schema_name {
            sqlx::query(&format!("CREATE SCHEMA IF NOT EXISTS \"{}\";", schema))
                .execute(&pool)
                .await
                .unwrap();
        }

        migrator.run(&pool).await.expect("Error running migrations");

        Self {
            default_db_dsn: dsn,
            tests_dsn,
            pool,
            name,
            _permit,
        }
    }

    pub fn pool(&self) -> PgPool {
        self.pool.clone()
    }

    pub fn dsn(&self) -> &str {
        &self.tests_dsn
    }
}

impl Drop for PostgresTestDb {
    fn drop(&mut self) {
        let default_db_dsn = self.default_db_dsn.clone();
        let name = self.name.to_owned();

        let cleanup = async move {
            let mut conn = sqlx::PgConnection::connect(&default_db_dsn)
                .await
                .unwrap();

            sqlx::query(&format!(
                "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '{}'",
                name
            ))
            .execute(&mut conn)
            .await
            .unwrap();

            sqlx::query(&format!("DROP DATABASE \"{}\"", name))
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

    pub static MIGRATIONS: Migrator = sqlx::migrate!("./migrations");

    #[tokio::test(flavor = "multi_thread")]
    async fn test_postgres_test_db_creation_and_dropping() {
        let container_name = "test-postgres-sqlx";
        let db = PostgresTestDb::create(container_name, &MIGRATIONS, None, None).await;
        let dsn = db.dsn().to_string();

        {
            let pool = db.pool();
            let result = sqlx::query("SELECT 1").execute(&pool).await;
            assert!(result.is_ok());

            let result = sqlx::query("SELECT * FROM test").execute(&pool).await;
            assert!(result.is_ok(), "Table 'test' should exist after migrations");
        }

        // Drop the db instance to trigger database deletion
        drop(db);

        // Give it a moment to run the cleanup task
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // After dropping, connecting to the test db should fail
        let result = sqlx::PgConnection::connect(&dsn).await;
        assert!(result.is_err(), "Should not be able to connect to dropped database");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_postgres_test_db_with_schema() {
        let container_name = "test-postgres-sqlx-schema";
        let schema_name = "custom_schema";
        let db = PostgresTestDb::create(container_name, &MIGRATIONS, Some(schema_name), None).await;
        
        {
            let pool = db.pool();
            
            // Check if table 'test' exists in the custom schema
            let row: (String,) = sqlx::query_as(
                "SELECT table_name FROM information_schema.tables WHERE table_schema = $1 AND table_name = 'test'"
            )
            .bind(schema_name)
            .fetch_one(&pool)
            .await
            .unwrap();
            
            assert_eq!(row.0, "test");
        }

        drop(db);
    }
}
