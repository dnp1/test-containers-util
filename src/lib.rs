#[cfg(any(test, feature = "pg-diesel"))]
mod diesel_pg;

#[cfg(any(test, feature = "moto"))]
mod moto_container;
#[cfg(any(test, feature = "postgres", feature = "pg-diesel"))]
mod postgres_container;

mod options;
#[cfg(any(test, feature = "moto", feature = "redis"))]
mod redis_container;
#[cfg(any(test, feature = "valkey", feature = "redis"))]
mod redis_valkey_connection_manager;

#[cfg(any(test, feature = "valkey", feature = "redis"))]
mod redis_valkey_registry;
#[cfg(any(test, feature = "valkey"))]
mod valkey_container;

#[cfg(any(
    test,
    all(feature = "valkey", feature = "bb8"),
    all(feature = "redis", feature = "bb8")
))]
mod valkey_redis_bb8;

#[cfg(any(test, feature = "pg-diesel"))]
mod rand;
mod sqlx_pg;

pub use options::Options;

// mod valkey_pool;
