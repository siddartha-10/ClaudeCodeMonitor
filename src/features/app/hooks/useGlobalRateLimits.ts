import { useCallback, useEffect, useState } from "react";
import { getGlobalRateLimits } from "../../../services/tauri";
import type { RateLimitSnapshot } from "../../../types";

function normalizeRateLimits(
  raw: Record<string, unknown>,
): RateLimitSnapshot | null {
  const asNumber = (value: unknown) =>
    typeof value === "number" && Number.isFinite(value) ? value : null;

  const normalizeWindow = (
    data: Record<string, unknown> | null,
  ): RateLimitSnapshot["primary"] => {
    if (!data) return null;
    const usedPercent = asNumber(data.usedPercent ?? data.used_percent);
    if (usedPercent === null) return null;
    return {
      usedPercent,
      windowDurationMins: (() => {
        const value = data.windowDurationMins ?? data.window_duration_mins;
        if (typeof value === "number") return value;
        if (typeof value === "string") {
          const parsed = Number(value);
          return Number.isFinite(parsed) ? parsed : null;
        }
        return null;
      })(),
      resetsAt: (() => {
        const value = data.resetsAt ?? data.resets_at;
        if (typeof value === "number") return value;
        if (typeof value === "string") {
          const parsed = Number(value);
          return Number.isFinite(parsed) ? parsed : null;
        }
        return null;
      })(),
    };
  };

  const primary = (raw.primary as Record<string, unknown>) ?? null;
  const secondary = (raw.secondary as Record<string, unknown>) ?? null;
  const sonnet = (raw.sonnet as Record<string, unknown>) ?? null;
  const credits = (raw.credits as Record<string, unknown>) ?? null;

  return {
    primary: normalizeWindow(primary),
    secondary: normalizeWindow(secondary),
    sonnet: normalizeWindow(sonnet),
    credits: credits
      ? {
          hasCredits: Boolean(credits.hasCredits ?? credits.has_credits),
          unlimited: Boolean(credits.unlimited),
          balance: String(credits.balance ?? null),
        }
      : null,
    planType: (raw.planType as string) ?? (raw.plan_type as string) ?? null,
  };
}

export function useGlobalRateLimits() {
  const [rateLimits, setRateLimits] = useState<RateLimitSnapshot | null>(null);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async () => {
    try {
      const response = await getGlobalRateLimits();
      if (response?.rateLimits) {
        setRateLimits(normalizeRateLimits(response.rateLimits));
      }
    } catch {
      // Ignore errors — rate limits are optional
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return { globalRateLimits: rateLimits, loading, refresh };
}
