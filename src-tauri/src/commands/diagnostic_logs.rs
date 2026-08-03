use crate::diagnostic_logs::{
    DiagnosticLogHealth, RequestTraceDetail, RequestTraceSummary, RuntimeLogQuery,
    RuntimeLogRecord, TraceQuery,
};

fn service() -> Result<&'static std::sync::Arc<crate::diagnostic_logs::DiagnosticLogService>, String>
{
    crate::diagnostic_logs::service()
        .ok_or_else(|| "Diagnostic log service is unavailable".to_string())
}

#[tauri::command]
pub async fn get_diagnostic_request_traces(
    query: TraceQuery,
) -> Result<Vec<RequestTraceSummary>, String> {
    service()?.list_traces(&query)
}

#[tauri::command]
pub async fn get_diagnostic_trace(
    trace_id: String,
) -> Result<Option<RequestTraceDetail>, String> {
    service()?.get_trace(&trace_id)
}

#[tauri::command]
pub async fn get_diagnostic_runtime_logs(
    query: RuntimeLogQuery,
) -> Result<Vec<RuntimeLogRecord>, String> {
    service()?.list_runtime_logs(&query)
}

#[tauri::command]
pub async fn get_diagnostic_log_health() -> Result<DiagnosticLogHealth, String> {
    Ok(service()?.health())
}

#[tauri::command]
pub async fn clear_diagnostic_logs(kind: String) -> Result<(), String> {
    service()?.clear(&kind)
}

#[tauri::command]
pub async fn export_diagnostic_trace(trace_id: String) -> Result<String, String> {
    let detail = service()?
        .get_trace(&trace_id)?
        .ok_or_else(|| "Diagnostic trace was not found".to_string())?;
    serde_json::to_string_pretty(&detail).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn record_frontend_diagnostic_log(
    level: String,
    target: String,
    message: String,
) -> Result<(), String> {
    crate::diagnostic_logs::record_runtime_log(&level, &target, &message, None);
    Ok(())
}
