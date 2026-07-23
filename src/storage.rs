//! Last-good snapshot cache.
//!
//! Stored as plain JSON in `%LOCALAPPDATA%\CodexPlus\usage-cache.json`.
//! The cache is intentionally *only* a snapshot — no credentials, no
//! account identifiers, no response bodies, no headers.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::error::AppError;
use crate::model::{RateWindow, UsageSnapshot, WindowKind, FIVE_HOUR_SECONDS};

const SCHEMA_VERSION: u32 = 2;
const LEGACY_SCHEMA_VERSION: u32 = 1;

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
    if !matches!(
        parsed.schema_version,
        LEGACY_SCHEMA_VERSION | SCHEMA_VERSION
    ) {
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
        file.write_all(body.as_bytes())
            .map_err(|e| AppError::CacheIo(e.to_string()))?;
        file.sync_all()
            .map_err(|e| AppError::CacheIo(e.to_string()))?;
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
    #[serde(default)]
    five_hour: Option<CacheWindow>,
    #[serde(default)]
    weekly: Option<CacheWindow>,
    fetched_at_unix: i64,
}

impl From<UsageSnapshot> for CacheSnapshot {
    fn from(value: UsageSnapshot) -> Self {
        Self {
            five_hour: value.five_hour.map(Into::into),
            weekly: value.weekly.map(Into::into),
            fetched_at_unix: value.fetched_at_unix,
        }
    }
}

impl TryFrom<CacheSnapshot> for UsageSnapshot {
    type Error = AppError;
    fn try_from(value: CacheSnapshot) -> Result<Self, Self::Error> {
        let five_hour = value
            .five_hour
            .map(|window| window.into_rate_window(WindowKind::FiveHour))
            .transpose()?;
        let weekly = value
            .weekly
            .map(|window| window.into_rate_window(WindowKind::Weekly))
            .transpose()?;
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

impl CacheWindow {
    /// Reconstruct the role-bound window. The caller supplies the slot so
    /// that the round trip never has to infer a `WindowKind` from a
    /// duration alone; that was a real source of false rejections when the
    /// server shifted the canonical window length.
    fn into_rate_window(self, kind: WindowKind) -> Result<RateWindow, AppError> {
        if !kind.accepts_duration(self.window_seconds) {
            return Err(AppError::WindowDurationMismatch {
                kind,
                actual_seconds: self.window_seconds as i64,
                expected_seconds: kind.canonical_seconds().unwrap_or(FIVE_HOUR_SECONDS),
            });
        }
        if !self.used_percent.is_finite() || !(0.0..=100.0).contains(&self.used_percent) {
            return Err(AppError::PercentInvalid(kind));
        }
        Ok(RateWindow {
            kind,
            used_percent: self.used_percent,
            window_seconds: self.window_seconds,
            resets_at_unix: self.resets_at_unix,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RateWindow, UsageSnapshot, WindowKind, WEEKLY_SECONDS};

    fn snap(used_five: f64, reset_five: i64) -> UsageSnapshot {
        UsageSnapshot {
            five_hour: Some(RateWindow {
                kind: WindowKind::FiveHour,
                used_percent: used_five,
                window_seconds: FIVE_HOUR_SECONDS,
                resets_at_unix: reset_five,
            }),
            weekly: Some(RateWindow {
                kind: WindowKind::Weekly,
                used_percent: 10.0,
                window_seconds: WEEKLY_SECONDS,
                resets_at_unix: UsageSnapshot::now() + 60 * 60 * 24 * 4,
            }),
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
        assert!((loaded.five_hour.as_ref().unwrap().used_percent - 30.0).abs() < 0.0001);
    }

    #[test]
    fn rejects_invalid_percent_on_load() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("usage-cache.json");
        let mut cache = CacheFile {
            schema_version: SCHEMA_VERSION,
            snapshot: CacheSnapshot::from(snap(50.0, UsageSnapshot::now() + 60)),
        };
        cache.snapshot.five_hour.as_mut().unwrap().used_percent = 200.0;
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
        broken.five_hour.as_mut().unwrap().window_seconds = 60;
        assert_eq!(save_to(&path, &broken), Err(AppError::CacheInvalid));
    }

    #[test]
    fn round_trips_weekly_only_snapshot() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("usage-cache.json");
        let now = UsageSnapshot::now();
        let snapshot = UsageSnapshot {
            five_hour: None,
            weekly: Some(RateWindow {
                kind: WindowKind::Weekly,
                used_percent: 30.0,
                window_seconds: WEEKLY_SECONDS,
                resets_at_unix: now + 3 * 86_400,
            }),
            fetched_at_unix: now,
        };
        save_to(&path, &snapshot).expect("save");
        let loaded = load_from(&path).expect("load").expect("snapshot");
        assert!(loaded.five_hour.is_none());
        assert_eq!(loaded.weekly.as_ref().unwrap().used_percent, 30.0);
    }

    #[test]
    fn loads_legacy_two_window_schema() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("usage-cache.json");
        let now = UsageSnapshot::now();
        let body = serde_json::json!({
            "schema_version": LEGACY_SCHEMA_VERSION,
            "snapshot": {
                "five_hour": {
                    "used_percent": 20.0,
                    "resets_at_unix": now + 3600,
                    "window_seconds": FIVE_HOUR_SECONDS
                },
                "weekly": {
                    "used_percent": 30.0,
                    "resets_at_unix": now + 3 * 86_400,
                    "window_seconds": WEEKLY_SECONDS
                },
                "fetched_at_unix": now
            }
        });
        std::fs::write(&path, serde_json::to_string(&body).unwrap()).unwrap();
        let loaded = load_from(&path).expect("load").expect("snapshot");
        assert!(loaded.five_hour.is_some());
        assert!(loaded.weekly.is_some());
    }

    #[test]
    fn round_trip_dynamic_window_durations() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("usage-cache.json");
        let now = UsageSnapshot::now();
        let snapshot = UsageSnapshot {
            five_hour: Some(RateWindow {
                kind: WindowKind::FiveHour,
                used_percent: 12.5,
                window_seconds: 6 * 3600,
                resets_at_unix: now + 6 * 3600,
            }),
            weekly: Some(RateWindow {
                kind: WindowKind::Weekly,
                used_percent: 30.0,
                window_seconds: 8 * 86_400,
                resets_at_unix: now + 8 * 86_400,
            }),
            fetched_at_unix: now,
        };
        save_to(&path, &snapshot).expect("save");
        let loaded = load_from(&path).expect("ok").expect("some");
        assert_eq!(loaded.five_hour.as_ref().unwrap().window_seconds, 6 * 3600);
        assert_eq!(loaded.weekly.as_ref().unwrap().window_seconds, 8 * 86_400);
    }

    #[test]
    fn load_rejects_duration_outside_role_bound() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("usage-cache.json");
        let now = UsageSnapshot::now();
        let mut snapshot = snap(30.0, now + 3600);
        snapshot.five_hour.as_mut().unwrap().window_seconds = 30 * 60; // below PRIMARY_WINDOW_MIN_SECONDS
        let body = serde_json::to_string(&CacheFile {
            schema_version: SCHEMA_VERSION,
            snapshot: CacheSnapshot::from(snapshot),
        })
        .unwrap();
        std::fs::write(&path, body).unwrap();
        let err = load_from(&path).expect_err("invalid");
        assert!(matches!(
            err,
            AppError::WindowDurationMismatch {
                kind: WindowKind::FiveHour,
                ..
            }
        ));
    }
}
