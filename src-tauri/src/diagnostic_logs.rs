use chrono::Utc;
use once_cell::sync::OnceCell;
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use uuid::Uuid;

const SCHEMA_VERSION: i32 = 1;
const QUEUE_CAPACITY: usize = 4096;
const MAX_BATCH_SIZE: usize = 100;
const BATCH_WAIT: Duration = Duration::from_millis(250);
const RETENTION_DAYS: i64 = 3;
const REQUEST_LOG_LIMIT_BYTES: i64 = 300 * 1024 * 1024;
const RUNTIME_LOG_LIMIT_BYTES: i64 = 50 * 1024 * 1024;
const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;
const COMPRESS_THRESHOLD_BYTES: usize = 4 * 1024;

static SERVICE: OnceCell<Arc<DiagnosticLogService>> = OnceCell::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestTraceSummary {
    pub trace_id: String,
    pub usage_request_id: Option<String>,
    pub app_type: String,
    pub method: String,
    pub path: String,
    pub request_model: Option<String>,
    pub response_model: Option<String>,
    pub final_provider_id: Option<String>,
    pub status_code: Option<u16>,
    pub is_streaming: bool,
    pub attempt_count: u32,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub duration_ms: Option<u64>,
    pub outcome: String,
    pub partial: bool,
    pub dropped_event_count: u64,
    pub stored_bytes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceEventRecord {
    pub event_id: i64,
    pub sequence: i64,
    pub occurred_at: i64,
    pub offset_ms: u64,
    pub stage: String,
    pub kind: String,
    pub attempt_no: Option<u32>,
    pub provider_id: Option<String>,
    pub status_code: Option<u16>,
    pub summary: Option<String>,
    pub payload: Option<Value>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestTraceDetail {
    pub trace: RequestTraceSummary,
    pub events: Vec<TraceEventRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeLogRecord {
    pub log_id: i64,
    pub occurred_at: i64,
    pub level: String,
    pub target: String,
    pub message: String,
    pub fields: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticLogHealth {
    pub available: bool,
    pub error: Option<String>,
    pub db_path: String,
    pub retention_days: i64,
    pub request_bytes: i64,
    pub runtime_bytes: i64,
    pub physical_bytes: u64,
    pub dropped_events: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceQuery {
    pub query: Option<String>,
    pub app_type: Option<String>,
    pub provider_id: Option<String>,
    pub status_code: Option<u16>,
    pub streaming: Option<bool>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeLogQuery {
    pub query: Option<String>,
    pub level: Option<String>,
    pub target: Option<String>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct BeginTrace {
    pub app_type: String,
    pub method: String,
    pub path: String,
    pub request_model: Option<String>,
    pub provider_id: Option<String>,
    pub is_streaming: bool,
    pub headers: Value,
    pub body: Value,
}

#[derive(Debug, Clone)]
pub struct TraceEventInput {
    pub trace_id: String,
    pub offset_ms: u64,
    pub stage: String,
    pub kind: String,
    pub attempt_no: Option<u32>,
    pub provider_id: Option<String>,
    pub status_code: Option<u16>,
    pub summary: Option<String>,
    pub payload: Option<Value>,
}

#[derive(Debug, Clone)]
struct CompleteTrace {
    trace_id: String,
    usage_request_id: Option<String>,
    response_model: Option<String>,
    final_provider_id: Option<String>,
    status_code: Option<u16>,
    duration_ms: u64,
    outcome: String,
    dropped_event_count: u64,
}

#[derive(Debug, Clone)]
struct RuntimeLogInput {
    occurred_at: i64,
    level: String,
    target: String,
    message: String,
    fields: Option<Value>,
}

enum WriteCommand {
    Begin { trace_id: String, input: BeginTrace },
    Event(TraceEventInput),
    Complete(CompleteTrace),
    Runtime(RuntimeLogInput),
}

impl WriteCommand {
    fn trace_id(&self) -> Option<&str> {
        match self {
            Self::Begin { trace_id, .. } => Some(trace_id),
            Self::Event(event) => Some(&event.trace_id),
            Self::Complete(complete) => Some(&complete.trace_id),
            Self::Runtime(_) => None,
        }
    }
}

pub struct DiagnosticLogService {
    db_path: PathBuf,
    sender: SyncSender<WriteCommand>,
    dropped_events: AtomicU64,
    dropped_by_trace: Mutex<HashMap<String, u64>>,
    last_error: RwLock<Option<String>>,
}

impl DiagnosticLogService {
    fn start(db_path: PathBuf) -> Result<Arc<Self>, String> {
        let conn = open_write_connection(&db_path)?;
        initialize_schema(&conn)?;
        drop(conn);

        let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
        let service = Arc::new(Self {
            db_path: db_path.clone(),
            sender,
            dropped_events: AtomicU64::new(0),
            dropped_by_trace: Mutex::new(HashMap::new()),
            last_error: RwLock::new(None),
        });
        let worker_service = service.clone();
        std::thread::Builder::new()
            .name("diagnostic-log-writer".to_string())
            .spawn(move || writer_loop(worker_service, receiver))
            .map_err(|error| format!("Failed to start diagnostic log writer: {error}"))?;
        Ok(service)
    }

    fn enqueue(&self, command: WriteCommand) {
        let trace_id = command.trace_id().map(str::to_string);
        match self.sender.try_send(command) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.dropped_events.fetch_add(1, Ordering::Relaxed);
                if let Some(trace_id) = trace_id {
                    if let Ok(mut dropped) = self.dropped_by_trace.lock() {
                        *dropped.entry(trace_id).or_default() += 1;
                    }
                }
            }
            Err(TrySendError::Disconnected(_)) => {
                self.dropped_events.fetch_add(1, Ordering::Relaxed);
                self.set_error("Diagnostic log writer is not running".to_string());
            }
        }
    }

    fn take_dropped_for_trace(&self, trace_id: &str) -> u64 {
        self.dropped_by_trace
            .lock()
            .ok()
            .and_then(|mut values| values.remove(trace_id))
            .unwrap_or(0)
    }

    fn set_error(&self, error: String) {
        if let Ok(mut last_error) = self.last_error.write() {
            *last_error = Some(error);
        }
    }

    fn clear_error(&self) {
        if let Ok(mut last_error) = self.last_error.write() {
            *last_error = None;
        }
    }

    fn open_read_connection(&self) -> Result<Connection, String> {
        let conn = Connection::open(&self.db_path)
            .map_err(|error| format!("Failed to open diagnostic log database: {error}"))?;
        conn.busy_timeout(Duration::from_secs(2))
            .map_err(|error| format!("Failed to configure diagnostic log database: {error}"))?;
        Ok(conn)
    }

    pub fn list_traces(&self, query: &TraceQuery) -> Result<Vec<RequestTraceSummary>, String> {
        let conn = self.open_read_connection()?;
        let limit = query.limit.unwrap_or(100).clamp(1, 200);
        let offset = query.offset.unwrap_or(0);
        let search = query.query.as_deref().map(|value| format!("%{}%", value.trim()));
        let mut stmt = conn
            .prepare(
                "SELECT trace_id, usage_request_id, app_type, method, path,
                        request_model, response_model, final_provider_id, status_code,
                        is_streaming, attempt_count, started_at, completed_at, duration_ms,
                        outcome, partial, dropped_event_count, stored_bytes
                 FROM request_traces
                 WHERE (?1 IS NULL OR app_type = ?1)
                   AND (?2 IS NULL OR final_provider_id = ?2)
                   AND (?3 IS NULL OR status_code = ?3)
                   AND (?4 IS NULL OR is_streaming = ?4)
                   AND (?5 IS NULL OR trace_id LIKE ?5 OR request_model LIKE ?5
                        OR response_model LIKE ?5 OR path LIKE ?5 OR final_provider_id LIKE ?5)
                 ORDER BY started_at DESC, trace_id DESC
                 LIMIT ?6 OFFSET ?7",
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(
                params![
                    query.app_type.as_deref(),
                    query.provider_id.as_deref(),
                    query.status_code.map(i64::from),
                    query.streaming.map(bool_to_i64),
                    search,
                    i64::from(limit),
                    i64::from(offset),
                ],
                map_trace_summary,
            )
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn get_trace(&self, trace_id: &str) -> Result<Option<RequestTraceDetail>, String> {
        let conn = self.open_read_connection()?;
        let trace = conn
            .query_row(
                "SELECT trace_id, usage_request_id, app_type, method, path,
                        request_model, response_model, final_provider_id, status_code,
                        is_streaming, attempt_count, started_at, completed_at, duration_ms,
                        outcome, partial, dropped_event_count, stored_bytes
                 FROM request_traces WHERE trace_id = ?1",
                [trace_id],
                map_trace_summary,
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some(trace) = trace else {
            return Ok(None);
        };

        let mut stmt = conn
            .prepare(
                "SELECT e.event_id, e.sequence, e.occurred_at, e.offset_ms, e.stage,
                        e.kind, e.attempt_no, e.provider_id, e.status_code, e.summary,
                        p.encoding, p.body, COALESCE(p.truncated, 0)
                 FROM trace_events e
                 LEFT JOIN trace_payloads p ON p.payload_id = e.payload_id
                 WHERE e.trace_id = ?1 ORDER BY e.sequence ASC",
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([trace_id], |row| {
                let encoding: Option<String> = row.get(10)?;
                let body: Option<Vec<u8>> = row.get(11)?;
                let payload = match (encoding.as_deref(), body) {
                    (Some(encoding), Some(body)) => decode_payload(encoding, &body),
                    _ => None,
                };
                Ok(TraceEventRecord {
                    event_id: row.get(0)?,
                    sequence: row.get(1)?,
                    occurred_at: row.get(2)?,
                    offset_ms: row.get::<_, i64>(3)?.max(0) as u64,
                    stage: row.get(4)?,
                    kind: row.get(5)?,
                    attempt_no: row.get::<_, Option<i64>>(6)?.map(|value| value.max(0) as u32),
                    provider_id: row.get(7)?,
                    status_code: row.get::<_, Option<i64>>(8)?.map(|value| value as u16),
                    summary: row.get(9)?,
                    payload,
                    truncated: row.get::<_, i64>(12)? != 0,
                })
            })
            .map_err(|error| error.to_string())?;
        let events = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(Some(RequestTraceDetail { trace, events }))
    }

    pub fn list_runtime_logs(
        &self,
        query: &RuntimeLogQuery,
    ) -> Result<Vec<RuntimeLogRecord>, String> {
        let conn = self.open_read_connection()?;
        let limit = query.limit.unwrap_or(200).clamp(1, 500);
        let offset = query.offset.unwrap_or(0);
        let search = query.query.as_deref().map(|value| format!("%{}%", value.trim()));
        let mut stmt = conn
            .prepare(
                "SELECT log_id, occurred_at, level, target, message, fields_json
                 FROM runtime_logs
                 WHERE (?1 IS NULL OR level = ?1)
                   AND (?2 IS NULL OR target LIKE ?2)
                   AND (?3 IS NULL OR message LIKE ?3 OR target LIKE ?3)
                 ORDER BY occurred_at DESC, log_id DESC
                 LIMIT ?4 OFFSET ?5",
            )
            .map_err(|error| error.to_string())?;
        let target = query.target.as_deref().map(|value| format!("%{}%", value.trim()));
        let rows = stmt
            .query_map(
                params![query.level.as_deref(), target, search, i64::from(limit), i64::from(offset)],
                |row| {
                    let fields_json: Option<String> = row.get(5)?;
                    Ok(RuntimeLogRecord {
                        log_id: row.get(0)?,
                        occurred_at: row.get(1)?,
                        level: row.get(2)?,
                        target: row.get(3)?,
                        message: row.get(4)?,
                        fields: fields_json.and_then(|value| serde_json::from_str(&value).ok()),
                    })
                },
            )
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn health(&self) -> DiagnosticLogHealth {
        let error = self.last_error.read().ok().and_then(|value| value.clone());
        let (request_bytes, runtime_bytes) = self
            .open_read_connection()
            .and_then(|conn| {
                conn.query_row(
                    "SELECT
                        COALESCE((SELECT SUM(stored_bytes) FROM request_traces), 0),
                        COALESCE((SELECT SUM(stored_bytes) FROM runtime_logs), 0)",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|query_error| query_error.to_string())
            })
            .unwrap_or((0, 0));
        let physical_bytes = std::fs::metadata(&self.db_path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        DiagnosticLogHealth {
            available: error.is_none(),
            error,
            db_path: self.db_path.to_string_lossy().into_owned(),
            retention_days: RETENTION_DAYS,
            request_bytes,
            runtime_bytes,
            physical_bytes,
            dropped_events: self.dropped_events.load(Ordering::Relaxed),
        }
    }

    pub fn clear(&self, kind: &str) -> Result<(), String> {
        let mut conn = open_write_connection(&self.db_path)?;
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        match kind {
            "requests" => {
                tx.execute("DELETE FROM request_traces", [])
                    .map_err(|error| error.to_string())?;
            }
            "runtime" => {
                tx.execute("DELETE FROM runtime_logs", [])
                    .map_err(|error| error.to_string())?;
            }
            "all" => {
                tx.execute("DELETE FROM request_traces", [])
                    .map_err(|error| error.to_string())?;
                tx.execute("DELETE FROM runtime_logs", [])
                    .map_err(|error| error.to_string())?;
            }
            _ => return Err("Unsupported diagnostic log category".to_string()),
        }
        tx.execute(
            "DELETE FROM trace_payloads
             WHERE payload_id NOT IN (SELECT payload_id FROM trace_events WHERE payload_id IS NOT NULL)",
            [],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE); PRAGMA incremental_vacuum;")
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

pub fn init() -> Result<Arc<DiagnosticLogService>, String> {
    if let Some(service) = SERVICE.get() {
        return Ok(service.clone());
    }
    let path = crate::config::get_app_config_dir().join("cc-switch-logs.db");
    let service = DiagnosticLogService::start(path)?;
    SERVICE
        .set(service.clone())
        .map_err(|_| "Diagnostic log service was initialized concurrently".to_string())?;
    Ok(service)
}

pub fn service() -> Option<&'static Arc<DiagnosticLogService>> {
    SERVICE.get()
}

pub fn begin_trace(mut input: BeginTrace) -> Option<String> {
    let service = service()?;
    input.headers = redact_value(&input.headers);
    input.body = redact_value(&input.body);
    let trace_id = Uuid::new_v4().to_string();
    service.enqueue(WriteCommand::Begin {
        trace_id: trace_id.clone(),
        input,
    });
    Some(trace_id)
}

pub fn record_trace_event(mut input: TraceEventInput) {
    if let Some(service) = service() {
        input.summary = input.summary.map(|summary| redact_text(&summary));
        input.payload = input.payload.map(|payload| redact_value(&payload));
        service.enqueue(WriteCommand::Event(input));
    }
}

#[allow(clippy::too_many_arguments)]
pub fn complete_trace(
    trace_id: &str,
    usage_request_id: Option<String>,
    response_model: Option<String>,
    final_provider_id: Option<String>,
    status_code: Option<u16>,
    duration_ms: u64,
    outcome: &str,
) {
    if let Some(service) = service() {
        let dropped_event_count = service.take_dropped_for_trace(trace_id);
        service.enqueue(WriteCommand::Complete(CompleteTrace {
            trace_id: trace_id.to_string(),
            usage_request_id,
            response_model,
            final_provider_id,
            status_code,
            duration_ms,
            outcome: outcome.to_string(),
            dropped_event_count,
        }));
    }
}

pub fn record_runtime_log(level: &str, target: &str, message: &str, fields: Option<Value>) {
    if let Some(service) = service() {
        service.enqueue(WriteCommand::Runtime(RuntimeLogInput {
            occurred_at: Utc::now().timestamp_millis(),
            level: level.to_ascii_lowercase(),
            target: target.to_string(),
            message: redact_text(message),
            fields: fields.map(|value| redact_value(&value)),
        }));
    }
}

pub fn redact_headers(headers: &http::HeaderMap) -> Value {
    let mut output = Map::new();
    for (name, value) in headers {
        let name = name.as_str();
        let rendered = if is_sensitive_key(name) {
            "[REDACTED]".to_string()
        } else {
            value
                .to_str()
                .map(|value| truncate_text(&redact_text(value), 512).0)
                .unwrap_or_else(|_| "[BINARY HEADER]".to_string())
        };
        output.insert(name.to_string(), Value::String(rendered));
    }
    Value::Object(output)
}

pub fn redact_value(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut output = Map::new();
            for (key, value) in object {
                if is_sensitive_key(key) {
                    output.insert(key.clone(), Value::String("[REDACTED]".to_string()));
                } else {
                    output.insert(key.clone(), redact_value(value));
                }
            }
            Value::Object(output)
        }
        Value::Array(values) => Value::Array(values.iter().map(redact_value).collect()),
        Value::String(value) if is_large_media_value(value) => json!({
            "omitted": true,
            "kind": "media-or-base64",
            "originalBytes": value.len(),
            "sha256": sha256_hex(value.as_bytes()),
        }),
        Value::String(value) => Value::String(redact_text(value)),
        _ => value.clone(),
    }
}

fn writer_loop(service: Arc<DiagnosticLogService>, receiver: Receiver<WriteCommand>) {
    let mut conn = match open_write_connection(&service.db_path).and_then(|conn| {
        initialize_schema(&conn)?;
        Ok(conn)
    }) {
        Ok(conn) => conn,
        Err(error) => {
            service.set_error(error);
            return;
        }
    };
    let mut last_maintenance = Instant::now() - Duration::from_secs(3600);

    loop {
        let first = match receiver.recv_timeout(BATCH_WAIT) {
            Ok(command) => command,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if last_maintenance.elapsed() >= Duration::from_secs(300) {
                    if let Err(error) = run_maintenance(&mut conn) {
                        service.set_error(error);
                    }
                    last_maintenance = Instant::now();
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let mut batch = Vec::with_capacity(MAX_BATCH_SIZE);
        batch.push(first);
        while batch.len() < MAX_BATCH_SIZE {
            match receiver.try_recv() {
                Ok(command) => batch.push(command),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }

        match write_batch(&mut conn, batch) {
            Ok(()) => service.clear_error(),
            Err(error) => service.set_error(error),
        }
        if last_maintenance.elapsed() >= Duration::from_secs(300) {
            if let Err(error) = run_maintenance(&mut conn) {
                service.set_error(error);
            }
            last_maintenance = Instant::now();
        }
    }
}

fn open_write_connection(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create diagnostic log directory: {error}"))?;
    }
    let conn = Connection::open(path)
        .map_err(|error| format!("Failed to open diagnostic log database: {error}"))?;
    conn.busy_timeout(Duration::from_secs(2))
        .map_err(|error| format!("Failed to configure diagnostic log database: {error}"))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA auto_vacuum = INCREMENTAL;",
    )
    .map_err(|error| format!("Failed to configure diagnostic log database: {error}"))?;
    Ok(conn)
}

fn initialize_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS request_traces (
            trace_id TEXT PRIMARY KEY,
            usage_request_id TEXT,
            app_type TEXT NOT NULL,
            method TEXT NOT NULL,
            path TEXT NOT NULL,
            request_model TEXT,
            response_model TEXT,
            final_provider_id TEXT,
            status_code INTEGER,
            is_streaming INTEGER NOT NULL DEFAULT 0,
            attempt_count INTEGER NOT NULL DEFAULT 0,
            started_at INTEGER NOT NULL,
            completed_at INTEGER,
            duration_ms INTEGER,
            outcome TEXT NOT NULL DEFAULT 'in_progress',
            partial INTEGER NOT NULL DEFAULT 0,
            dropped_event_count INTEGER NOT NULL DEFAULT 0,
            stored_bytes INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_request_traces_started
            ON request_traces(started_at DESC, trace_id DESC);
        CREATE INDEX IF NOT EXISTS idx_request_traces_app
            ON request_traces(app_type, started_at DESC);
        CREATE INDEX IF NOT EXISTS idx_request_traces_provider
            ON request_traces(final_provider_id, started_at DESC);
        CREATE INDEX IF NOT EXISTS idx_request_traces_status
            ON request_traces(status_code, started_at DESC);
        CREATE INDEX IF NOT EXISTS idx_request_traces_usage
            ON request_traces(usage_request_id);
        CREATE TABLE IF NOT EXISTS trace_payloads (
            payload_id INTEGER PRIMARY KEY AUTOINCREMENT,
            content_type TEXT NOT NULL,
            encoding TEXT NOT NULL,
            body BLOB NOT NULL,
            original_bytes INTEGER NOT NULL,
            stored_bytes INTEGER NOT NULL,
            sha256 TEXT NOT NULL,
            truncated INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS trace_events (
            event_id INTEGER PRIMARY KEY AUTOINCREMENT,
            trace_id TEXT NOT NULL REFERENCES request_traces(trace_id) ON DELETE CASCADE,
            sequence INTEGER NOT NULL,
            occurred_at INTEGER NOT NULL,
            offset_ms INTEGER NOT NULL,
            stage TEXT NOT NULL,
            kind TEXT NOT NULL,
            attempt_no INTEGER,
            provider_id TEXT,
            status_code INTEGER,
            summary TEXT,
            payload_id INTEGER REFERENCES trace_payloads(payload_id) ON DELETE SET NULL,
            UNIQUE(trace_id, sequence)
        );
        CREATE INDEX IF NOT EXISTS idx_trace_events_trace
            ON trace_events(trace_id, sequence);
        CREATE TABLE IF NOT EXISTS runtime_logs (
            log_id INTEGER PRIMARY KEY AUTOINCREMENT,
            occurred_at INTEGER NOT NULL,
            level TEXT NOT NULL,
            target TEXT NOT NULL,
            message TEXT NOT NULL,
            fields_json TEXT,
            stored_bytes INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_runtime_logs_time
            ON runtime_logs(occurred_at DESC, log_id DESC);
        CREATE INDEX IF NOT EXISTS idx_runtime_logs_level
            ON runtime_logs(level, occurred_at DESC);
        CREATE INDEX IF NOT EXISTS idx_runtime_logs_target
            ON runtime_logs(target, occurred_at DESC);",
    )
    .map_err(|error| format!("Failed to initialize diagnostic log schema: {error}"))?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(|error| format!("Failed to update diagnostic log schema version: {error}"))?;
    Ok(())
}

fn write_batch(conn: &mut Connection, batch: Vec<WriteCommand>) -> Result<(), String> {
    let tx = conn.transaction().map_err(|error| error.to_string())?;
    for command in batch {
        match command {
            WriteCommand::Begin { trace_id, input } => {
                let started_at = Utc::now().timestamp_millis();
                let payload = json!({
                    "headers": redact_value(&input.headers),
                    "body": redact_value(&input.body),
                });
                tx.execute(
                    "INSERT OR IGNORE INTO request_traces (
                        trace_id, app_type, method, path, request_model, final_provider_id,
                        is_streaming, started_at, stored_bytes
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        trace_id,
                        input.app_type,
                        input.method,
                        input.path,
                        input.request_model,
                        input.provider_id,
                        bool_to_i64(input.is_streaming),
                        started_at,
                        0_i64,
                    ],
                )
                .map_err(|error| error.to_string())?;
                insert_event(
                    &tx,
                    &TraceEventInput {
                        trace_id,
                        offset_ms: 0,
                        stage: "client_request".to_string(),
                        kind: "request".to_string(),
                        attempt_no: None,
                        provider_id: None,
                        status_code: None,
                        summary: Some("Client request received".to_string()),
                        payload: Some(payload),
                    },
                )?;
            }
            WriteCommand::Event(event) => insert_event(&tx, &event)?,
            WriteCommand::Complete(complete) => {
                let completed_at = Utc::now().timestamp_millis();
                insert_event(
                    &tx,
                    &TraceEventInput {
                        trace_id: complete.trace_id.clone(),
                        offset_ms: complete.duration_ms,
                        stage: "complete".to_string(),
                        kind: complete.outcome.clone(),
                        attempt_no: None,
                        provider_id: complete.final_provider_id.clone(),
                        status_code: complete.status_code,
                        summary: Some(format!("Request {}", complete.outcome)),
                        payload: None,
                    },
                )?;
                tx.execute(
                    "UPDATE request_traces SET
                        usage_request_id = COALESCE(?2, usage_request_id),
                        response_model = COALESCE(?3, response_model),
                        final_provider_id = COALESCE(?4, final_provider_id),
                        status_code = COALESCE(?5, status_code),
                        completed_at = ?6,
                        duration_ms = ?7,
                        outcome = ?8,
                        partial = CASE WHEN ?9 > 0 THEN 1 ELSE partial END,
                        dropped_event_count = dropped_event_count + ?9
                     WHERE trace_id = ?1",
                    params![
                        complete.trace_id,
                        complete.usage_request_id,
                        complete.response_model,
                        complete.final_provider_id,
                        complete.status_code.map(i64::from),
                        completed_at,
                        complete.duration_ms as i64,
                        complete.outcome,
                        complete.dropped_event_count as i64,
                    ],
                )
                .map_err(|error| error.to_string())?;
            }
            WriteCommand::Runtime(input) => {
                let fields_json = input
                    .fields
                    .as_ref()
                    .and_then(|fields| serde_json::to_string(fields).ok());
                let stored_bytes = input.message.len()
                    + input.target.len()
                    + fields_json.as_ref().map(String::len).unwrap_or(0);
                tx.execute(
                    "INSERT INTO runtime_logs (
                        occurred_at, level, target, message, fields_json, stored_bytes
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        input.occurred_at,
                        input.level,
                        input.target,
                        input.message,
                        fields_json,
                        stored_bytes as i64,
                    ],
                )
                .map_err(|error| error.to_string())?;
            }
        }
    }
    tx.commit().map_err(|error| error.to_string())
}

fn insert_event(tx: &rusqlite::Transaction<'_>, event: &TraceEventInput) -> Result<(), String> {
    let trace_exists = tx
        .query_row(
            "SELECT 1 FROM request_traces WHERE trace_id = ?1",
            [&event.trace_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .is_some();
    if !trace_exists {
        return Ok(());
    }

    let (payload_id, payload_bytes) = match &event.payload {
        Some(payload) => {
            let (payload_id, stored_bytes) = insert_payload(tx, payload)?;
            (Some(payload_id), stored_bytes)
        }
        None => (None, 0),
    };
    let sequence: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM trace_events WHERE trace_id = ?1",
            [&event.trace_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let occurred_at = Utc::now().timestamp_millis();
    let summary_bytes = event.summary.as_ref().map(String::len).unwrap_or(0) as i64;
    tx.execute(
        "INSERT INTO trace_events (
            trace_id, sequence, occurred_at, offset_ms, stage, kind, attempt_no,
            provider_id, status_code, summary, payload_id
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            event.trace_id.as_str(),
            sequence,
            occurred_at,
            event.offset_ms as i64,
            event.stage.as_str(),
            event.kind.as_str(),
            event.attempt_no.map(i64::from),
            event.provider_id.as_deref(),
            event.status_code.map(i64::from),
            event.summary.as_deref(),
            payload_id,
        ],
    )
    .map_err(|error| error.to_string())?;
    tx.execute(
        "UPDATE request_traces SET
            stored_bytes = stored_bytes + ?2,
            attempt_count = MAX(attempt_count, COALESCE(?3, attempt_count))
         WHERE trace_id = ?1",
        params![event.trace_id.as_str(), payload_bytes + summary_bytes, event.attempt_no.map(i64::from)],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn insert_payload(tx: &rusqlite::Transaction<'_>, payload: &Value) -> Result<(i64, i64), String> {
    let serialized = serde_json::to_vec(&redact_value(payload)).map_err(|error| error.to_string())?;
    let original_bytes = serialized.len();
    let hash = sha256_hex(&serialized);
    let (bounded, truncated) = if original_bytes > MAX_PAYLOAD_BYTES {
        let preview = String::from_utf8_lossy(&serialized[..MAX_PAYLOAD_BYTES]);
        let (preview, _) = truncate_text(&preview, MAX_PAYLOAD_BYTES.saturating_sub(256));
        let placeholder = json!({
            "truncated": true,
            "originalBytes": original_bytes,
            "sha256": hash,
            "preview": preview,
        });
        (
            serde_json::to_vec(&placeholder).map_err(|error| error.to_string())?,
            true,
        )
    } else {
        (serialized, false)
    };
    let (encoding, body) = if bounded.len() >= COMPRESS_THRESHOLD_BYTES {
        match zstd::stream::encode_all(Cursor::new(&bounded), 1) {
            Ok(compressed) if compressed.len() < bounded.len() => ("zstd", compressed),
            _ => ("identity", bounded),
        }
    } else {
        ("identity", bounded)
    };
    let stored_bytes = body.len() as i64;
    tx.execute(
        "INSERT INTO trace_payloads (
            content_type, encoding, body, original_bytes, stored_bytes, sha256, truncated
         ) VALUES ('application/json', ?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            encoding,
            body,
            original_bytes as i64,
            stored_bytes,
            hash,
            bool_to_i64(truncated),
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok((tx.last_insert_rowid(), stored_bytes))
}

fn run_maintenance(conn: &mut Connection) -> Result<(), String> {
    let cutoff = Utc::now().timestamp_millis() - RETENTION_DAYS * 24 * 60 * 60 * 1000;
    let tx = conn.transaction().map_err(|error| error.to_string())?;
    tx.execute("DELETE FROM request_traces WHERE started_at < ?1", [cutoff])
        .map_err(|error| error.to_string())?;
    tx.execute("DELETE FROM runtime_logs WHERE occurred_at < ?1", [cutoff])
        .map_err(|error| error.to_string())?;
    trim_trace_capacity(&tx)?;
    trim_runtime_capacity(&tx)?;
    tx.execute(
        "DELETE FROM trace_payloads
         WHERE payload_id NOT IN (SELECT payload_id FROM trace_events WHERE payload_id IS NOT NULL)",
        [],
    )
    .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())?;
    conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE); PRAGMA incremental_vacuum(1000);")
        .map_err(|error| error.to_string())
}

fn trim_trace_capacity(tx: &rusqlite::Transaction<'_>) -> Result<(), String> {
    loop {
        let total: i64 = tx
            .query_row("SELECT COALESCE(SUM(stored_bytes), 0) FROM request_traces", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if total <= REQUEST_LOG_LIMIT_BYTES {
            return Ok(());
        }
        let deleted = tx
            .execute(
                "DELETE FROM request_traces WHERE trace_id IN (
                    SELECT trace_id FROM request_traces ORDER BY started_at ASC LIMIT 100
                 )",
                [],
            )
            .map_err(|error| error.to_string())?;
        if deleted == 0 {
            return Ok(());
        }
    }
}

fn trim_runtime_capacity(tx: &rusqlite::Transaction<'_>) -> Result<(), String> {
    loop {
        let total: i64 = tx
            .query_row("SELECT COALESCE(SUM(stored_bytes), 0) FROM runtime_logs", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if total <= RUNTIME_LOG_LIMIT_BYTES {
            return Ok(());
        }
        let deleted = tx
            .execute(
                "DELETE FROM runtime_logs WHERE log_id IN (
                    SELECT log_id FROM runtime_logs ORDER BY occurred_at ASC LIMIT 500
                 )",
                [],
            )
            .map_err(|error| error.to_string())?;
        if deleted == 0 {
            return Ok(());
        }
    }
}

fn map_trace_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<RequestTraceSummary> {
    Ok(RequestTraceSummary {
        trace_id: row.get(0)?,
        usage_request_id: row.get(1)?,
        app_type: row.get(2)?,
        method: row.get(3)?,
        path: row.get(4)?,
        request_model: row.get(5)?,
        response_model: row.get(6)?,
        final_provider_id: row.get(7)?,
        status_code: row.get::<_, Option<i64>>(8)?.map(|value| value as u16),
        is_streaming: row.get::<_, i64>(9)? != 0,
        attempt_count: row.get::<_, i64>(10)?.max(0) as u32,
        started_at: row.get(11)?,
        completed_at: row.get(12)?,
        duration_ms: row.get::<_, Option<i64>>(13)?.map(|value| value.max(0) as u64),
        outcome: row.get(14)?,
        partial: row.get::<_, i64>(15)? != 0,
        dropped_event_count: row.get::<_, i64>(16)?.max(0) as u64,
        stored_bytes: row.get(17)?,
    })
}

fn decode_payload(encoding: &str, body: &[u8]) -> Option<Value> {
    let decoded = match encoding {
        "zstd" => zstd::stream::decode_all(Cursor::new(body)).ok()?,
        _ => body.to_vec(),
    };
    serde_json::from_slice(&decoded).ok()
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    matches!(
        normalized.trim_end_matches('s'),
        "key"
            | "apikey"
            | "accesskey"
            | "secretkey"
            | "privatekey"
            | "clientsecret"
            | "token"
            | "authtoken"
            | "accesstoken"
            | "refreshtoken"
            | "idtoken"
            | "sessiontoken"
            | "sessionid"
            | "authorization"
            | "proxyauthorization"
            | "auth"
            | "bearer"
            | "password"
            | "passwd"
            | "pwd"
            | "secret"
            | "credential"
            | "cookie"
            | "setcookie"
    )
}

fn is_large_media_value(value: &str) -> bool {
    if value.starts_with("data:") {
        return true;
    }
    value.len() >= 4096
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=' | b'-' | b'_'))
}

fn redact_text(value: &str) -> String {
    static AUTH: OnceCell<Regex> = OnceCell::new();
    static NAMED: OnceCell<Regex> = OnceCell::new();
    let auth = AUTH.get_or_init(|| {
        Regex::new(r#"(?i)\b(Bearer|Basic|Token|ApiKey)\s+[^\s"',}\]]+"#)
            .expect("valid auth redaction regex")
    });
    let named = NAMED.get_or_init(|| {
        Regex::new(
            r#"(?i)((?:api[_-]?key|access[_-]?token|refresh[_-]?token|password|secret|cookie)\s*[:=]\s*["']?)[^\s"',}]+"#,
        )
        .expect("valid named secret redaction regex")
    });
    let redacted = auth.replace_all(value, "$1 [REDACTED]");
    named.replace_all(&redacted, "$1[REDACTED]").into_owned()
}

fn truncate_text(value: &str, max_bytes: usize) -> (String, bool) {
    if value.len() <= max_bytes {
        return (value.to_string(), false);
    }
    let mut boundary = max_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    (format!("{}\n[truncated]", &value[..boundary]), true)
}

fn sha256_hex(value: &[u8]) -> String {
    let digest = Sha256::digest(value);
    format!("{digest:x}")
}

fn bool_to_i64(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_nested_secrets_and_large_media() {
        let value = json!({
            "authorization": "Bearer secret",
            "nested": { "apiKey": "sk-test-secret" },
            "image": format!("data:image/png;base64,{}", "A".repeat(5000)),
            "message": "Bearer visible-secret",
        });
        let redacted = redact_value(&value);
        assert_eq!(redacted["authorization"], "[REDACTED]");
        assert_eq!(redacted["nested"]["apiKey"], "[REDACTED]");
        assert_eq!(redacted["image"]["omitted"], true);
        assert_eq!(redacted["message"], "Bearer [REDACTED]");
    }

    #[test]
    fn schema_supports_trace_round_trip() {
        let conn = Connection::open_in_memory().unwrap();
        initialize_schema(&conn).unwrap();
        let table_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN
                    ('request_traces', 'trace_events', 'trace_payloads', 'runtime_logs')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 4);
    }
}
