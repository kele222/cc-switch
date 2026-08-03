export type DiagnosticLogKind = "requests" | "runtime" | "all";

export interface RequestTraceSummary {
  traceId: string;
  usageRequestId: string | null;
  appType: string;
  method: string;
  path: string;
  requestModel: string | null;
  responseModel: string | null;
  finalProviderId: string | null;
  statusCode: number | null;
  isStreaming: boolean;
  attemptCount: number;
  startedAt: number;
  completedAt: number | null;
  durationMs: number | null;
  outcome: string;
  partial: boolean;
  droppedEventCount: number;
  storedBytes: number;
}

export interface TraceEventRecord {
  eventId: number;
  sequence: number;
  occurredAt: number;
  offsetMs: number;
  stage: string;
  kind: string;
  attemptNo: number | null;
  providerId: string | null;
  statusCode: number | null;
  summary: string | null;
  payload: unknown | null;
  truncated: boolean;
}

export interface RequestTraceDetail {
  trace: RequestTraceSummary;
  events: TraceEventRecord[];
}

export interface RuntimeLogRecord {
  logId: number;
  occurredAt: number;
  level: string;
  target: string;
  message: string;
  fields: unknown | null;
}

export interface DiagnosticLogHealth {
  available: boolean;
  error: string | null;
  dbPath: string;
  retentionDays: number;
  requestBytes: number;
  runtimeBytes: number;
  physicalBytes: number;
  droppedEvents: number;
}

export interface TraceQuery {
  query?: string;
  appType?: string;
  providerId?: string;
  statusCode?: number;
  streaming?: boolean;
  offset?: number;
  limit?: number;
}

export interface RuntimeLogQuery {
  query?: string;
  level?: string;
  target?: string;
  offset?: number;
  limit?: number;
}
