//! History segments: immutable git packs in an S3-compatible bucket, listed
//! in order by the `repo_segments` manifest. A node hydrates a repository by
//! downloading the segments it lacks; a push adds one segment (the
//! quarantined pack); server-made objects are packed and added the same way.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use diesel::prelude::*;
use uuid::Uuid;
use vcsstar_db::schema::repo_segments;

use crate::{Result, StoreError};

pub trait SegmentStore: Send + Sync {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<()>;
    fn get(&self, key: &str) -> Result<Vec<u8>>;
    fn delete(&self, key: &str) -> Result<()>;
    fn describe(&self) -> String;
}

/// S3-compatible store (SeaweedFS, MinIO, AWS) with path-style addressing.
pub struct S3Store {
    bucket: Box<s3::Bucket>,
}

impl S3Store {
    pub fn new(
        endpoint: &str,
        bucket: &str,
        region: &str,
        access_key: &str,
        secret_key: &str,
    ) -> Result<Self> {
        let region = s3::Region::Custom {
            region: region.to_string(),
            endpoint: endpoint.trim_end_matches('/').to_string(),
        };
        let creds =
            s3::creds::Credentials::new(Some(access_key), Some(secret_key), None, None, None)
                .map_err(|e| StoreError::Other(format!("s3 credentials: {e}")))?;
        let bucket = s3::Bucket::new(bucket, region, creds)
            .map_err(|e| StoreError::Other(format!("s3 bucket: {e}")))?
            .with_path_style();
        Ok(Self { bucket })
    }
}

impl SegmentStore for S3Store {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<()> {
        let resp = self
            .bucket
            .put_object(key, bytes)
            .map_err(|e| StoreError::Other(format!("s3 put {key}: {e}")))?;
        if !(200..300).contains(&resp.status_code()) {
            return Err(StoreError::Other(format!(
                "s3 put {key}: HTTP {}",
                resp.status_code()
            )));
        }
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Vec<u8>> {
        let resp = self
            .bucket
            .get_object(key)
            .map_err(|e| StoreError::Other(format!("s3 get {key}: {e}")))?;
        if resp.status_code() != 200 {
            return Err(StoreError::Other(format!(
                "s3 get {key}: HTTP {}",
                resp.status_code()
            )));
        }
        Ok(resp.bytes().to_vec())
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.bucket
            .delete_object(key)
            .map_err(|e| StoreError::Other(format!("s3 delete {key}: {e}")))?;
        Ok(())
    }

    fn describe(&self) -> String {
        format!("s3://{}", self.bucket.name())
    }
}

/// In-memory store for tests.
#[derive(Default)]
pub struct MemoryStore {
    objects: Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
}

impl SegmentStore for MemoryStore {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<()> {
        self.objects
            .lock()
            .unwrap()
            .insert(key.to_string(), bytes.to_vec());
        Ok(())
    }
    fn get(&self, key: &str) -> Result<Vec<u8>> {
        self.objects
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or_else(|| StoreError::Other(format!("missing segment {key}")))
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.objects.lock().unwrap().remove(key);
        Ok(())
    }
    fn describe(&self) -> String {
        "memory".into()
    }
}

/// One manifest row.
#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = repo_segments)]
pub struct Segment {
    pub repo_id: Uuid,
    pub ordinal: i32,
    pub object_key: String,
    pub size: i64,
    pub content_hash: String,
    pub kind: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub superseded_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Live segments in order.
pub fn manifest(conn: &mut PgConnection, repo_id: Uuid) -> Result<Vec<Segment>> {
    Ok(repo_segments::table
        .filter(repo_segments::repo_id.eq(repo_id))
        .filter(repo_segments::superseded_at.is_null())
        .order(repo_segments::ordinal.asc())
        .select(Segment::as_select())
        .load(conn)?)
}

/// Append one segment to the manifest (inside the caller's transaction).
pub fn append(
    conn: &mut PgConnection,
    repo_id: Uuid,
    object_key: &str,
    size: i64,
    content_hash: &str,
    kind: &str,
) -> Result<i32> {
    let next: Option<i32> = repo_segments::table
        .filter(repo_segments::repo_id.eq(repo_id))
        .select(diesel::dsl::max(repo_segments::ordinal))
        .first(conn)?;
    let ordinal = next.unwrap_or(0) + 1;
    diesel::insert_into(repo_segments::table)
        .values((
            repo_segments::repo_id.eq(repo_id),
            repo_segments::ordinal.eq(ordinal),
            repo_segments::object_key.eq(object_key),
            repo_segments::size.eq(size),
            repo_segments::content_hash.eq(content_hash),
            repo_segments::kind.eq(kind),
        ))
        .execute(conn)?;
    Ok(ordinal)
}

/// Fork: the child starts with the parent's manifest (copy-on-write).
pub fn copy_all(conn: &mut PgConnection, src: Uuid, dst: Uuid) -> Result<usize> {
    let rows = manifest(conn, src)?;
    for s in &rows {
        diesel::insert_into(repo_segments::table)
            .values((
                repo_segments::repo_id.eq(dst),
                repo_segments::ordinal.eq(s.ordinal),
                repo_segments::object_key.eq(&s.object_key),
                repo_segments::size.eq(s.size),
                repo_segments::content_hash.eq(&s.content_hash),
                repo_segments::kind.eq(&s.kind),
            ))
            .execute(conn)?;
    }
    Ok(rows.len())
}

/// A pack file pair ready to upload or already uploaded.
#[derive(Debug, Clone)]
pub struct PackFiles {
    pub pack: PathBuf,
    pub idx: PathBuf,
    /// `pack-<hash>` without extension.
    pub stem: String,
}

/// Find the pack(s) in a directory (`<quarantine>/pack` or `objects/pack`).
pub fn packs_in(dir: &Path) -> Result<Vec<PackFiles>> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(out);
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|s| s.to_str()) == Some("pack") {
            let stem = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let idx = p.with_extension("idx");
            if idx.exists() {
                out.push(PackFiles { pack: p, idx, stem });
            }
        }
    }
    out.sort_by(|a, b| a.stem.cmp(&b.stem));
    Ok(out)
}

pub fn object_key(repo_id: Uuid, stem: &str) -> String {
    format!("repos/{}/{stem}.pack", repo_id.simple())
}

pub fn idx_key(pack_key: &str) -> String {
    pack_key.trim_end_matches(".pack").to_string() + ".idx"
}

/// The pack checksum (trailing 20 or 32 bytes) as hex: the content hash.
pub fn pack_hash(stem: &str) -> String {
    stem.trim_start_matches("pack-").to_string()
}
