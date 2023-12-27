use std::{cmp::Ordering, num::NonZeroUsize};

use surrealdb::{Connection, Surreal};
use tracing::{debug, info, trace, warn};

/// A typedef of the result returned by many methods.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Enum listing possible errors.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Surreldb error")]
    /// Surrealdb error, query may indicate the attempted SQL query
    SurrealdbError {
        /// SQL query that caused the error
        query: String,
        /// Error returned by surrealdb
        err: surrealdb::Error,
    },
    #[error("Migration definition error")]
    /// Something wrong with migration definitions
    MigrationDefinition(MigrationDefinitionError),
    #[error("Unrecognized error")]
    Unrecognized(Box<dyn std::error::Error + Send + Sync + 'static>),
}

impl From<surrealdb::Error> for Error {
    fn from(e: surrealdb::Error) -> Error {
        Error::SurrealdbError {
            query: String::new(),
            err: e,
        }
    }
}

/// Errors related to schema versions
#[derive(thiserror::Error, Debug)]
pub enum MigrationDefinitionError {
    #[error("Down not defined error")]
    /// Migration has no down version
    DownNotDefined {
        /// Index of the migration that caused the error
        migration_index: usize,
    },
    #[error("No migration defined")]
    /// Attempt to migrate when no migrations are defined
    NoMigrationsDefined,
    #[error("Database too far ahead")]
    /// Attempt to migrate when the database is currently at a higher migration level
    DatabaseTooFarAhead,
}

/// One migration.
#[derive(Debug)]
pub struct M<'a> {
    up: &'a str,
    comment: Option<&'a str>,
}

impl<'a> M<'a> {
    /// Create a schema update. The SQL command will be executed only when the migration has not been
    /// executed on the underlying database.
    ///
    /// # Example
    ///
    /// ```no_test
    /// use surrealdb_migration::M;
    ///
    /// M::up("DEFINE TABLE user; DEFINE FIELD username ON user TYPE string;");
    /// ```
    pub fn up(sql: &'a str) -> Self {
        Self {
            up: sql,
            comment: None,
        }
    }

    /// Add a comment to the schema update
    pub const fn comment(mut self, comment: &'a str) -> Self {
        self.comment = Some(comment);
        self
    }

    /// Generate a sha256 checksum based on the up sql
    pub fn checksum(&self) -> String {
        sha256::digest(self.up)
    }
}

/// Set of migrations
#[derive(Debug)]
pub struct Migrations<'a> {
    ms: Vec<M<'a>>,
}

impl<'a> Migrations<'a> {
    /// Create a set of migrations.
    ///
    /// # Example
    ///
    /// ```no_test
    /// use surrealdb_migration::{Migrations, M};
    ///
    /// let migrations = Migrations::new(vec![
    ///     M::up("DEFINE TABLE user; DEFINE FIELD username ON user TYPE string;"),
    ///     M::up("DEFINE FIELD password ON user TYPE string;"),
    /// ]);
    /// ```
    #[must_use]
    pub fn new(ms: Vec<M<'a>>) -> Self {
        Migrations { ms }
    }

    /// Migrate the database to latest schema version. The migrations are applied atomically.
    ///
    /// # Example
    ///
    /// ```no_test
    /// use surrealdb_migration::{Migrations, M};
    ///
    /// let db = surrealdb::engine::any::connect("file://data.db");
    ///
    /// let migrations = Migrations::new(vec![
    ///     M::up("DEFINE TABLE user; DEFINE FIELD username ON user TYPE string;"),
    ///     M::up("DEFINE FIELD password ON user TYPE string;"),
    /// ]);
    ///
    /// // Go to the latest version
    /// migrations.to_latest(&db).unwrap();
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::MigrationDefinition`] if no migration is defined.
    pub async fn to_latest<C: Connection>(&self, db: &Surreal<C>) -> Result<()> {
        self.ensure_migrations_table(db).await?;
        let v_max = self.max_schema_version();
        match v_max {
            SchemaVersion::NoneSet => {
                warn!("No migration defined");
                Err(Error::MigrationDefinition(
                    MigrationDefinitionError::NoMigrationsDefined,
                ))
            }
            SchemaVersion::Inside(v) => {
                info!("some migrations defined (version: {v}), try to migrate");
                self.goto(db, v_max.into()).await
            }
            SchemaVersion::Outside(_) => unreachable!(),
        }
    }

    async fn ensure_migrations_table<C: Connection>(&self, db: &Surreal<C>) -> Result<()> {
        info!("Ensuring _migrations table");

        db.query(
            r#"
    DEFINE TABLE _migrations SCHEMAFULL;
    DEFINE FIELD version                    ON _migrations TYPE number;
    DEFINE FIELD comment                    ON _migrations TYPE string;
    DEFINE FIELD checksum                   ON _migrations TYPE string;
    DEFINE FIELD installed_on               ON _migrations TYPE datetime;
    DEFINE INDEX _migrations_version_idx    ON TABLE _migrations COLUMNS version UNIQUE;
            "#,
        )
        .await?
        .check()?;

        info!("_migrations table defined");

        Ok(())
    }

    /// Go to a given db version
    async fn goto<C: Connection>(&self, db: &Surreal<C>, target_db_version: usize) -> Result<()> {
        self.ensure_migrations_table(db).await?;
        let current_version = get_current_version(db).await?;

        let res = match target_db_version.cmp(&current_version) {
            Ordering::Less => {
                if current_version > self.ms.len() {
                    return Err(Error::MigrationDefinition(
                        MigrationDefinitionError::DatabaseTooFarAhead,
                    ));
                }
                info!(
                    "rollback to older version requested, target_db_version: {}, current_version: {}",
                    target_db_version, current_version
                );
                self.goto_down(db, current_version, target_db_version).await
            }
            Ordering::Equal => {
                info!("no migration to run, db already up to date");
                return Ok(()); // return directly, so the migration message is not printed
            }
            Ordering::Greater => {
                info!(
                    "some migrations to run, target: {target_db_version}, current: {current_version}"
                );
                self.goto_up(db, current_version, target_db_version).await
            }
        };

        if res.is_ok() {
            info!("Database migrated to version {}", target_db_version);
        }

        res
    }

    /// Migrate upward methods. This is rolled back on error.
    /// On success, returns the number of update performed
    /// All versions are db versions
    async fn goto_up<C: Connection>(
        &self,
        db: &Surreal<C>,
        current_version: usize,
        target_version: usize,
    ) -> Result<()> {
        debug_assert!(current_version <= target_version);
        debug_assert!(target_version <= self.ms.len());

        trace!("start migration");

        let mut queries = db.query("BEGIN;");

        for v in current_version..target_version {
            let m = &self.ms[v];
            info!("Running: v{} {}", v + 1, m.comment.unwrap_or_default());
            debug!("{}", m.up);

            queries = queries
                .query(m.up)
                .query(format!(
                    r#"
                INSERT INTO _migrations {{
                    version: $version_{v},
                    comment: $comment_{v},
                    checksum: $checksum_{v},
                    installed_on: time::now()
                }};
                "#,
                ))
                .bind((format!("version_{v}"), v + 1))
                .bind((format!("comment_{v}"), m.comment.unwrap_or_default()))
                .bind((format!("checksum_{v}"), m.checksum()));
        }

        queries.query("COMMIT;").await?.check()?;

        trace!("committed migration transaction");

        Ok(())
    }

    /// Migrate downward. This is rolled back on error.
    /// All versions are db versions
    async fn goto_down<C: Connection>(
        &self,
        _db: &Surreal<C>,
        _current_version: usize,
        _target_version: usize,
    ) -> Result<()> {
        todo!()
    }

    /// Maximum version defined in the migration set
    fn max_schema_version(&self) -> SchemaVersion {
        match self.ms.len() {
            0 => SchemaVersion::NoneSet,
            v => SchemaVersion::Inside(
                NonZeroUsize::new(v).expect("schema version should not be equal to 0"),
            ),
        }
    }
}

// Read user version field from the db
async fn get_current_version<C: Connection>(db: &Surreal<C>) -> Result<usize, surrealdb::Error> {
    let mut result = db
        .query(r#"SELECT version FROM _migrations ORDER BY version DESC LIMIT 1"#)
        .await?
        .check()?;

    let query_result: Option<usize> = result.take((0, "version"))?;
    match query_result {
        Some(version) => Ok(version),
        None => Ok(0),
    }
}

/// Schema version, in the context of Migrations
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum SchemaVersion {
    /// No schema version set
    NoneSet,
    /// The current version in the database is inside the range of defined
    /// migrations
    Inside(NonZeroUsize),
    /// The current version in the database is outside any migration defined
    Outside(NonZeroUsize),
}

impl From<&SchemaVersion> for usize {
    /// Translate schema version to db version
    fn from(schema_version: &SchemaVersion) -> usize {
        match schema_version {
            SchemaVersion::NoneSet => 0,
            SchemaVersion::Inside(v) | SchemaVersion::Outside(v) => From::from(*v),
        }
    }
}

impl From<SchemaVersion> for usize {
    fn from(schema_version: SchemaVersion) -> Self {
        From::from(&schema_version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::Value;
    use surrealdb::sql::{Datetime, Thing};

    #[derive(Debug, Deserialize)]
    pub struct MigrationRow {
        pub id: Thing,
        pub version: usize,
        pub comment: String,
        pub checksum: String,
        pub installed_on: Datetime,
    }

    #[tokio::test]
    async fn empty_db_should_have_version_0() -> Result<()> {
        let db = surrealdb::engine::any::connect("mem://").await?;
        db.use_ns("test").use_db("test").await?;
        let version = get_current_version(&db).await?;
        assert_eq!(version, 0);
        Ok(())
    }

    #[tokio::test]
    async fn fail_with_no_migrations_defined_when_no_migrations() -> Result<()> {
        let db = surrealdb::engine::any::connect("mem://").await?;
        db.use_ns("test").use_db("test").await?;
        let migrations = Migrations::new(vec![]);
        let result = migrations.to_latest(&db).await;
        matches!(
            result,
            Err(Error::MigrationDefinition(
                MigrationDefinitionError::NoMigrationsDefined
            ))
        );
        Ok(())
    }

    #[tokio::test]
    async fn empty_migrations_table_is_created_when_run_migrations() -> Result<()> {
        let db = surrealdb::engine::any::connect("mem://").await?;
        db.use_ns("test").use_db("test").await?;
        let migrations = Migrations::new(vec![]);
        let _ = migrations.to_latest(&db).await;
        let mut result = db.query("INFO FOR TABLE _migrations;").await?.check()?;
        let result: Vec<Value> = result.take((0, "fields"))?;
        assert_eq!(
            &result[0]["checksum"],
            "DEFINE FIELD checksum ON _migrations TYPE string PERMISSIONS FULL"
        );
        assert_eq!(
            &result[0]["comment"],
            "DEFINE FIELD comment ON _migrations TYPE string PERMISSIONS FULL"
        );
        assert_eq!(
            &result[0]["installed_on"],
            "DEFINE FIELD installed_on ON _migrations TYPE datetime PERMISSIONS FULL"
        );
        assert_eq!(
            &result[0]["version"],
            "DEFINE FIELD version ON _migrations TYPE number PERMISSIONS FULL"
        );

        let mut result = db.query("SELECT count() from _migrations").await?.check()?;
        let query_result: Option<u64> = result.take((0, "count"))?;
        assert_eq!(query_result.unwrap_or_default(), 0);

        Ok(())
    }

    #[tokio::test]
    async fn run_to_latest() -> Result<()> {
        let db = surrealdb::engine::any::connect("mem://").await?;
        db.use_ns("test").use_db("test").await?;
        let migrations = Migrations::new(vec![
            M::up("DEFINE TABLE animal SCHEMAFULL; DEFINE FIELD name ON animal TYPE string; DEFINE FIELD created_at ON animal TYPE datetime DEFAULT time::now()")
                .comment("Create animal table"),
            M::up("INSERT INTO animal { name: 'dog' };"),
            M::up("INSERT INTO animal { name: 'cat' };"),
        ]);
        migrations.to_latest(&db).await?;

        let mut result = db
            .query("SELECT * from _migrations ORDER BY version")
            .await?
            .check()?;
        let query_result: Vec<MigrationRow> = result.take(0)?;

        assert_eq!(query_result.len(), 3);

        assert_eq!(query_result[0].version, 1);
        assert_eq!(query_result[0].comment, "Create animal table");
        assert_eq!(
            query_result[0].checksum,
            "4c2febc958756775b6f0fb01145674baded963d03c1aacea816ec4f61ee66ede"
        );

        assert_eq!(query_result[1].version, 2);
        assert_eq!(query_result[1].comment, "");
        assert_eq!(
            query_result[1].checksum,
            "ecc954865aba2afb559495875055b552320665d725c163b956a6aa349785a99e"
        );

        assert_eq!(query_result[2].version, 3);
        assert_eq!(query_result[2].comment, "");
        assert_eq!(
            query_result[2].checksum,
            "1e46fb427cd9e10b9bd7f77a97e64739a64e43acb3dbe30243448311c47c8693"
        );

        let mut result = db
            .query("SELECT name, created_at from animal ORDER BY created_at")
            .await?
            .check()?;
        let query_result: Vec<String> = result.take((0, "name"))?;

        assert_eq!(query_result.len(), 2);
        assert_eq!(query_result[0], "dog");
        assert_eq!(query_result[1], "cat");

        // run 2nd migration adding horse
        let migrations = Migrations::new(vec![
            M::up("DEFINE TABLE animal SCHEMAFULL; DEFINE FIELD name ON animal TYPE string;")
                .comment("Create animal table"),
            M::up("INSERT INTO animal { name: 'dog' };"),
            M::up("INSERT INTO animal { name: 'cat' };"),
            M::up("INSERT INTO animal { name: 'horse' };"),
        ]);
        migrations.to_latest(&db).await?;

        let mut result = db
            .query("SELECT * from _migrations ORDER BY version")
            .await?
            .check()?;
        let query_result: Vec<MigrationRow> = result.take(0)?;

        assert_eq!(query_result.len(), 4);

        assert_eq!(query_result[0].version, 1);
        assert_eq!(query_result[0].comment, "Create animal table");
        assert_eq!(
            query_result[0].checksum,
            "4c2febc958756775b6f0fb01145674baded963d03c1aacea816ec4f61ee66ede"
        );

        assert_eq!(query_result[1].version, 2);
        assert_eq!(query_result[1].comment, "");
        assert_eq!(
            query_result[1].checksum,
            "ecc954865aba2afb559495875055b552320665d725c163b956a6aa349785a99e"
        );

        assert_eq!(query_result[2].version, 3);
        assert_eq!(query_result[2].comment, "");
        assert_eq!(
            query_result[2].checksum,
            "1e46fb427cd9e10b9bd7f77a97e64739a64e43acb3dbe30243448311c47c8693"
        );

        assert_eq!(query_result[3].version, 4);
        assert_eq!(query_result[3].comment, "");
        assert_eq!(
            query_result[3].checksum,
            "b0afb9a8540da6104dbed171693e8434057f21b839cec9d331771e5a5804d0c4"
        );

        let mut result = db
            .query("SELECT name, created_at from animal ORDER BY created_at")
            .await?
            .check()?;
        let query_result: Vec<String> = result.take((0, "name"))?;

        assert_eq!(query_result.len(), 3);
        assert_eq!(query_result[0], "dog");
        assert_eq!(query_result[1], "cat");
        assert_eq!(query_result[2], "horse");

        Ok(())
    }
}
