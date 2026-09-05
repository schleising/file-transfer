//! Persist computers and saved locations.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationKind {
    Source,
    Dest,
    Either,
}

impl LocationKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Dest => "dest",
            Self::Either => "either",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "source" => Self::Source,
            "dest" => Self::Dest,
            _ => Self::Either,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Computer {
    pub id: Uuid,
    pub name: String,
    /// SSH destination: alias or `user@host`. Empty when `is_local`.
    pub ssh_destination: String,
    pub ssh_port: Option<u16>,
    pub identity_file: Option<PathBuf>,
    pub bonjour_name: Option<String>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub is_local: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Location {
    pub id: Uuid,
    pub computer_id: Uuid,
    pub name: String,
    pub path: PathBuf,
    pub kind: LocationKind,
    pub sort_order: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open_default() -> Result<Self> {
        let dir = app_data_dir()?;
        std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        Self::open(dir.join("file-transfer.sqlite3"))
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path.as_ref())
            .with_context(|| format!("open db {}", path.as_ref().display()))?;
        conn.busy_timeout(std::time::Duration::from_millis(2000))?;
        let store = Self { conn };
        store.migrate()?;
        store.ensure_local_computer()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS computers (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                ssh_destination TEXT NOT NULL DEFAULT '',
                ssh_port INTEGER,
                identity_file TEXT,
                bonjour_name TEXT,
                last_seen_at TEXT,
                is_local INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS locations (
                id TEXT PRIMARY KEY NOT NULL,
                computer_id TEXT NOT NULL REFERENCES computers(id) ON DELETE CASCADE,
                name TEXT NOT NULL,
                path TEXT NOT NULL,
                kind TEXT NOT NULL,
                sort_order INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            DROP TABLE IF EXISTS jobs;
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            );
            "#,
        )?;
        self.migrate_locations_sort_order()?;
        Ok(())
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    fn migrate_locations_sort_order(&self) -> Result<()> {
        let has_column: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('locations') WHERE name = 'sort_order'",
            [],
            |r| r.get(0),
        )?;
        if has_column == 0 {
            self.conn.execute(
                "ALTER TABLE locations ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
            let mut stmt = self.conn.prepare(
                "SELECT id FROM locations WHERE computer_id = ?1 ORDER BY name COLLATE NOCASE",
            )?;
            let computer_ids: Vec<String> = self
                .conn
                .prepare("SELECT id FROM computers")?
                .query_map([], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            for computer_id in computer_ids {
                let ids = stmt
                    .query_map(params![computer_id], |r| r.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                for (index, id) in ids.iter().enumerate() {
                    self.conn.execute(
                        "UPDATE locations SET sort_order = ?1 WHERE id = ?2",
                        params![index as i32, id],
                    )?;
                }
            }
        }
        Ok(())
    }

    fn ensure_local_computer(&self) -> Result<()> {
        let exists: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM computers WHERE is_local = 1 LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if exists.is_some() {
            return Ok(());
        }
        let now = Utc::now();
        let id = Uuid::new_v4();
        self.conn.execute(
            "INSERT INTO computers (id, name, ssh_destination, ssh_port, identity_file, bonjour_name, last_seen_at, is_local, created_at, updated_at)
             VALUES (?1, ?2, '', NULL, NULL, NULL, NULL, 1, ?3, ?4)",
            params![
                id.to_string(),
                "This Mac",
                now.to_rfc3339(),
                now.to_rfc3339()
            ],
        )?;
        if let Some(home) = dirs::home_dir() {
            let loc_id = Uuid::new_v4();
            self.conn.execute(
                "INSERT INTO locations (id, computer_id, name, path, kind, sort_order, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    loc_id.to_string(),
                    id.to_string(),
                    "Home",
                    home.to_string_lossy(),
                    "either",
                    0_i32,
                    now.to_rfc3339(),
                    now.to_rfc3339(),
                ],
            )?;
        }
        Ok(())
    }

    pub fn local_computer(&self) -> Result<Computer> {
        self.computers()?
            .into_iter()
            .find(|c| c.is_local)
            .context("local computer missing")
    }

    pub fn computers(&self) -> Result<Vec<Computer>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, ssh_destination, ssh_port, identity_file, bonjour_name, last_seen_at, is_local, created_at, updated_at
             FROM computers ORDER BY is_local DESC, name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Computer {
                id: parse_uuid(&row.get::<_, String>(0)?)?,
                name: row.get(1)?,
                ssh_destination: row.get(2)?,
                ssh_port: row.get::<_, Option<i64>>(3)?.map(|p| p as u16),
                identity_file: row.get::<_, Option<String>>(4)?.map(PathBuf::from),
                bonjour_name: row.get(5)?,
                last_seen_at: parse_opt_time(row.get(6)?)?,
                is_local: row.get::<_, i64>(7)? != 0,
                created_at: parse_time(&row.get::<_, String>(8)?)?,
                updated_at: parse_time(&row.get::<_, String>(9)?)?,
            })
        })?;
        collect_mapped(rows)
    }

    pub fn upsert_computer(&self, c: &Computer) -> Result<()> {
        self.conn.execute(
            "INSERT INTO computers (id, name, ssh_destination, ssh_port, identity_file, bonjour_name, last_seen_at, is_local, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
                name=excluded.name,
                ssh_destination=excluded.ssh_destination,
                ssh_port=excluded.ssh_port,
                identity_file=excluded.identity_file,
                bonjour_name=excluded.bonjour_name,
                last_seen_at=excluded.last_seen_at,
                is_local=excluded.is_local,
                updated_at=excluded.updated_at",
            params![
                c.id.to_string(),
                c.name,
                c.ssh_destination,
                c.ssh_port.map(|p| p as i64),
                c.identity_file.as_ref().map(|p| p.to_string_lossy().to_string()),
                c.bonjour_name,
                c.last_seen_at.map(|t| t.to_rfc3339()),
                c.is_local as i64,
                c.created_at.to_rfc3339(),
                c.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn locations(&self) -> Result<Vec<Location>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, computer_id, name, path, kind, sort_order, created_at, updated_at
             FROM locations ORDER BY sort_order, name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], map_location)?;
        collect_mapped(rows)
    }

    pub fn locations_for(&self, computer_id: Uuid) -> Result<Vec<Location>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, computer_id, name, path, kind, sort_order, created_at, updated_at
             FROM locations WHERE computer_id = ?1 ORDER BY sort_order, name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map(params![computer_id.to_string()], map_location)?;
        collect_mapped(rows)
    }

    pub fn upsert_location(&self, loc: &Location) -> Result<()> {
        self.conn.execute(
            "INSERT INTO locations (id, computer_id, name, path, kind, sort_order, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                computer_id=excluded.computer_id,
                name=excluded.name,
                path=excluded.path,
                kind=excluded.kind,
                sort_order=excluded.sort_order,
                updated_at=excluded.updated_at",
            params![
                loc.id.to_string(),
                loc.computer_id.to_string(),
                loc.name,
                loc.path.to_string_lossy(),
                loc.kind.as_str(),
                loc.sort_order,
                loc.created_at.to_rfc3339(),
                loc.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn reorder_locations(&self, computer_id: Uuid, ordered_ids: &[Uuid]) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        for (index, id) in ordered_ids.iter().enumerate() {
            self.conn.execute(
                "UPDATE locations SET sort_order = ?1, updated_at = ?2
                 WHERE id = ?3 AND computer_id = ?4",
                params![index as i32, now, id.to_string(), computer_id.to_string(),],
            )?;
        }
        Ok(())
    }

    pub fn delete_location(&self, id: Uuid) -> Result<()> {
        self.conn.execute(
            "DELETE FROM locations WHERE id = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }
}

fn app_data_dir() -> Result<PathBuf> {
    let base = dirs::data_dir().context("no data dir")?;
    Ok(base.join("File Transfer"))
}

fn parse_uuid(s: &str) -> Result<Uuid, rusqlite::Error> {
    Uuid::parse_str(s).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

fn parse_time(s: &str) -> Result<DateTime<Utc>, rusqlite::Error> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

fn parse_opt_time(s: Option<String>) -> Result<Option<DateTime<Utc>>, rusqlite::Error> {
    match s {
        None => Ok(None),
        Some(s) => Ok(Some(parse_time(&s)?)),
    }
}

fn map_location(row: &rusqlite::Row<'_>) -> rusqlite::Result<Location> {
    Ok(Location {
        id: parse_uuid(&row.get::<_, String>(0)?)?,
        computer_id: parse_uuid(&row.get::<_, String>(1)?)?,
        name: row.get(2)?,
        path: PathBuf::from(row.get::<_, String>(3)?),
        kind: LocationKind::parse(&row.get::<_, String>(4)?),
        sort_order: row.get(5)?,
        created_at: parse_time(&row.get::<_, String>(6)?)?,
        updated_at: parse_time(&row.get::<_, String>(7)?)?,
    })
}

fn collect_mapped<T, I>(rows: I) -> Result<Vec<T>>
where
    I: Iterator<Item = Result<T, rusqlite::Error>>,
{
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_computer_location() {
        let store = Store::open(":memory:").unwrap();
        let local = store.local_computer().unwrap();
        assert!(local.is_local);

        let now = Utc::now();
        let remote = Computer {
            id: Uuid::new_v4(),
            name: "NAS".into(),
            ssh_destination: "nas".into(),
            ssh_port: None,
            identity_file: None,
            bonjour_name: Some("nas.local".into()),
            last_seen_at: None,
            is_local: false,
            created_at: now,
            updated_at: now,
        };
        store.upsert_computer(&remote).unwrap();

        let loc = Location {
            id: Uuid::new_v4(),
            computer_id: remote.id,
            name: "Media".into(),
            path: PathBuf::from("/data/media"),
            kind: LocationKind::Either,
            sort_order: 0,
            created_at: now,
            updated_at: now,
        };
        store.upsert_location(&loc).unwrap();
        assert_eq!(store.locations_for(remote.id).unwrap().len(), 1);
    }

    #[test]
    fn settings_roundtrip() {
        let store = Store::open(":memory:").unwrap();
        assert_eq!(store.setting("window.frame").unwrap(), None);
        store
            .set_setting("window.frame", "100 80 1280 840")
            .unwrap();
        assert_eq!(
            store.setting("window.frame").unwrap().as_deref(),
            Some("100 80 1280 840")
        );
        store.set_setting("window.frame", "10 20 1300 860").unwrap();
        assert_eq!(
            store.setting("window.frame").unwrap().as_deref(),
            Some("10 20 1300 860")
        );
    }
}
