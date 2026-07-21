//! Last-good snapshot cache.
//!
//! Stored as plain JSON in `%LOCALAPPDATA%\CodexPlus\usage-cache.json`.
//! The cache is intentionally *only* a snapshot — no credentials, no
//! account identifiers, no response bodies, no headers.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::error::AppError;
use crate::model::{
    RateWindow, UsageSnapshot, WindowKind, FIVE_HOUR_SECONDS, WEEKLY_SECONDS,
};

const SCHEMA_VERSION: u32 = 1;

pub fn cache_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|p| p.join("CodexPlus"))
}

pub fn cache_path() -> Option<PathBuf> {
    cache_dir().map(|p| p.join("usage-cache.json"))
}

pub fn load() -> Option<UsageSnapshot> {
    let path = cache_path()?;
    load_from(&path).ok().flatten()
}

pub fn load_from(path: &Path) -> Result<Option<UsageSnapshot>, AppError> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(AppError::CacheIo(err.to_string())),
    };
    let parsed: CacheFile = match serde_json::from_str(&raw) {
        Ok(c) => c,
        Err(_) => return Err(AppError::CacheInvalid),
    };
    if parsed.schema_version != SCHEMA_VERSION {
        return Err(AppError::CacheInvalid);
    }
    let snap: UsageSnapshot = parsed.snapshot.try_into()?;
    Ok(Some(snap))
}

pub fn save(snapshot: &UsageSnapshot) -> Result<(), AppError> {
    let path = cache_path().ok_or(AppError::AuthPathUnset)?;
    save_to(&path, snapshot)
}

pub fn save_to(path: &Path, snapshot: &UsageSnapshot) -> Result<(), AppError> {
    if !snapshot.is_sane() {
        return Err(AppError::CacheInvalid);
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| AppError::CacheIo(e.to_string()))?;
    }
    let payload = CacheFile {
        schema_version: SCHEMA_VERSION,
        snapshot: CacheSnapshot::from(snapshot.clone()),
    };
    let body = serde_json::to_string_pretty(&payload)
        .map_err(|_| AppError::CacheIo("serialise".into()))?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = fs::File::create(&tmp).map_err(|e| AppError::CacheIo(e.to_string()))?;
        file.write_all(body.as_bytes()).map_err(|e| AppError::CacheIo(e.to_string()))?;
        file.sync_all().map_err(|e| AppError::CacheIo(e.to_string()))?;
    }
    if let Err(err) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(AppError::CacheIo(err.to_string()));
    }
    Ok(())
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct CacheFile {
    schema_version: u32,
    snapshot: CacheSnapshot,
}

/// Cache representation intentionally mirrors the domain snapshot but is
/// kept private so future schema migrations can land without disturbing
/// [`crate::model`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CacheSnapshot {
    five_hour: CacheWindow,
    weekly: CacheWindow,
    fetched_at_unix: i64,
}

impl From<UsageSnapshot> for CacheSnapshot {
    fn from(value: UsageSnapshot) -> Self {
        Self {
            five_hour: value.five_hour.into(),
            weekly: value.weekly.into(),
            fetched_at_unix: value.fetched_at_unix,
        }
    }
}

impl TryFrom<CacheSnapshot> for UsageSnapshot {
    type Error = AppError;
    fn try_from(value: CacheSnapshot) -> Result<Self, Self::Error> {
        let five_hour = value.five_hour.try_into()?;
        let weekly = value.weekly.try_into()?;
        let snapshot = UsageSnapshot {
            five_hour,
            weekly,
            fetched_at_unix: value.fetched_at_unix,
        };
        if !snapshot.is_sane() {
            return Err(AppError::CacheInvalid);
        }
        Ok(snapshot)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CacheWindow {
    used_percent: f64,
    resets_at_unix: i64,
    window_seconds: u64,
}

impl From<RateWindow> for CacheWindow {
    fn from(value: RateWindow) -> Self {
        Self {
            used_percent: value.used_percent,
            resets_at_unix: value.resets_at_unix,
            window_seconds: value.window_seconds,
        }
    }
}

impl TryFrom<CacheWindow> for RateWindow {
    type Error = AppError;
    fn try_from(value: CacheWindow) -> Result<Self, Self::Error> {
        let kind = match value.window_seconds {
            FIVE_HOUR_SECONDS => WindowKind::FiveHour,
            WEEKLY_SECONDS => WindowKind::Weekly,
            other => {
                return Err(AppError::WindowDurationMismatch {
                    kind: WindowKind::Other,
                    actual_seconds: other as i64,
                })
            }
        };
        if !value.used_percent.is_finite() || !(0.0..=100.0).contains(&value.used_percent) {
            return Err(AppError::PercentInvalid(kind));
        }
        Ok(RateWindow {
            kind,
            used_percent: value.used_percent,
            window_seconds: value.window_seconds,
            resets_at_unix: value.resets_at_unix,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RateWindow, UsageSnapshot, WindowKind};

    fn snap(used_five: f64, reset_five: i64) -> UsageSnapshot {
        UsageSnapshot {
            five_hour: RateWindow {
                kind: WindowKind::FiveHour,
                used_percent: used_five,
                window_seconds: FIVE_HOUR_SECONDS,
                resets_at_unix: reset_five,
            },
            weekly: RateWindow {
                kind: WindowKind::Weekly,
                used_percent: 10.0,
                window_seconds: WEEKLY_SECONDS,
                resets_at_unix: UsageSnapshot::now() + 60 * 60 * 24 * 4,
            },
            fetched_at_unix: UsageSnapshot::now(),
        }
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("usage-cache.json");
        let snapshot = snap(30.0, UsageSnapshot::now() + 60 * 60 * 2);
        save_to(&path, &snapshot).expect("save");
        let loaded = load_from(&path).expect("ok").expect("some");
        assert!((loaded.five_hour.used_percent - 30.0).abs() < 0.0001);
    }

    #[test]
    fn rejects_invalid_percent_on_load() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("usage-cache.json");
        let mut cache = CacheFile {
            schema_version: SCHEMA_VERSION,
            snapshot: CacheSnapshot::from(snap(50.0, UsageSnapshot::now() + 60)),
        };
        cache.snapshot.five_hour.used_percent = 200.0;
        let body = serde_json::to_string(&cache).unwrap();
        std::fs::write(&path, body).unwrap();
        let err = load_from(&path).expect_err("invalid");
        // TryFrom converts the inner window first; invalid percent surfaces
        // before sanity validation, so we may see PercentInvalid here.
        match err {
            AppError::PercentInvalid(crate::model::WindowKind::FiveHour) => {}
            AppError::CacheInvalid => {}
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn rejects_unsupported_schema() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("usage-cache.json");
        std::fs::write(&path, r#"{"schema_version":99,"snapshot":{}}"#).unwrap();
        let err = load_from(&path).expect_err("invalid");
        assert_eq!(err, AppError::CacheInvalid);
    }

    #[test]
    fn save_rejects_insane_snapshot() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("usage-cache.json");
        let snapshot = snap(20.0, UsageSnapshot::now() + 100);
        save_to(&path, &snapshot).expect("save ok");
        let mut broken = snap(99.0, UsageSnapshot::now() + 100);
        broken.five_hour.window_seconds = 60;
        assert_eq!(save_to(&path, &broken), Err(AppError::CacheInvalid));
    }
}
