# SurrealDB Migrator

Migrator library for [SurrealDB](https://surrealdb.com).

# Example

```bash
cargo add surrealdb-migrator
```

```rust
use surrealdb_migrator::{Migrations, M};

let db = surrealdb::engine::any::connect("mem://");

let migrations = Migrations::new(vec![
    M::up("DEFINE TABLE animal; DEFINE FIELD name ON animal TYPE string;").down("REMOVE TABLE user;"),
    M::up("DEFINE TABLE food; DEFINE FIELD name ON food TYPE string;").down("REMOVE TABLE food;"),
]);

// Go to the latest version
migrations.to_latest(&db).unwrap();

// Go to a specific version
migrations.to_version(&db, 0).unwrap();
```

# LICENSE

Apache License

# Acknowledgments

Thanks to [rusqlite_migration](https://github.com/cljoly/rusqlite_migration) where the code for surrealdb-migrator is inspired from.
