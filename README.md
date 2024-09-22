# SurrealDB Migrator

Migrator library for [SurrealDB](https://surrealdb.com).

# Example

```bash
cargo add surrealdb-migrator
```

## Using code for migration scripts

```rust
use surrealdb_migrator::{Migrations, M};

let db = surrealdb::engine::any::connect("mem://").unwrap();

db.use_ns("sample").use_db("sample").await.unwrap();

let migrations = Migrations::new(vec![
    M::up("DEFINE TABLE animal; DEFINE FIELD name ON animal TYPE string;").down("REMOVE TABLE user;"),
    M::up("DEFINE TABLE food; DEFINE FIELD name ON food TYPE string;").down("REMOVE TABLE food;"),
]);

// Go to the latest version
migrations.to_latest(&db).unwrap();

// Go to a specific version
migrations.to_version(&db, 0).unwrap();
```

## Using files for migration script

The migrations are loaded and stored in the binary. `from-directory` feature flags needs to be enabled.

The migration directory pointed to by `include_dir!()` must contain
subdirectories in accordance with the given pattern:
`{usize id indicating the order}-{convenient migration name}`

Those directories must contain at least an `up.surql` file containing a valid upward
migration. They can also contain a `down.surql` file containing a downward migration.

Example structure

```
migrations
├── 01-friend_car
│  └── up.surql
├── 02-add_birthday_column
│  └── up.surql
└── 03-add_animal_table
    ├── down.surql
    └── up.surql
```

```rust
use include_dir::{include_dir, Dir}; // cargo add include_dir
static MIGRATION_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/migrations");

let migrations = Migrations::from_directory(&MIGRATION_DIR).unwrap();
migrations.to_latest(&db).await?;
```

# LICENSE

Apache License

# Acknowledgments

Thanks to [rusqlite_migration](https://github.com/cljoly/rusqlite_migration) where the code for surrealdb-migrator is inspired from.
