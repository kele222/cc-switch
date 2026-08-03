const invoke = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import { diagnosticLogsApi } from "@/lib/api/diagnosticLogs";

describe("diagnosticLogsApi", () => {
  beforeEach(() => invoke.mockReset());

  it("uses the diagnostic trace command contracts", async () => {
    invoke.mockResolvedValueOnce([]).mockResolvedValueOnce(null);

    await diagnosticLogsApi.getRequestTraces({
      query: "claude",
      streaming: true,
      limit: 50,
    });
    await diagnosticLogsApi.getTrace("trace-1");

    expect(invoke).toHaveBeenNthCalledWith(1, "get_diagnostic_request_traces", {
      query: { query: "claude", streaming: true, limit: 50 },
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "get_diagnostic_trace", {
      traceId: "trace-1",
    });
  });

  it("passes clear and export arguments without renaming values", async () => {
    invoke.mockResolvedValueOnce(undefined).mockResolvedValueOnce("{}");

    await diagnosticLogsApi.clear("runtime");
    await diagnosticLogsApi.exportTrace("trace-2");

    expect(invoke).toHaveBeenNthCalledWith(1, "clear_diagnostic_logs", {
      kind: "runtime",
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "export_diagnostic_trace", {
      traceId: "trace-2",
    });
  });
});
