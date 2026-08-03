import { invoke } from "@tauri-apps/api/core";
import type {
  DiagnosticLogHealth,
  DiagnosticLogKind,
  RequestTraceDetail,
  RequestTraceSummary,
  RuntimeLogQuery,
  RuntimeLogRecord,
  TraceQuery,
} from "@/types/diagnosticLogs";

export const diagnosticLogsApi = {
  getRequestTraces: (query: TraceQuery): Promise<RequestTraceSummary[]> =>
    invoke("get_diagnostic_request_traces", { query }),
  getTrace: (traceId: string): Promise<RequestTraceDetail | null> =>
    invoke("get_diagnostic_trace", { traceId }),
  getRuntimeLogs: (query: RuntimeLogQuery): Promise<RuntimeLogRecord[]> =>
    invoke("get_diagnostic_runtime_logs", { query }),
  getHealth: (): Promise<DiagnosticLogHealth> =>
    invoke("get_diagnostic_log_health"),
  clear: (kind: DiagnosticLogKind): Promise<void> =>
    invoke("clear_diagnostic_logs", { kind }),
  exportTrace: (traceId: string): Promise<string> =>
    invoke("export_diagnostic_trace", { traceId }),
  recordFrontendLog: (
    level: string,
    target: string,
    message: string,
  ): Promise<void> =>
    invoke("record_frontend_diagnostic_log", { level, target, message }),
};
