import { fireEvent, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

const mocks = vi.hoisted(() => ({
  clear: vi.fn(),
}));

vi.mock("@/lib/query/diagnosticLogs", () => ({
  diagnosticLogKeys: { all: ["diagnostic-logs"] },
  useDiagnosticTraces: () => ({
    data: [
      {
        traceId: "trace-1",
        usageRequestId: null,
        appType: "claude",
        method: "POST",
        path: "/v1/messages",
        requestModel: "claude-sonnet",
        responseModel: "claude-sonnet",
        finalProviderId: "provider-1",
        statusCode: 200,
        isStreaming: true,
        attemptCount: 1,
        startedAt: 1_700_000_000_000,
        completedAt: 1_700_000_000_120,
        durationMs: 120,
        outcome: "success",
        partial: false,
        droppedEventCount: 0,
        storedBytes: 1024,
      },
    ],
    isLoading: false,
    isError: false,
  }),
  useDiagnosticTrace: () => ({
    data: {
      trace: {
        traceId: "trace-1",
        usageRequestId: null,
        appType: "claude",
        method: "POST",
        path: "/v1/messages",
        requestModel: "claude-sonnet",
        responseModel: "claude-sonnet",
        finalProviderId: "provider-1",
        statusCode: 200,
        isStreaming: true,
        attemptCount: 1,
        startedAt: 1_700_000_000_000,
        completedAt: 1_700_000_000_120,
        durationMs: 120,
        outcome: "success",
        partial: false,
        droppedEventCount: 0,
        storedBytes: 1024,
      },
      events: [
        {
          eventId: 1,
          sequence: 1,
          occurredAt: 1_700_000_000_000,
          offsetMs: 0,
          stage: "client_request",
          kind: "request",
          attemptNo: null,
          providerId: null,
          statusCode: null,
          summary: "Client request received",
          payload: { body: { model: "claude-sonnet" } },
          truncated: false,
        },
      ],
    },
    isLoading: false,
  }),
  useDiagnosticRuntimeLogs: () => ({
    data: [],
    isLoading: false,
    isError: false,
  }),
  useDiagnosticLogHealth: () => ({
    data: {
      available: true,
      retentionDays: 3,
      requestBytes: 1024,
      runtimeBytes: 0,
    },
  }),
  useClearDiagnosticLogs: () => ({ mutate: mocks.clear }),
}));

vi.mock("@/lib/api/diagnosticLogs", () => ({
  diagnosticLogsApi: { exportTrace: vi.fn() },
}));

import { LogCenterPage } from "@/components/logs/LogCenterPage";

describe("LogCenterPage", () => {
  it("opens a request trace and renders its timeline", () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    render(
      <QueryClientProvider client={queryClient}>
        <LogCenterPage />
      </QueryClientProvider>,
    );

    fireEvent.click(screen.getByText("POST /v1/messages"));

    expect(screen.getByText("client_request")).toBeInTheDocument();
    expect(screen.getByText("Client request received")).toBeInTheDocument();
    expect(screen.getByText("request")).toBeInTheDocument();
  });
});
