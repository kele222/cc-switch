import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { RequestDetailPanel } from "@/components/usage/RequestDetailPanel";
import type { RequestLog } from "@/types/usage";

const useRequestDetailMock = vi.hoisted(() => vi.fn());

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, defaultValue?: string) => defaultValue ?? key,
    i18n: { language: "en" },
  }),
}));

vi.mock("@/lib/query/usage", () => ({
  useRequestDetail: (requestId: string) => useRequestDetailMock(requestId),
}));

vi.mock("@/components/ui/dialog", () => ({
  Dialog: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  DialogContent: ({ children }: { children: React.ReactNode }) => (
    <div>{children}</div>
  ),
  DialogHeader: ({ children }: { children: React.ReactNode }) => (
    <header>{children}</header>
  ),
  DialogTitle: ({ children }: { children: React.ReactNode }) => (
    <h2>{children}</h2>
  ),
}));

describe("RequestDetailPanel", () => {
  it("shows reasoning tokens without adding them to the total again", () => {
    const request: RequestLog = {
      requestId: "req-1",
      providerId: "provider-1",
      providerName: "Provider One",
      appType: "codex",
      model: "gpt-5.6",
      costMultiplier: "1",
      inputTokens: 1000,
      outputTokens: 20000,
      reasoningTokens: 12345,
      cacheReadTokens: 0,
      cacheCreationTokens: 0,
      inputCostUsd: "0.001",
      outputCostUsd: "0.002",
      cacheReadCostUsd: "0",
      cacheCreationCostUsd: "0",
      totalCostUsd: "0.003",
      isStreaming: true,
      latencyMs: 1200,
      statusCode: 200,
      createdAt: 1_710_000_000,
      dataSource: "proxy",
    };
    useRequestDetailMock.mockReturnValue({ data: request, isLoading: false });

    render(<RequestDetailPanel requestId="req-1" onClose={vi.fn()} />);

    expect(screen.getByText("usage.reasoningTokens")).toBeInTheDocument();
    expect(screen.getByText("12,345")).toBeInTheDocument();
    expect(screen.getByText("21,000")).toBeInTheDocument();
  });
});
