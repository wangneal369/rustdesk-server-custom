use async_trait::async_trait;
use hbb_common::{log, ResultType};
use sqlx::{
    sqlite::SqliteConnectOptions, ConnectOptions, Connection, Error as SqlxError, SqliteConnection,
};
use std::{ops::DerefMut, str::FromStr};
//use sqlx::postgres::PgPoolOptions;
//use sqlx::mysql::MySqlPoolOptions;

type Pool = deadpool::managed::Pool<DbPool>;

pub struct DbPool {
    url: String,
}

#[async_trait]
impl deadpool::managed::Manager for DbPool {
    type Type = SqliteConnection;
    type Error = SqlxError;
    async fn create(&self) -> Result<SqliteConnection, SqlxError> {
        let opt = SqliteConnectOptions::from_str(&self.url)?
            .log_statements(log::LevelFilter::Debug);
        SqliteConnection::connect_with(&opt).await
    }
    async fn recycle(
        &self,
        obj: &mut SqliteConnection,
    ) -> deadpool::managed::RecycleResult<SqlxError> {
        Ok(obj.ping().await?)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AuditWrite { Stored, Duplicate, Conflict }

#[derive(Clone)]
pub struct Database {
    pool: Pool,
}

#[derive(Default)]
pub struct Peer {
    pub guid: Vec<u8>,
    pub id: String,
    pub uuid: Vec<u8>,
    pub pk: Vec<u8>,
    pub user: Option<Vec<u8>>,
    pub info: String,
    pub status: Option<i64>,
}

impl Database {
    pub async fn new(url: &str) -> ResultType<Database> {
        if !std::path::Path::new(url).exists() {
            std::fs::File::create(url)?;
        }
        let n: usize = crate::common::get_arg_or("MAX_DATABASE_CONNECTIONS", "1".to_owned())
            .parse()
            .unwrap_or(1);
        log::debug!("MAX_DATABASE_CONNECTIONS={}", n);
        let pool = Pool::new(
            DbPool {
                url: url.to_owned(),
            },
            n,
        );
        let _ = pool.get().await?; // test
        let db = Database { pool };
        db.create_tables().await?;
        Ok(db)
    }

    async fn create_tables(&self) -> ResultType<()> {
        sqlx::raw_sql(
            "
            create table if not exists peer (
                guid blob primary key not null,
                id varchar(100) not null,
                uuid blob not null,
                pk blob not null,
                created_at datetime not null default(current_timestamp),
                user blob,
                status tinyint,
                note varchar(300),
                info text not null
            ) without rowid;
            create unique index if not exists index_peer_id on peer (id);
            create index if not exists index_peer_user on peer (user);
            create index if not exists index_peer_created_at on peer (created_at);
            create index if not exists index_peer_status on peer (status);
        "
        )
        .execute(self.pool.get().await?.deref_mut())
        .await?;
        sqlx::raw_sql(
            "CREATE TABLE IF NOT EXISTS audit_event (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                device_id TEXT NOT NULL,
                nonce TEXT NOT NULL,
                kind TEXT NOT NULL,
                source_ip TEXT NOT NULL,
                payload TEXT NOT NULL,
                received_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                UNIQUE(device_id, nonce)
            );
            CREATE INDEX IF NOT EXISTS audit_received_at ON audit_event(received_at);"
        ).execute(self.pool.get().await?.deref_mut()).await?;
        Ok(())
    }

    pub(crate) async fn store_audit(
        &self, device: &str, nonce: &str, kind: &str, source: &str, payload: &str,
    ) -> ResultType<AuditWrite> {
        let mut conn = self.pool.get().await?;
        let inserted = sqlx::query("INSERT INTO audit_event(device_id,nonce,kind,source_ip,payload) VALUES(?,?,?,?,?) ON CONFLICT(device_id,nonce) DO NOTHING")
            .bind(device).bind(nonce).bind(kind).bind(source).bind(payload)
            .execute(conn.deref_mut()).await?.rows_affected() > 0;
        if !inserted {
            let existing: (String, String) = sqlx::query_as("SELECT kind,payload FROM audit_event WHERE device_id=? AND nonce=?")
                .bind(device).bind(nonce).fetch_one(conn.deref_mut()).await?;
            return Ok(if existing == (kind.to_owned(), payload.to_owned()) { AuditWrite::Duplicate } else { AuditWrite::Conflict });
        }
        Ok(AuditWrite::Stored)
    }

    pub async fn get_peer(&self, id: &str) -> ResultType<Option<Peer>> {
        Ok(sqlx::query_as!(
            Peer,
            "select guid, id, uuid, pk, user, status, info from peer where id = ?",
            id
        )
        .fetch_optional(self.pool.get().await?.deref_mut())
        .await?)
    }

    pub async fn insert_peer(
        &self,
        id: &str,
        uuid: &[u8],
        pk: &[u8],
        info: &str,
    ) -> ResultType<Vec<u8>> {
        let guid = uuid::Uuid::new_v4().as_bytes().to_vec();
        sqlx::query!(
            "insert into peer(guid, id, uuid, pk, info) values(?, ?, ?, ?, ?)",
            guid,
            id,
            uuid,
            pk,
            info
        )
        .execute(self.pool.get().await?.deref_mut())
        .await?;
        Ok(guid)
    }

    pub async fn update_pk(
        &self,
        guid: &Vec<u8>,
        id: &str,
        pk: &[u8],
        info: &str,
    ) -> ResultType<()> {
        sqlx::query!(
            "update peer set id=?, pk=?, info=? where guid=?",
            id,
            pk,
            info,
            guid
        )
        .execute(self.pool.get().await?.deref_mut())
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hbb_common::tokio;
    #[tokio::test]
    async fn peer_updates_and_audit_records_survive_database_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.sqlite3");
        let path = path.to_str().unwrap();
        let db = Database::new(path).await.unwrap();
        let guid = db.insert_peer("original", b"device", b"first-key", "{}").await.unwrap();
        db.update_pk(&guid, "renamed", b"second-key", "{\"ip\":\"127.0.0.1\"}").await.unwrap();
        assert!(db.get_peer("original").await.unwrap().is_none());
        let peer = db.get_peer("renamed").await.unwrap().unwrap();
        assert_eq!(peer.guid, guid);
        assert_eq!(peer.pk, b"second-key");
        assert_eq!(peer.uuid, b"device");
        assert_eq!(db.store_audit("device", "nonce", "conn", "127.0.0.1", "{}").await.unwrap(), AuditWrite::Stored);
        drop(db);
        let db = Database::new(path).await.unwrap();
        assert_eq!(db.get_peer("renamed").await.unwrap().unwrap().guid, guid);
        assert_eq!(db.store_audit("device", "nonce", "conn", "127.0.0.2", "{}").await.unwrap(), AuditWrite::Duplicate);
        assert_eq!(db.store_audit("device", "nonce", "file", "127.0.0.1", "{}").await.unwrap(), AuditWrite::Conflict);
    }

    #[test]
    fn test_insert() {
        insert();
    }

    #[tokio::main(flavor = "multi_thread")]
    async fn insert() {
        let db = super::Database::new("test.sqlite3").await.unwrap();
        let mut jobs = vec![];
        for i in 0..10000 {
            let cloned = db.clone();
            let id = i.to_string();
            let a = tokio::spawn(async move {
                let empty_vec = Vec::new();
                cloned
                    .insert_peer(&id, &empty_vec, &empty_vec, "")
                    .await
                    .unwrap();
            });
            jobs.push(a);
        }
        for i in 0..10000 {
            let cloned = db.clone();
            let id = i.to_string();
            let a = tokio::spawn(async move {
                cloned.get_peer(&id).await.unwrap();
            });
            jobs.push(a);
        }
        hbb_common::futures::future::join_all(jobs).await;
    }
}
