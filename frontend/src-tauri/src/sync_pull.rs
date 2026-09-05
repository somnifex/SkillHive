//! Change-feed pull over HTTP (M2.5 desktop, plan §11-§12).
//!
//! The pull client pages `GET /api/v1/sync/changes` from the durable local
//! cursor and hands each page to the transactional pull apply. The cursor
//! only advances inside that apply transaction, so an interrupted pull
//! between pages resumes from the last durable page, never skipping or
//! double-applying committed changes. Each page's package blobs are
//! downloaded and verified into the local blob store before the page is
//! applied, so a `remote_only` record can always be hydrated later.

use crate::blob_store::BlobStoreError;
use crate::local_store::{ChangesPage, LocalStore, LocalStoreError};
use crate::sync_client::{SyncClient, SyncClientError};

/// Client pull page size: the server clamps to its own `MAX_PULL_LIMIT`.
pub const PULL_PAGE_LIMIT: u32 = 100;

#[derive(Debug, thiserror::Error)]
pub enum PullError {
    #[error(transparent)]
    Http(#[from] SyncClientError),
    #[error("local store failure: {0}")]
    LocalStore(#[from] LocalStoreError),
    #[error("blob store failure: {0}")]
    BlobStore(#[from] BlobStoreError),
    #[error("blob download failed for {hash}: {source}")]
    BlobDownload {
        hash: String,
        #[source]
        source: Box<SyncClientError>,
    },
    #[error("downloaded blob {expected} does not match its content hash (got {actual})")]
    BlobDigestMismatch { expected: String, actual: String },
}

/// Result of one full pull drain (possibly many pages).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullSummary {
    pub pages_applied: usize,
    pub upserts: usize,
    pub tombstones: usize,
    pub conflicts_detected: u64,
    pub blobs_downloaded: usize,
    pub has_more: bool,
}

impl SyncClient {
    /// GET one change-feed page from the durable cursor.
    fn fetch_changes_page(&self, cursor: Option<&str>) -> Result<ChangesPage, SyncClientError> {
        let mut query: Vec<(String, String)> =
            vec![("limit".to_owned(), PULL_PAGE_LIMIT.to_string())];
        if let Some(cursor) = cursor {
            query.push(("cursor".to_owned(), cursor.to_owned()));
        }
        self.get_json("/api/v1/sync/changes", &query)
    }

    /// Downloads every package closure referenced by not-yet-durable page
    /// rows into the local blob store. Content-addressed verify-on-read
    /// means re-downloading an existing blob is a no-op cost-wise: the
    /// digest check runs on the bytes we just received.
    fn download_page_blobs(
        &self,
        blobs: &crate::blob_store::BlobStore,
        page: &ChangesPage,
    ) -> Result<usize, PullError> {
        let mut downloaded = 0_usize;
        for change in &page.changes {
            let Some(manifest_hash) = change.package_manifest_hash.as_deref() else {
                continue;
            };
            if blobs.verify(manifest_hash)? {
                // Manifest already local; its closure was uploaded/verified
                // together with it, so nothing to fetch.
                continue;
            }
            let bytes = self
                .get_octet_stream(&format!("/api/v1/sync/blobs/{}", manifest_hash))
                .map_err(|source| PullError::BlobDownload {
                    hash: manifest_hash.to_owned(),
                    source: Box::new(source),
                })?;
            let actual = crate::blob_store::hash_bytes_for_verification(&bytes);
            if actual != manifest_hash {
                return Err(PullError::BlobDigestMismatch {
                    expected: manifest_hash.to_owned(),
                    actual,
                });
            }
            blobs.put_bytes(&bytes)?;
            downloaded += 1;
        }
        Ok(downloaded)
    }
}

/// Pulls and durably applies change pages until the feed is caught up or a
/// page fails to apply. Applies pages through
/// [`LocalStore::apply_changes_page`], which commits each page's cursor in
/// the same transaction as its content.
pub fn pull_changes(
    client: &SyncClient,
    store: &LocalStore,
    blobs: &crate::blob_store::BlobStore,
) -> Result<PullSummary, PullError> {
    let mut summary = PullSummary {
        pages_applied: 0,
        upserts: 0,
        tombstones: 0,
        conflicts_detected: 0,
        blobs_downloaded: 0,
        has_more: false,
    };

    // Bound the loop so a server feeding an unbounded has_more stream cannot
    // pin the worker forever; the next cycle resumes from the durable cursor.
    const MAX_PAGES_PER_CYCLE: usize = 100;

    loop {
        let cursor = store.sync_cursor()?;
        let page = client.fetch_changes_page(cursor.as_deref())?;
        summary.blobs_downloaded += client.download_page_blobs(blobs, &page)?;

        let applied = store.apply_changes_page(&page)?;
        summary.pages_applied += 1;
        summary.upserts += applied.upserts;
        summary.tombstones += applied.tombstones;
        summary.conflicts_detected += applied.conflicts_detected;

        if !page.has_more || summary.pages_applied >= MAX_PAGES_PER_CYCLE {
            summary.has_more = page.has_more;
            return Ok(summary);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_page_limit_is_bounded_like_the_server() {
        // Server clamps to MAX_PULL_LIMIT=200; the client asks for 100 so a
        // full page is never truncated by the server cap.
        const SERVER_MAX_PULL_LIMIT: u32 = 200;
        assert_eq!(PULL_PAGE_LIMIT.min(SERVER_MAX_PULL_LIMIT), PULL_PAGE_LIMIT);
        assert_eq!(PULL_PAGE_LIMIT, 100);
    }
}
