use rusqlite::OptionalExtension;
use tokio_rusqlite::Connection;
use chrono::NaiveDate;
use anyhow::Result;

pub struct Class {
    pub id: usize,
    pub name: String,
    pub created_at: NaiveDate,
    pub deleted_at: Option<NaiveDate>
}
impl Class {
    pub fn is_deleted(&self) -> bool {
        self.deleted_at.is_some()
    }
}

pub struct User {
    pub id: usize,
    pub name: String,
    pub password_hash: String,
    pub created_at: NaiveDate,
    pub deleted_at: Option<NaiveDate>
}

pub struct DB {
    pub conn: Connection,
}
impl DB {
    pub async fn new(path: Option<&str>) -> Self {
        let conn = match path {
            Some(path) => Connection::open(path).await.expect("Failed to open database"),
            None => Connection::open_in_memory().await.expect("Failed to open database")
        };
        let db = Self { conn };
        db.create_tables().await;
        db
    }
    pub async fn create_tables(&self) {
        self.conn.call(|conn| {
            conn.execute("
                CREATE TABLE IF NOT EXISTS classes (
                    id INTEGER PRIMARY KEY,
                    name TEXT NOT NULL UNIQUE,
                    created_at TEXT NOT NULL,
                    deleted_at TEXT
                )
            ", ())?;
            conn.execute("
                CREATE TABLE IF NOT EXISTS users (
                    id INTEGER PRIMARY KEY,
                    username TEXT NOT NULL,
                    password_hash TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    deleted_at TEXT
                )
            ", ())?;
            conn.execute("
                CREATE TABLE IF NOT EXISTS user_classes (
                    user_id INTEGER NOT NULL,
                    class_id INTEGER NOT NULL,
                    FOREIGN KEY (user_id) REFERENCES users (id),
                    FOREIGN KEY (class_id) REFERENCES classes (id)
                )
            ", ())?;
            conn.execute("
                CREATE TABLE IF NOT EXISTS homeworks (
                    id INTEGER PRIMARY KEY,
                    name TEXT NOT NULL,
                    description TEXT NOT NULL,
                    class_id INTEGER NOT NULL,
                    created_by INTEGER,
                    lesson_id INTEGER,
                    created_at TEXT NOT NULL,
                    deleted_at TEXT,
                    FOREIGN KEY (class_id) REFERENCES classes (id),
                    FOREIGN KEY (created_by) REFERENCES users (id)
                )
            ", ())?;
            conn.execute("
                CREATE TABLE IF NOT EXISTS lesson_types (
                    id INTEGER PRIMARY KEY,
                    name TEXT NOT NULL UNIQUE,
                    created_at TEXT NOT NULL,
                    deleted_at TEXT
                )
            ", ())?;
            conn.execute("
                CREATE TABLE IF NOT EXISTS lessons (
                    id INTEGER PRIMARY KEY,
                    type_id INTEGER NOT NULL,
                    start TEXT NOT NULL,
                    end TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    deleted_at TEXT,
                    FOREIGN KEY (type_id) REFERENCES lesson_types (id)
                )
            ", ())?;
            conn.execute("
                CREATE TABLE IF NOT EXISTS lesson_classes (
                    lesson_id INTEGER NOT NULL,
                    class_id INTEGER NOT NULL,
                    FOREIGN KEY (lesson_id) REFERENCES lessons (id),
                    FOREIGN KEY (class_id) REFERENCES classes (id)
                )
            ", ())?;

            Ok(())
        }).await.expect("Failed to create tables");
    }
    
    // classes
    pub async fn insert_class(&self, name: String) -> Result<()> {
        self.conn.call(|conn| {
            conn.execute("
                INSERT INTO classes (name, created_at)
                VALUES (?1, datetime('now'))
            ", [name])?;
            Ok(())
        }).await?;
        Ok(())
    }
    pub async fn get_class(&self, id: usize) -> Option<Class> {
        self.conn.call(move |conn| {
            conn.query_row("
                SELECT id, name, created_at, deleted_at
                FROM classes
                WHERE id = ?1
            ", [id], |row| {
                Ok(Class {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    created_at: row.get(2)?,
                    deleted_at: row.get(3)?
                })
            }).optional()
        }).await.expect("Failed to get class")
    }

    // users
    pub async fn insert_user(&self, name: String, password_hash: String) -> Result<()> {
        self.conn.call(|conn| {
            conn.execute("
                INSERT INTO users (name, password_hash, created_at)
                VALUES (?1, ?2, datetime('now'))
            ", [name, password_hash])?;
            Ok(())
        }).await?;
        Ok(())
    }
    pub async fn get_user(&self, id: usize) -> Option<User> {
        self.conn.call(move |conn| {
            conn.query_row("
                SELECT id, name, password_hash, created_at, deleted_at
                FROM classes
                WHERE id = ?1
            ", [id], |row| {
                Ok(User {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    password_hash: row.get(2)?,
                    created_at: row.get(3)?,
                    deleted_at: row.get(4)?
                })
            }).optional()
        }).await.expect("Failed to get user")
    }
}
