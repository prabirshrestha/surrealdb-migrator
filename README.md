# SurrealDB Migrator

Migrator library for [SurrealDB](https://surrealdb.com).

# Example

```bash
cargo add surrealdb-migrator
```

```rust
use surrealdb_migrator::{Migrations, M};

let db = surrealdb::engine::any::connect("...connection string...");

let migrations = Migrations::new(vec![
    M::up("DEFINE TABLE user; DEFINE FIELD username ON user TYPE string;"),
    M::up("DEFINE FIELD password ON user TYPE string;"),
]);

// Go to the latest version
migrations.to_latest(&db).unwrap();
```

# LICENSE

Apache License

# Acknowledgments

Thanks to [rusqlite_migration](https://github.com/cljoly/rusqlite_migration) where the code for surrealdb-migrator is inspired from.
