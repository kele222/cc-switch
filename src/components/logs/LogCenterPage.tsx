import { useEffect, useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  AlertCircle,
  ArrowLeft,
  CheckCircle2,
  Circle,
  Copy,
  Download,
  Loader2,
  Pause,
  Play,
  RefreshCw,
  Search,
  Trash2,
  Waves,
} from "lucide-react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { diagnosticLogsApi } from "@/lib/api/diagnosticLogs";
import {
  diagnosticLogKeys,
  useClearDiagnosticLogs,
  useDiagnosticLogHealth,
  useDiagnosticRuntimeLogs,
  useDiagnosticTrace,
  useDiagnosticTraces,
} from "@/lib/query/diagnosticLogs";
import { cn } from "@/lib/utils";
import type {
  DiagnosticLogKind,
  RequestTraceSummary,
  RuntimeLogQuery,
  TraceEventRecord,
  TraceQuery,
} from "@/types/diagnosticLogs";

type LogTab = "requests" | "runtime";

const selectClassName =
  "h-9 rounded-md border border-input bg-background px-3 text-sm text-foreground outline-none focus:ring-1 focus:ring-ring";

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MiB`;
}

function formatDuration(durationMs: number | null): string {
  if (durationMs == null) return "-";
  if (durationMs < 1000) return `${durationMs} ms`;
  return `${(durationMs / 1000).toFixed(2)} s`;
}

function formatTimestamp(timestamp: number): string {
  return new Date(timestamp).toLocaleString();
}

function outcomeClass(outcome: string): string {
  if (outcome === "success") return "text-emerald-600 dark:text-emerald-400";
  if (outcome === "error") return "text-red-600 dark:text-red-400";
  return "text-amber-600 dark:text-amber-400";
}

function levelClass(level: string): string {
  switch (level.toLowerCase()) {
    case "error":
      return "bg-red-100 text-red-700 dark:bg-red-950/60 dark:text-red-300";
    case "warn":
      return "bg-amber-100 text-amber-700 dark:bg-amber-950/60 dark:text-amber-300";
    case "debug":
    case "trace":
      return "bg-violet-100 text-violet-700 dark:bg-violet-950/60 dark:text-violet-300";
    default:
      return "bg-sky-100 text-sky-700 dark:bg-sky-950/60 dark:text-sky-300";
  }
}

function TraceRow({
  trace,
  selected,
  onSelect,
}: {
  trace: RequestTraceSummary;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onSelect}
      className={cn(
        "w-full border-b border-border-default px-3 py-3 text-left transition-colors hover:bg-muted/60",
        selected && "bg-blue-50 dark:bg-blue-950/30",
      )}
    >
      <div className="flex min-w-0 items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2">
          {trace.outcome === "success" ? (
            <CheckCircle2
              className={cn("h-4 w-4 shrink-0", outcomeClass(trace.outcome))}
            />
          ) : trace.outcome === "error" ? (
            <AlertCircle
              className={cn("h-4 w-4 shrink-0", outcomeClass(trace.outcome))}
            />
          ) : (
            <Circle
              className={cn("h-4 w-4 shrink-0", outcomeClass(trace.outcome))}
            />
          )}
          <span className="truncate font-mono text-xs font-semibold">
            {trace.method} {trace.path}
          </span>
        </div>
        <span className="shrink-0 text-xs text-muted-foreground">
          {trace.statusCode ?? "..."}
        </span>
      </div>
      <div className="mt-2 flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
        <span className="truncate">{trace.appType}</span>
        <span aria-hidden="true">·</span>
        <span className="truncate">{trace.requestModel ?? "-"}</span>
        {trace.isStreaming ? <Waves className="h-3.5 w-3.5 shrink-0" /> : null}
        <span className="ml-auto shrink-0">
          {formatDuration(trace.durationMs)}
        </span>
      </div>
      <div className="mt-1 truncate text-[11px] text-muted-foreground">
        {formatTimestamp(trace.startedAt)}
      </div>
    </button>
  );
}

function EventPayload({ event }: { event: TraceEventRecord }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(event.stage !== "stream");
  if (event.payload == null) return null;
  const payload = JSON.stringify(event.payload, null, 2);

  return (
    <div className="mt-2 min-w-0">
      <div className="mb-1 flex items-center gap-1">
        <Button
          variant="ghost"
          size="sm"
          className="h-7 px-2"
          onClick={() => setExpanded((value) => !value)}
        >
          {expanded ? t("logs.collapse") : t("logs.expand")}
        </Button>
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7"
          title={t("common.copy")}
          onClick={() => void navigator.clipboard.writeText(payload)}
        >
          <Copy className="h-3.5 w-3.5" />
        </Button>
        {event.truncated ? (
          <Badge variant="outline" className="border-amber-400 text-amber-700">
            {t("logs.truncated")}
          </Badge>
        ) : null}
      </div>
      {expanded ? (
        <pre className="max-h-80 overflow-auto rounded-md bg-zinc-950 p-3 font-mono text-[11px] leading-relaxed text-zinc-100">
          {payload}
        </pre>
      ) : null}
    </div>
  );
}

function TraceDetail({
  traceId,
  live,
  onBack,
}: {
  traceId: string;
  live: boolean;
  onBack: () => void;
}) {
  const { t } = useTranslation();
  const detailQuery = useDiagnosticTrace(traceId, { live });
  const detail = detailQuery.data;

  if (detailQuery.isLoading) {
    return <LoadingState />;
  }
  if (!detail) {
    return <EmptyState text={t("logs.traceNotFound")} />;
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="border-b border-border-default px-4 py-3">
        <div className="flex items-start gap-2">
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8 shrink-0 lg:hidden"
            onClick={onBack}
            title={t("common.back")}
          >
            <ArrowLeft className="h-4 w-4" />
          </Button>
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <span className="font-mono text-sm font-semibold">
                {detail.trace.method} {detail.trace.path}
              </span>
              <Badge variant="outline">
                {detail.trace.statusCode ?? "..."}
              </Badge>
              {detail.trace.isStreaming ? (
                <Badge
                  variant="outline"
                  className="border-sky-400 text-sky-700 dark:text-sky-300"
                >
                  SSE
                </Badge>
              ) : null}
              {detail.trace.partial ? (
                <Badge
                  variant="outline"
                  className="border-amber-400 text-amber-700 dark:text-amber-300"
                >
                  {t("logs.partial")}
                </Badge>
              ) : null}
            </div>
            <div className="mt-2 grid grid-cols-2 gap-x-4 gap-y-1 text-xs text-muted-foreground sm:grid-cols-4">
              <span>
                {t("logs.app")}: {detail.trace.appType}
              </span>
              <span>
                {t("logs.provider")}: {detail.trace.finalProviderId ?? "-"}
              </span>
              <span>
                {t("logs.model")}:{" "}
                {detail.trace.responseModel ?? detail.trace.requestModel ?? "-"}
              </span>
              <span>
                {t("logs.duration")}: {formatDuration(detail.trace.durationMs)}
              </span>
              <span>
                {t("logs.attempts")}: {detail.trace.attemptCount}
              </span>
              <span>
                {t("logs.size")}: {formatBytes(detail.trace.storedBytes)}
              </span>
              <span
                className="col-span-2 truncate font-mono"
                title={detail.trace.traceId}
              >
                ID: {detail.trace.traceId}
              </span>
            </div>
          </div>
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3">
        <div className="relative space-y-3 before:absolute before:bottom-3 before:left-[7px] before:top-3 before:w-px before:bg-border-default">
          {detail.events.map((event) => (
            <div key={event.eventId} className="relative pl-7">
              <div className="absolute left-0 top-1.5 h-[15px] w-[15px] rounded-full border-2 border-background bg-blue-500" />
              <div className="rounded-md border border-border-default bg-background p-3">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-mono text-xs font-semibold">
                    {event.stage}
                  </span>
                  <Badge variant="secondary">{event.kind}</Badge>
                  {event.attemptNo != null ? (
                    <span className="text-xs text-muted-foreground">
                      #{event.attemptNo}
                    </span>
                  ) : null}
                  <span className="ml-auto text-xs tabular-nums text-muted-foreground">
                    +{event.offsetMs} ms
                  </span>
                </div>
                {event.summary ? (
                  <p className="mt-1 break-words text-sm text-muted-foreground">
                    {event.summary}
                  </p>
                ) : null}
                <EventPayload event={event} />
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

function LoadingState() {
  return (
    <div className="flex h-full items-center justify-center text-muted-foreground">
      <Loader2 className="h-5 w-5 animate-spin" />
    </div>
  );
}

function EmptyState({ text }: { text: string }) {
  return (
    <div className="flex h-full min-h-32 items-center justify-center px-4 text-center text-sm text-muted-foreground">
      {text}
    </div>
  );
}

export function LogCenterPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [tab, setTab] = useState<LogTab>("requests");
  const [live, setLive] = useState(true);
  const [search, setSearch] = useState("");
  const [appType, setAppType] = useState("");
  const [statusCode, setStatusCode] = useState("");
  const [streaming, setStreaming] = useState("");
  const [level, setLevel] = useState("");
  const [target, setTarget] = useState("");
  const [selectedTraceId, setSelectedTraceId] = useState<string | null>(null);
  const [clearKind, setClearKind] = useState<DiagnosticLogKind | null>(null);

  const traceQuery = useMemo<TraceQuery>(
    () => ({
      query: search.trim() || undefined,
      appType: appType || undefined,
      statusCode: statusCode ? Number(statusCode) : undefined,
      streaming:
        streaming === "true" ? true : streaming === "false" ? false : undefined,
      limit: 200,
    }),
    [appType, search, statusCode, streaming],
  );
  const runtimeQuery = useMemo<RuntimeLogQuery>(
    () => ({
      query: search.trim() || undefined,
      level: level || undefined,
      target: target.trim() || undefined,
      limit: 500,
    }),
    [level, search, target],
  );

  const tracesQuery = useDiagnosticTraces(traceQuery, {
    live: live && tab === "requests",
  });
  const runtimeQueryResult = useDiagnosticRuntimeLogs(runtimeQuery, {
    live: live && tab === "runtime",
  });
  const healthQuery = useDiagnosticLogHealth({ live });
  const clearMutation = useClearDiagnosticLogs();

  useEffect(() => {
    const traces = tracesQuery.data ?? [];
    if (
      selectedTraceId &&
      !traces.some((trace) => trace.traceId === selectedTraceId)
    ) {
      setSelectedTraceId(null);
    }
  }, [selectedTraceId, tracesQuery.data]);

  const refresh = async () => {
    await queryClient.invalidateQueries({ queryKey: diagnosticLogKeys.all });
  };

  const exportSelectedTrace = async () => {
    if (!selectedTraceId) return;
    try {
      const content = await diagnosticLogsApi.exportTrace(selectedTraceId);
      const url = URL.createObjectURL(
        new Blob([content], { type: "application/json" }),
      );
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = `cc-switch-trace-${selectedTraceId}.json`;
      anchor.click();
      URL.revokeObjectURL(url);
      toast.success(t("logs.exported"));
    } catch (error) {
      toast.error(t("logs.exportFailed", { error: String(error) }));
    }
  };

  const health = healthQuery.data;
  const categoryBytes =
    tab === "requests" ? health?.requestBytes : health?.runtimeBytes;

  return (
    <div className="flex h-full min-h-0 flex-col px-4 pb-4 sm:px-6">
      <div className="flex flex-wrap items-center gap-2 border-b border-border-default pb-3">
        <Tabs value={tab} onValueChange={(value) => setTab(value as LogTab)}>
          <TabsList>
            <TabsTrigger value="requests">
              {t("logs.requestTraces")}
            </TabsTrigger>
            <TabsTrigger value="runtime">{t("logs.runtimeLogs")}</TabsTrigger>
          </TabsList>
        </Tabs>
        <div className="ml-auto flex items-center gap-1">
          <span
            className={cn(
              "mr-2 text-xs",
              health?.available === false
                ? "text-red-600"
                : "text-muted-foreground",
            )}
          >
            {health?.available === false
              ? t("logs.unavailable")
              : t("logs.retentionSummary", {
                  days: health?.retentionDays ?? 3,
                  size: formatBytes(categoryBytes ?? 0),
                })}
          </span>
          <Button
            variant="ghost"
            size="icon"
            title={live ? t("logs.pauseLive") : t("logs.resumeLive")}
            onClick={() => setLive((value) => !value)}
          >
            {live ? (
              <Pause className="h-4 w-4" />
            ) : (
              <Play className="h-4 w-4" />
            )}
          </Button>
          <Button
            variant="ghost"
            size="icon"
            title={t("common.refresh")}
            onClick={() => void refresh()}
          >
            <RefreshCw className="h-4 w-4" />
          </Button>
          {tab === "requests" ? (
            <Button
              variant="ghost"
              size="icon"
              title={t("logs.export")}
              disabled={!selectedTraceId}
              onClick={() => void exportSelectedTrace()}
            >
              <Download className="h-4 w-4" />
            </Button>
          ) : null}
          <Button
            variant="ghost"
            size="icon"
            className="text-red-500 hover:text-red-600"
            title={t("logs.clear")}
            onClick={() => setClearKind(tab)}
          >
            <Trash2 className="h-4 w-4" />
          </Button>
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-2 py-3">
        <div className="relative min-w-48 flex-1 sm:max-w-sm">
          <Search className="absolute left-3 top-2.5 h-4 w-4 text-muted-foreground" />
          <Input
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder={t("logs.searchPlaceholder")}
            className="pl-9"
          />
        </div>
        {tab === "requests" ? (
          <>
            <select
              value={appType}
              onChange={(event) => setAppType(event.target.value)}
              className={selectClassName}
              aria-label={t("logs.app")}
            >
              <option value="">{t("logs.allApps")}</option>
              <option value="claude">Claude</option>
              <option value="claude-desktop">Claude Desktop</option>
              <option value="codex">Codex</option>
              <option value="gemini">Gemini</option>
            </select>
            <Input
              type="number"
              min={100}
              max={599}
              value={statusCode}
              onChange={(event) => setStatusCode(event.target.value)}
              placeholder={t("logs.status")}
              className="w-28"
            />
            <select
              value={streaming}
              onChange={(event) => setStreaming(event.target.value)}
              className={selectClassName}
              aria-label={t("logs.streamType")}
            >
              <option value="">{t("logs.allTypes")}</option>
              <option value="true">SSE</option>
              <option value="false">{t("logs.nonStreaming")}</option>
            </select>
          </>
        ) : (
          <>
            <select
              value={level}
              onChange={(event) => setLevel(event.target.value)}
              className={selectClassName}
              aria-label={t("logs.level")}
            >
              <option value="">{t("logs.allLevels")}</option>
              <option value="error">ERROR</option>
              <option value="warn">WARN</option>
              <option value="info">INFO</option>
              <option value="debug">DEBUG</option>
              <option value="trace">TRACE</option>
            </select>
            <Input
              value={target}
              onChange={(event) => setTarget(event.target.value)}
              placeholder={t("logs.target")}
              className="w-44"
            />
          </>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-hidden rounded-md border border-border-default bg-background">
        {tab === "requests" ? (
          <div className="grid h-full min-h-0 lg:grid-cols-[minmax(280px,38%)_minmax(0,1fr)]">
            <div
              className={cn(
                "min-h-0 overflow-y-auto border-r border-border-default",
                selectedTraceId && "hidden lg:block",
              )}
            >
              {tracesQuery.isLoading ? (
                <LoadingState />
              ) : tracesQuery.isError ? (
                <EmptyState text={t("logs.loadFailed")} />
              ) : (tracesQuery.data?.length ?? 0) === 0 ? (
                <EmptyState text={t("logs.noRequestLogs")} />
              ) : (
                tracesQuery.data?.map((trace) => (
                  <TraceRow
                    key={trace.traceId}
                    trace={trace}
                    selected={selectedTraceId === trace.traceId}
                    onSelect={() => setSelectedTraceId(trace.traceId)}
                  />
                ))
              )}
            </div>
            <div
              className={cn("min-h-0", !selectedTraceId && "hidden lg:block")}
            >
              {selectedTraceId ? (
                <TraceDetail
                  traceId={selectedTraceId}
                  live={live}
                  onBack={() => setSelectedTraceId(null)}
                />
              ) : (
                <EmptyState text={t("logs.selectTrace")} />
              )}
            </div>
          </div>
        ) : (
          <div className="h-full min-h-0 overflow-y-auto">
            {runtimeQueryResult.isLoading ? (
              <LoadingState />
            ) : runtimeQueryResult.isError ? (
              <EmptyState text={t("logs.loadFailed")} />
            ) : (runtimeQueryResult.data?.length ?? 0) === 0 ? (
              <EmptyState text={t("logs.noRuntimeLogs")} />
            ) : (
              <div className="divide-y divide-border-default font-mono text-xs">
                {runtimeQueryResult.data?.map((record) => (
                  <div
                    key={record.logId}
                    className="grid gap-2 px-3 py-2 hover:bg-muted/50 sm:grid-cols-[170px_72px_minmax(120px,220px)_1fr]"
                  >
                    <span className="text-muted-foreground">
                      {formatTimestamp(record.occurredAt)}
                    </span>
                    <span
                      className={cn(
                        "w-fit rounded px-1.5 py-0.5 font-semibold",
                        levelClass(record.level),
                      )}
                    >
                      {record.level}
                    </span>
                    <span
                      className="truncate text-muted-foreground"
                      title={record.target}
                    >
                      {record.target}
                    </span>
                    <span className="whitespace-pre-wrap break-words text-foreground">
                      {record.message}
                    </span>
                    {record.fields != null ? (
                      <pre className="overflow-x-auto rounded-md bg-muted p-2 sm:col-start-4">
                        {JSON.stringify(record.fields, null, 2)}
                      </pre>
                    ) : null}
                  </div>
                ))}
              </div>
            )}
          </div>
        )}
      </div>

      <ConfirmDialog
        isOpen={clearKind != null}
        title={t("logs.clearConfirmTitle")}
        message={t("logs.clearConfirmMessage")}
        confirmText={t("logs.clear")}
        onCancel={() => setClearKind(null)}
        onConfirm={() => {
          if (!clearKind) return;
          clearMutation.mutate(clearKind, {
            onSuccess: () => {
              setSelectedTraceId(null);
              setClearKind(null);
              toast.success(t("logs.cleared"));
            },
            onError: (error) =>
              toast.error(t("logs.clearFailed", { error: String(error) })),
          });
        }}
      />
    </div>
  );
}
