# test-containers-util

Ergonomic, reusable [Testcontainers](https://github.com/testcontainers/testcontainers-rs) helpers for integration tests in Rust.

Each helper starts the Docker container **once per named instance** and reuses it across all tests in the process. Database isolation (for Postgres) is achieved by creating a fresh database per test and dropping it on `Drop`. Redis/Valkey isolation uses one of 16 available database indices, returned to the pool when the guard is dropped.

## Requirements

- Docker (or a compatible runtime) accessible on the host
- Tokio async runtime (`flavor = "multi_thread"` recommended)
- Testcontainers reusable-containers support requires the `TESTCONTAINERS_RYUK_DISABLED=true` environment variable **or** a Ryuk container running

## Features

| Feature     | Description |
|-------------|-------------|
| `postgres`  | Bare PostgreSQL container (DSN only) |
| `pg-diesel` | PostgreSQL + Diesel async ORM with migrations and bb8 pool |
| `pg-sqlx`   | PostgreSQL + SQLx async driver with migrations |
| `bb8`       | BB8 connection pool (used together with `valkey` or `redis`) |
| `valkey`    | Valkey container with connection manager / bb8 pool |
| `redis`     | Redis container with connection manager / bb8 pool |
| `moto`      | AWS Moto mock server for S3, SNS, SQS, etc. |

## Installation

Add the crate as a **dev-dependency** and enable only the features you need:

```toml
[dev-dependencies]
test-containers-util = { version = "0.1", features = ["pg-diesel"] }

# or pick multiple features
test-containers-util = { version = "0.1", features = ["pg-sqlx", "valkey", "bb8", "moto"] }
```

## Usage

### PostgreSQL + Diesel

```rust
use diesel_migrations::{embed_migrations, EmbeddedMigrations};
use test_containers_util::diesel_pg::PostgresTestDb;

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./migrations");

#[tokio::test(flavor = "multi_thread")]
async fn my_diesel_test() {
    let db = PostgresTestDb::create("my-pg-container", MIGRATIONS, None, None).await;

    let pool = db.pool(); // bb8 Pool<AsyncPgConnection>
    let mut conn = pool.get().await.unwrap();

    diesel::sql_query("SELECT 1").execute(&mut conn).await.unwrap();

    // `db` dropped here → test database is automatically deleted
}
```

To use a custom PostgreSQL schema:

```rust
let db = PostgresTestDb::create("my-pg-container", MIGRATIONS, Some("my_schema"), None).await;
```

### PostgreSQL + SQLx

```rust
use test_containers_util::sqlx_pg::PostgresTestDb;

static MIGRATIONS: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[tokio::test(flavor = "multi_thread")]
async fn my_sqlx_test() {
    let db = PostgresTestDb::create("my-pg-container", &MIGRATIONS, None, None).await;

    let pool = db.pool(); // sqlx::PgPool
    sqlx::query("SELECT 1").execute(&pool).await.unwrap();

    // `db` dropped here → test database is automatically deleted
}
```

### Redis — Connection Manager

```rust
use test_containers_util::redis_valkey_connection_manager::redis_conn_manager;

#[tokio::test]
async fn my_redis_test() {
    let mgr = redis_conn_manager("my-redis-container", None).await;
    let mut conn = mgr.get_manager();

    redis::cmd("SET").arg("key").arg("value").query_async::<()>(&mut conn).await.unwrap();
    let val: String = redis::cmd("GET").arg("key").query_async(&mut conn).await.unwrap();
    assert_eq!(val, "value");

    // `mgr` dropped here → database index returned to shared pool
}
```

### Valkey — BB8 Pool

```rust
use test_containers_util::valkey_redis_bb8::valkey_pool;

#[tokio::test]
async fn my_valkey_test() {
    let pool = valkey_pool("my-valkey-container", None).await;
    let mut conn = pool.get().await.unwrap();

    redis::cmd("SET").arg("k").arg("v").query_async::<()>(&mut *conn).await.unwrap();
    let val: String = redis::cmd("GET").arg("k").query_async(&mut *conn).await.unwrap();
    assert_eq!(val, "v");
}
```

### AWS Moto (S3 / SNS / SQS / …)

```rust
use test_containers_util::moto_container::get_aws_config;
use aws_sdk_sns::Client as SnsClient;

#[tokio::test]
async fn my_aws_test() {
    let config = get_aws_config("my-moto-container").await;
    let sns = SnsClient::new(&config);

    sns.create_topic().name("my-topic").send().await.unwrap();
}
```

### Custom Docker image tag or command

Pass an `Options` value to override the image tag or container command:

```rust
use test_containers_util::Options;

let opts = Options {
    tag: "16-alpine".to_string(),
    cmd: vec!["-c".to_string(), "max_connections=200".to_string()],
};

let db = PostgresTestDb::create("my-pg-container", MIGRATIONS, None, Some(opts)).await;
```

## How it works

### Container reuse

Containers are keyed by name. The first test to call a helper starts the container; every subsequent call in the same process receives the already-running instance. Testcontainers' `ReuseDirective::Always` keeps the container alive across process restarts as well (requires Ryuk to be disabled).

### Database isolation (PostgreSQL)

Each `PostgresTestDb` creates a randomly-named database (e.g. `test_a3f7bc…`) inside the shared container, runs your migrations, and hands you a connection pool scoped to that database. On drop, all connections are terminated and the database is deleted.

Up to **30 concurrent** `PostgresTestDb` instances are allowed per named container (semaphore-guarded) to avoid exhausting Postgres connection limits.

### Database isolation (Redis / Valkey)

Redis supports up to **16 logical databases** (indices 0–15). Each `RedisValkeyTestConnManager` / `RedisValkeyTestBB8` acquires a free index from a per-container registry and flushes it with `FLUSHDB`. On drop the index is released back to the registry.

### Cleanup on drop

All cleanup (DROP DATABASE, FLUSHDB + index release) happens in `impl Drop`. Postgres cleanup is spawned as a Tokio task on the current runtime handle (or on a temporary runtime if called outside a Tokio context).

## License

MIT OR Apache-2.0
