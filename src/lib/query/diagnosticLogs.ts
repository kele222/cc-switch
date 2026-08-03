import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { diagnosticLogsApi } from "@/lib/api/diagnosticLogs";
import type {
  DiagnosticLogKind,
  RuntimeLogQuery,
  TraceQuery,
} from "@/types/diagnosticLogs";

export const diagnosticLogKeys = {
  all: ["diagnostic-logs"] as const,
  traces: (query: TraceQuery) =>
    [...diagnosticLogKeys.all, "traces", query] as const,
  trace: (traceId: string) =>
    [...diagnosticLogKeys.all, "trace", traceId] as const,
  runtime: (query: RuntimeLogQuery) =>
    [...diagnosticLogKeys.all, "runtime", query] as const,
  health: () => [...diagnosticLogKeys.all, "health"] as const,
};

type LiveQueryOptions = { live?: boolean };

export function useDiagnosticTraces(
  query: TraceQuery,
  options: LiveQueryOptions = {},
) {
  return useQuery({
    queryKey: diagnosticLogKeys.traces(query),
    queryFn: () => diagnosticLogsApi.getRequestTraces(query),
    refetchInterval: options.live ? 1000 : false,
    refetchIntervalInBackground: false,
  });
}

export function useDiagnosticTrace(
  traceId: string | null,
  options: LiveQueryOptions = {},
) {
  return useQuery({
    queryKey: diagnosticLogKeys.trace(traceId ?? ""),
    queryFn: () => diagnosticLogsApi.getTrace(traceId ?? ""),
    enabled: Boolean(traceId),
    refetchInterval: options.live ? 1000 : false,
    refetchIntervalInBackground: false,
  });
}

export function useDiagnosticRuntimeLogs(
  query: RuntimeLogQuery,
  options: LiveQueryOptions = {},
) {
  return useQuery({
    queryKey: diagnosticLogKeys.runtime(query),
    queryFn: () => diagnosticLogsApi.getRuntimeLogs(query),
    refetchInterval: options.live ? 1000 : false,
    refetchIntervalInBackground: false,
  });
}

export function useDiagnosticLogHealth(options: LiveQueryOptions = {}) {
  return useQuery({
    queryKey: diagnosticLogKeys.health(),
    queryFn: diagnosticLogsApi.getHealth,
    refetchInterval: options.live ? 5000 : false,
    refetchIntervalInBackground: false,
  });
}

export function useClearDiagnosticLogs() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (kind: DiagnosticLogKind) => diagnosticLogsApi.clear(kind),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: diagnosticLogKeys.all }),
  });
}
