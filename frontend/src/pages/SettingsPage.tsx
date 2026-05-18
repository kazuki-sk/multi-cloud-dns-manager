import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import axios from "axios";
import {
  Monitor,
  Sun,
  Moon,
  CheckCircle2,
  XCircle,
  RefreshCw,
  Save,
  Check,
} from "lucide-react";
import { useTheme } from "../hooks/useTheme";
import type { ThemePreference } from "../hooks/useTheme";
import { apiClient, DEFAULT_API_BASE_URL } from "../api/client";

// ── Health check ──────────────────────────────────────────────────────────────

function deriveHealthUrl(baseUrl: string): string {
  if (/^https?:\/\//.test(baseUrl)) {
    try {
      return new URL("/health", baseUrl).toString();
    } catch {
      return "/health";
    }
  }
  return "/health";
}

async function checkHealth(baseUrl: string): Promise<{ status: string }> {
  const url = deriveHealthUrl(baseUrl);
  const res = await axios.get<{ status: string }>(url);
  return res.data;
}

// ── Section wrapper ───────────────────────────────────────────────────────────

function Section({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="rounded-lg border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-900">
      <div className="border-b border-gray-200 px-5 py-4 dark:border-gray-700">
        <h2 className="text-sm font-semibold text-gray-900 dark:text-white">
          {title}
        </h2>
        {description && (
          <p className="mt-0.5 text-xs text-gray-500 dark:text-gray-400">
            {description}
          </p>
        )}
      </div>
      <div className="px-5 py-4">{children}</div>
    </div>
  );
}

function Row({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-start justify-between gap-6">
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium text-gray-700 dark:text-gray-300">
          {label}
        </p>
        {hint && (
          <p className="mt-0.5 text-xs text-gray-400 dark:text-gray-500">
            {hint}
          </p>
        )}
      </div>
      <div className="flex-shrink-0">{children}</div>
    </div>
  );
}

// ── Appearance section ────────────────────────────────────────────────────────

const THEME_OPTIONS: { value: ThemePreference; label: string; Icon: React.ComponentType<{ className?: string }> }[] = [
  { value: "system", label: "System", Icon: Monitor },
  { value: "light",  label: "Light",  Icon: Sun },
  { value: "dark",   label: "Dark",   Icon: Moon },
];

function ThemeSelector() {
  const { preference, setThemePreference } = useTheme();

  return (
    <div className="inline-flex rounded-lg border border-gray-200 bg-gray-50 p-0.5 dark:border-gray-700 dark:bg-gray-800">
      {THEME_OPTIONS.map(({ value, label, Icon }) => (
        <button
          key={value}
          onClick={() => setThemePreference(value)}
          className={[
            "flex items-center gap-1.5 rounded-md px-3 py-1.5 text-sm font-medium transition-colors",
            preference === value
              ? "bg-white text-gray-900 shadow-sm dark:bg-gray-700 dark:text-white"
              : "text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200",
          ].join(" ")}
        >
          <Icon className="h-3.5 w-3.5" />
          {label}
        </button>
      ))}
    </div>
  );
}

// ── API Connection section ────────────────────────────────────────────────────

function ConnectionSection() {
  const [inputUrl, setInputUrl] = useState(
    () => localStorage.getItem("api-base-url") ?? DEFAULT_API_BASE_URL
  );
  const [savedUrl, setSavedUrl] = useState(
    () => localStorage.getItem("api-base-url") ?? DEFAULT_API_BASE_URL
  );
  const [justSaved, setJustSaved] = useState(false);

  const healthQuery = useQuery({
    queryKey: ["health", savedUrl],
    queryFn: () => checkHealth(savedUrl),
    retry: 1,
    refetchInterval: 30_000,
    staleTime: 10_000,
  });

  function handleSave() {
    const trimmed = inputUrl.trim() || DEFAULT_API_BASE_URL;
    setInputUrl(trimmed);
    setSavedUrl(trimmed);
    localStorage.setItem("api-base-url", trimmed);
    apiClient.defaults.baseURL = trimmed;
    setJustSaved(true);
    setTimeout(() => setJustSaved(false), 2000);
    healthQuery.refetch();
  }

  const isDirty = inputUrl.trim() !== savedUrl;

  return (
    <div className="space-y-4">
      {/* Status row */}
      <Row label="Backend status" hint="Checked every 30 seconds">
        <div className="flex items-center gap-2">
          {healthQuery.isLoading ? (
            <span className="flex items-center gap-1.5 text-sm text-gray-400 dark:text-gray-500">
              <RefreshCw className="h-4 w-4 animate-spin" />
              Checking…
            </span>
          ) : healthQuery.isSuccess ? (
            <span className="flex items-center gap-1.5 text-sm font-medium text-green-600 dark:text-green-400">
              <CheckCircle2 className="h-4 w-4" />
              Connected
            </span>
          ) : (
            <span className="flex items-center gap-1.5 text-sm font-medium text-red-600 dark:text-red-400">
              <XCircle className="h-4 w-4" />
              Disconnected
            </span>
          )}
          <button
            onClick={() => healthQuery.refetch()}
            disabled={healthQuery.isFetching}
            className="rounded p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600 disabled:opacity-40 dark:hover:bg-gray-800 dark:hover:text-gray-300"
            title="Retry"
          >
            <RefreshCw
              className={`h-3.5 w-3.5 ${healthQuery.isFetching ? "animate-spin" : ""}`}
            />
          </button>
        </div>
      </Row>

      {/* Divider */}
      <div className="border-t border-gray-100 dark:border-gray-800" />

      {/* URL row */}
      <Row
        label="API Base URL"
        hint="Endpoint used for all API requests. Changing this takes effect immediately."
      >
        <div className="flex items-center gap-2">
          <input
            type="text"
            value={inputUrl}
            onChange={(e) => setInputUrl(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && handleSave()}
            className="w-56 rounded-md border border-gray-300 bg-white px-3 py-1.5 font-mono text-sm text-gray-900 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-800 dark:text-white"
          />
          <button
            onClick={handleSave}
            disabled={!isDirty && !justSaved}
            className={[
              "flex items-center gap-1.5 rounded-md px-3 py-1.5 text-sm font-medium transition-colors",
              justSaved
                ? "bg-green-600 text-white"
                : isDirty
                ? "bg-blue-600 text-white hover:bg-blue-700"
                : "cursor-default bg-gray-100 text-gray-400 dark:bg-gray-800 dark:text-gray-500",
            ].join(" ")}
          >
            {justSaved ? (
              <>
                <Check className="h-3.5 w-3.5" />
                Saved
              </>
            ) : (
              <>
                <Save className="h-3.5 w-3.5" />
                Save
              </>
            )}
          </button>
        </div>
      </Row>
    </div>
  );
}

// ── About section ─────────────────────────────────────────────────────────────

const INFO_ROWS: { label: string; value: string }[] = [
  { label: "Application", value: "Multi-Cloud DNS Manager" },
  { label: "Description", value: "Unified management tool for DNS records across multiple cloud providers." },
  { label: "Stack", value: "Rust / axum / SQLite · React / Vite / TypeScript" },
];

function AboutSection() {
  return (
    <dl className="space-y-3">
      {INFO_ROWS.map(({ label, value }) => (
        <div key={label} className="flex gap-4">
          <dt className="w-28 flex-shrink-0 text-sm text-gray-500 dark:text-gray-400">
            {label}
          </dt>
          <dd className="text-sm text-gray-800 dark:text-gray-200">{value}</dd>
        </div>
      ))}
    </dl>
  );
}

// ── SettingsPage ──────────────────────────────────────────────────────────────

export function SettingsPage() {
  return (
    <div className="space-y-5">
      <h1 className="text-xl font-semibold text-gray-900 dark:text-white">
        Settings
      </h1>

      <div className="max-w-2xl space-y-4">
        <Section
          title="Appearance"
          description="Choose how the interface looks."
        >
          <Row
            label="Theme"
            hint="System follows your OS preference and updates automatically."
          >
            <ThemeSelector />
          </Row>
        </Section>

        <Section
          title="API Connection"
          description="Configure the backend endpoint."
        >
          <ConnectionSection />
        </Section>

        <Section title="About">
          <AboutSection />
        </Section>
      </div>
    </div>
  );
}
