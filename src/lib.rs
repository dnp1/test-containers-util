//! # test-containers-util
//!
//! Ergonomic, reusable [Testcontainers](https://github.com/testcontainers/testcontainers-rs)
//! helpers for integration tests in Rust.
//!
//! Each helper starts the Docker container **once per named instance** and reuses
//! it across all tests in the same process. Database isolation is achieved
//! automatically:
//!
//! - **PostgreSQL** – a randomly-named database is created per test and deleted on drop.
//! - **Redis / Valkey** – one of 16 logical database indices is acquired per test
//!   and released on drop.
//!
//! ## Features
//!
//! Enable only the features you need in your `Cargo.toml`:
//!
//! | Feature     | Description |
//! |-------------|-------------|
//! | `postgres`  | Bare PostgreSQL container (DSN only) |
//! | `pg-diesel` | PostgreSQL + Diesel async ORM with migrations and bb8 pool |
//! | `pg-sqlx`   | PostgreSQL + SQLx async driver with migrations |
//! | `bb8`       | BB8 connection pool (combine with `valkey` or `redis`) |
//! | `valkey`    | Valkey container with connection manager / bb8 pool |
//! | `redis`     | Redis container with connection manager / bb8 pool |
//! | `moto`      | AWS Moto mock server (S3, SNS, SQS, …) |
//!
//! ## Quick start
//!
//! ```toml
//! [dev-dependencies]
//! test-containers-util = { version = "0.1", features = ["pg-diesel"] }
//! ```
//!
//! ```rust,ignore
//! use diesel_migrations::{embed_migrations, EmbeddedMigrations};
//! use test_containers_util::diesel_pg::PostgresTestDb;
//!
//! const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./migrations");
//!
//! #[tokio::test(flavor = "multi_thread")]
//! async fn my_test() {
//!     let db = PostgresTestDb::create("my-container", MIGRATIONS, None, None).await;
//!     let pool = db.pool();
//!     // … use pool …
//! }
//! ```

#[cfg(any(test, feature = "pg-diesel"))]
pub mod diesel_pg;

#[cfg(any(test, feature = "moto"))]
pub mod moto_container;
#[cfg(any(test, feature = "postgres", feature = "pg-diesel", feature = "pg-sqlx"))]
pub mod postgres_container;


#[cfg(any(test, feature = "moto", feature = "redis"))]
mod redis_container;
#[cfg(any(test, feature = "valkey", feature = "redis"))]
pub mod redis_valkey_connection_manager;

#[cfg(any(test, feature = "valkey", feature = "redis"))]
mod redis_valkey_registry;
#[cfg(any(test, feature = "valkey"))]
pub mod valkey_container;

#[cfg(any(
    test,
    all(feature = "valkey", feature = "bb8"),
    all(feature = "redis", feature = "bb8")
))]
pub mod valkey_redis_bb8;


#[cfg(any(test, feature = "pg-sqlx"))]
pub mod sqlx_pg;

#[cfg(any(test, feature = "pg-diesel", feature = "pg-sqlx"))]
mod rand;
mod options;
pub use options::Options;

// mod valkey_pool;
