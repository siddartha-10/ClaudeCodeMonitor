// @vitest-environment jsdom
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppSettings, ClaudeDoctorResult } from "../../../types";
import { useAppSettings } from "./useAppSettings";
import { getAppSettings, runClaudeDoctor, updateAppSettings } from "../../../services/tauri";
import { UI_SCALE_DEFAULT, UI_SCALE_MAX } from "../../../utils/uiScale";

vi.mock("../../../services/tauri", () => ({
  getAppSettings: vi.fn(),
  updateAppSettings: vi.fn(),
  runClaudeDoctor: vi.fn(),
}));

const getAppSettingsMock = vi.mocked(getAppSettings);
const updateAppSettingsMock = vi.mocked(updateAppSettings);
const runClaudeDoctorMock = vi.mocked(runClaudeDoctor);

describe("useAppSettings", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    cleanup();
  });

  it("loads settings and normalizes theme + uiScale", async () => {
    getAppSettingsMock.mockResolvedValue(
      {
        uiScale: UI_SCALE_MAX + 1,
        theme: "nope" as unknown as AppSettings["theme"],
        backendMode: "remote",
        remoteBackendHost: "example:1234",
      } as AppSettings,
    );

    const { result } = renderHook(() => useAppSettings());

    await waitFor(() => expect(result.current.isLoading).toBe(false));

    expect(result.current.settings.uiScale).toBe(UI_SCALE_MAX);
    expect(result.current.settings.theme).toBe("system");
    expect(result.current.settings.backendMode).toBe("remote");
    expect(result.current.settings.remoteBackendHost).toBe("example:1234");
  });

  it("keeps defaults when getAppSettings fails", async () => {
    getAppSettingsMock.mockRejectedValue(new Error("boom"));

    const { result } = renderHook(() => useAppSettings());

    await waitFor(() => expect(result.current.isLoading).toBe(false));

    expect(result.current.settings.uiScale).toBe(UI_SCALE_DEFAULT);
    expect(result.current.settings.theme).toBe("system");
    expect(result.current.settings.backendMode).toBe("local");
    expect(result.current.settings.dictationModelId).toBe("base");
  });

  it("persists settings via updateAppSettings and updates local state", async () => {
    getAppSettingsMock.mockResolvedValue({} as AppSettings);
    const { result } = renderHook(() => useAppSettings());

    await waitFor(() => expect(result.current.isLoading).toBe(false));

    const next: AppSettings = {
      ...result.current.settings,
      theme: "nope" as unknown as AppSettings["theme"],
      uiScale: 0.04,
      notificationSoundsEnabled: false,
    };
    const saved: AppSettings = {
      ...result.current.settings,
      theme: "dark",
      uiScale: 2.4,
      notificationSoundsEnabled: false,
    };
    updateAppSettingsMock.mockResolvedValue(saved);

    let returned: AppSettings | undefined;
    await act(async () => {
      returned = await result.current.saveSettings(next);
    });

    expect(updateAppSettingsMock).toHaveBeenCalledWith(
      expect.objectContaining({
        theme: "system",
        uiScale: 0.1,
        notificationSoundsEnabled: false,
      }),
    );
    expect(returned).toEqual(saved);
    expect(result.current.settings.theme).toBe("dark");
    expect(result.current.settings.uiScale).toBe(2.4);
  });

  it("surfaces doctor errors", async () => {
    getAppSettingsMock.mockResolvedValue({} as AppSettings);
    runClaudeDoctorMock.mockRejectedValue(new Error("doctor fail"));
    const { result } = renderHook(() => useAppSettings());

    await waitFor(() => expect(result.current.isLoading).toBe(false));

    await expect(result.current.doctor("/bin/claude")).rejects.toThrow(
      "doctor fail",
    );
    expect(runClaudeDoctorMock).toHaveBeenCalledWith("/bin/claude");
  });

  it("returns doctor results", async () => {
    getAppSettingsMock.mockResolvedValue({} as AppSettings);
    const response: ClaudeDoctorResult = {
      ok: true,
      claudeBin: "/bin/claude",
      version: "1.0.0",
      path: null,
    };
    runClaudeDoctorMock.mockResolvedValue(response);
    const { result } = renderHook(() => useAppSettings());

    await waitFor(() => expect(result.current.isLoading).toBe(false));

    await expect(result.current.doctor("/bin/claude")).resolves.toEqual(
      response,
    );
  });
});
