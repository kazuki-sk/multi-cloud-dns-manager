import { useState } from "react";
import { Link } from "react-router-dom";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Dialog, DialogPanel, DialogTitle } from "@headlessui/react";
import {
  Plus,
  AlertCircle,
  Loader2,
  X,
  ChevronDown,
  ChevronRight,
  Trash2,
} from "lucide-react";
import { getProviders, createProvider, deleteProvider } from "../api/providers";
import type { CreateProviderParams } from "../api/providers";
import { getBindingsByProvider, createBinding, deleteBinding } from "../api/bindings";
import { getZones } from "../api/zones";
import type {
  Provider,
  ProviderStatus,
  ProviderType,
  ProviderZoneBinding,
} from "../types";

// ── Constants ─────────────────────────────────────────────────────────────────

const PROVIDER_TYPES: ProviderType[] = ["route53", "azuredns", "gcloud"];

const DEFAULT_CREDENTIAL_KEYS: Record<ProviderType, string[]> = {
  route53: ["access_key_id", "secret_access_key"],
  azuredns: [
    "tenant_id",
    "client_id",
    "client_secret",
    "subscription_id",
    "resource_group",
  ],
  gcloud: ["project_id", "service_account_json"],
};

const PROVIDER_LABELS: Record<ProviderType, string> = {
  route53: "Route 53",
  azuredns: "Azure DNS",
  gcloud: "Google Cloud DNS",
};

// ── Shared helpers ────────────────────────────────────────────────────────────

interface KVPair {
  key: string;
  value: string;
}

function makeDefaultPairs(type: ProviderType): KVPair[] {
  return DEFAULT_CREDENTIAL_KEYS[type].map((key) => ({ key, value: "" }));
}

function formatDate(iso: string) {
  return new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  }).format(new Date(iso));
}

function ErrorBanner({ message }: { message: string }) {
  return (
    <div className="flex items-center gap-3 rounded-lg border border-red-200 bg-red-50 p-4 text-red-700 dark:border-red-800 dark:bg-red-900/20 dark:text-red-400">
      <AlertCircle className="h-5 w-5 flex-shrink-0" />
      <p className="text-sm">{message}</p>
    </div>
  );
}

function TableSkeleton() {
  return (
    <div className="animate-pulse space-y-2">
      {[...Array(3)].map((_, i) => (
        <div key={i} className="h-12 rounded bg-gray-100 dark:bg-gray-800" />
      ))}
    </div>
  );
}

// ── Confirm Delete Dialog ─────────────────────────────────────────────────────

interface ConfirmDeleteDialogProps {
  open: boolean;
  title: string;
  message: string;
  isPending: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

function ConfirmDeleteDialog({
  open,
  title,
  message,
  isPending,
  onConfirm,
  onCancel,
}: ConfirmDeleteDialogProps) {
  return (
    <Dialog open={open} onClose={onCancel} className="relative z-50">
      <div className="fixed inset-0 bg-black/40 dark:bg-black/60" aria-hidden="true" />
      <div className="fixed inset-0 flex items-center justify-center p-4">
        <DialogPanel className="w-full max-w-sm rounded-xl bg-white p-6 shadow-xl dark:bg-gray-900">
          <DialogTitle className="text-base font-semibold text-gray-900 dark:text-white">
            {title}
          </DialogTitle>
          <p className="mt-2 text-sm text-gray-600 dark:text-gray-400">{message}</p>
          <div className="mt-4 flex justify-end gap-2">
            <button
              onClick={onCancel}
              disabled={isPending}
              className="rounded-md px-4 py-2 text-sm font-medium text-gray-600 hover:bg-gray-100 disabled:opacity-50 dark:text-gray-400 dark:hover:bg-gray-800"
            >
              Cancel
            </button>
            <button
              onClick={onConfirm}
              disabled={isPending}
              className="flex items-center gap-2 rounded-md bg-red-600 px-4 py-2 text-sm font-medium text-white hover:bg-red-700 disabled:cursor-not-allowed disabled:opacity-60"
            >
              {isPending && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
              Delete
            </button>
          </div>
        </DialogPanel>
      </div>
    </Dialog>
  );
}

// ── Status badge ──────────────────────────────────────────────────────────────

const STATUS_STYLES: Record<ProviderStatus, string> = {
  active:
    "bg-green-100 text-green-700 dark:bg-green-900/40 dark:text-green-400",
  paused:
    "bg-amber-100 text-amber-700 dark:bg-amber-900/40 dark:text-amber-400",
  error: "bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-400",
};

function StatusBadge({ status }: { status: ProviderStatus }) {
  return (
    <span
      className={`inline-block rounded px-1.5 py-0.5 text-xs font-medium ${STATUS_STYLES[status]}`}
    >
      {status}
    </span>
  );
}

// ── Add Provider Modal ────────────────────────────────────────────────────────

interface AddProviderModalProps {
  open: boolean;
  onClose: () => void;
}

function AddProviderModal({ open, onClose }: AddProviderModalProps) {
  const queryClient = useQueryClient();

  const [name, setName] = useState("");
  const [providerType, setProviderType] = useState<ProviderType>("route53");
  const [credentials, setCredentials] = useState<KVPair[]>(
    makeDefaultPairs("route53")
  );
  const [formError, setFormError] = useState<string | null>(null);

  const mutation = useMutation({
    mutationFn: (params: CreateProviderParams) => createProvider(params),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["providers"] });
      handleClose();
    },
    onError: (err: unknown) => {
      setFormError(
        err instanceof Error ? err.message : "Failed to create provider."
      );
    },
  });

  function handleClose() {
    setName("");
    setProviderType("route53");
    setCredentials(makeDefaultPairs("route53"));
    setFormError(null);
    mutation.reset();
    onClose();
  }

  function handleProviderTypeChange(newType: ProviderType) {
    setProviderType(newType);
    setCredentials(makeDefaultPairs(newType));
  }

  function updateCredKey(i: number, key: string) {
    setCredentials((prev) => prev.map((p, j) => (j === i ? { ...p, key } : p)));
  }

  function updateCredValue(i: number, value: string) {
    setCredentials((prev) =>
      prev.map((p, j) => (j === i ? { ...p, value } : p))
    );
  }

  function addCredPair() {
    setCredentials((prev) => [...prev, { key: "", value: "" }]);
  }

  function removeCredPair(i: number) {
    setCredentials((prev) => prev.filter((_, j) => j !== i));
  }

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setFormError(null);

    if (!name.trim()) {
      setFormError("Name is required.");
      return;
    }
    const validPairs = credentials.filter((p) => p.key.trim());
    if (validPairs.length === 0) {
      setFormError("At least one credential key is required.");
      return;
    }
    const credentialsObj: Record<string, string> = Object.fromEntries(
      validPairs.map((p) => [p.key.trim(), p.value])
    );

    mutation.mutate({
      name: name.trim(),
      provider_type: providerType,
      credentials: credentialsObj,
    });
  }

  return (
    <Dialog open={open} onClose={handleClose} className="relative z-50">
      <div
        className="fixed inset-0 bg-black/40 dark:bg-black/60"
        aria-hidden="true"
      />
      <div className="fixed inset-0 flex items-center justify-center p-4">
        <DialogPanel className="w-full max-w-lg rounded-xl bg-white p-6 shadow-xl dark:bg-gray-900">
          <div className="mb-4 flex items-center justify-between">
            <DialogTitle className="text-base font-semibold text-gray-900 dark:text-white">
              Add Provider
            </DialogTitle>
            <button
              onClick={handleClose}
              className="rounded-md p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600 dark:hover:bg-gray-800 dark:hover:text-gray-300"
            >
              <X className="h-4 w-4" />
            </button>
          </div>

          <form onSubmit={handleSubmit} className="space-y-4">
            <div>
              <label
                htmlFor="prov-name"
                className="mb-1 block text-sm font-medium text-gray-700 dark:text-gray-300"
              >
                Name
              </label>
              <input
                id="prov-name"
                type="text"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="e.g. prod-route53"
                className="w-full rounded-md border border-gray-300 bg-white px-3 py-2 text-sm text-gray-900 placeholder-gray-400 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-800 dark:text-white dark:placeholder-gray-500"
                autoFocus
              />
            </div>

            <div>
              <label
                htmlFor="prov-type"
                className="mb-1 block text-sm font-medium text-gray-700 dark:text-gray-300"
              >
                Provider Type
              </label>
              <select
                id="prov-type"
                value={providerType}
                onChange={(e) =>
                  handleProviderTypeChange(e.target.value as ProviderType)
                }
                className="w-full rounded-md border border-gray-300 bg-white px-3 py-2 text-sm text-gray-900 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-800 dark:text-white"
              >
                {PROVIDER_TYPES.map((t) => (
                  <option key={t} value={t}>
                    {PROVIDER_LABELS[t]}
                  </option>
                ))}
              </select>
            </div>

            <div>
              <span className="mb-1 block text-sm font-medium text-gray-700 dark:text-gray-300">
                Credentials
              </span>
              <div className="space-y-2">
                {credentials.map((pair, i) => (
                  <div key={i} className="flex items-center gap-2">
                    <input
                      type="text"
                      value={pair.key}
                      onChange={(e) => updateCredKey(i, e.target.value)}
                      placeholder="key"
                      className="w-2/5 rounded-md border border-gray-300 bg-white px-3 py-1.5 text-sm font-mono text-gray-900 placeholder-gray-400 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-800 dark:text-white dark:placeholder-gray-500"
                    />
                    <span className="text-gray-400">:</span>
                    <input
                      type="password"
                      value={pair.value}
                      onChange={(e) => updateCredValue(i, e.target.value)}
                      placeholder="value"
                      autoComplete="off"
                      className="min-w-0 flex-1 rounded-md border border-gray-300 bg-white px-3 py-1.5 text-sm text-gray-900 placeholder-gray-400 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-800 dark:text-white dark:placeholder-gray-500"
                    />
                    <button
                      type="button"
                      onClick={() => removeCredPair(i)}
                      className="flex-shrink-0 rounded-md p-1.5 text-gray-400 hover:bg-gray-100 hover:text-red-500 dark:hover:bg-gray-800"
                    >
                      <X className="h-3.5 w-3.5" />
                    </button>
                  </div>
                ))}
              </div>
              <button
                type="button"
                onClick={addCredPair}
                className="mt-2 text-xs font-medium text-blue-600 hover:underline dark:text-blue-400"
              >
                + Add key
              </button>
            </div>

            {formError && (
              <p className="text-sm text-red-600 dark:text-red-400">
                {formError}
              </p>
            )}

            <div className="flex justify-end gap-2 pt-2">
              <button
                type="button"
                onClick={handleClose}
                className="rounded-md px-4 py-2 text-sm font-medium text-gray-600 hover:bg-gray-100 dark:text-gray-400 dark:hover:bg-gray-800"
              >
                Cancel
              </button>
              <button
                type="submit"
                disabled={mutation.isPending}
                className="flex items-center gap-2 rounded-md bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:cursor-not-allowed disabled:opacity-60"
              >
                {mutation.isPending && (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                )}
                Add Provider
              </button>
            </div>
          </form>
        </DialogPanel>
      </div>
    </Dialog>
  );
}

// ── Add Zone Binding Modal ────────────────────────────────────────────────────

interface AddZoneBindingModalProps {
  open: boolean;
  onClose: () => void;
  providerId: string;
}

function AddZoneBindingModal({
  open,
  onClose,
  providerId,
}: AddZoneBindingModalProps) {
  const queryClient = useQueryClient();

  const [zoneId, setZoneId] = useState("");
  const [providerZoneId, setProviderZoneId] = useState("");
  const [formError, setFormError] = useState<string | null>(null);

  const { data: zones } = useQuery({
    queryKey: ["zones"],
    queryFn: getZones,
    enabled: open,
  });

  const mutation = useMutation({
    mutationFn: () =>
      createBinding(zoneId, {
        provider_id: providerId,
        provider_zone_id: providerZoneId.trim(),
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["provider-bindings", providerId],
      });
      handleClose();
    },
    onError: (err: unknown) => {
      setFormError(
        err instanceof Error ? err.message : "Failed to create binding."
      );
    },
  });

  function handleClose() {
    setZoneId("");
    setProviderZoneId("");
    setFormError(null);
    mutation.reset();
    onClose();
  }

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setFormError(null);
    if (!zoneId) {
      setFormError("Select a zone.");
      return;
    }
    if (!providerZoneId.trim()) {
      setFormError("Provider Zone ID is required.");
      return;
    }
    mutation.mutate();
  }

  return (
    <Dialog open={open} onClose={handleClose} className="relative z-50">
      <div
        className="fixed inset-0 bg-black/40 dark:bg-black/60"
        aria-hidden="true"
      />
      <div className="fixed inset-0 flex items-center justify-center p-4">
        <DialogPanel className="w-full max-w-md rounded-xl bg-white p-6 shadow-xl dark:bg-gray-900">
          <div className="mb-4 flex items-center justify-between">
            <DialogTitle className="text-base font-semibold text-gray-900 dark:text-white">
              Add Zone Binding
            </DialogTitle>
            <button
              onClick={handleClose}
              className="rounded-md p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600 dark:hover:bg-gray-800 dark:hover:text-gray-300"
            >
              <X className="h-4 w-4" />
            </button>
          </div>

          <form onSubmit={handleSubmit} className="space-y-4">
            <div>
              <label
                htmlFor="binding-zone"
                className="mb-1 block text-sm font-medium text-gray-700 dark:text-gray-300"
              >
                Zone
              </label>
              <select
                id="binding-zone"
                value={zoneId}
                onChange={(e) => setZoneId(e.target.value)}
                className="w-full rounded-md border border-gray-300 bg-white px-3 py-2 text-sm text-gray-900 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-800 dark:text-white"
              >
                <option value="">— select zone —</option>
                {zones?.map((z) => (
                  <option key={z.id} value={z.id}>
                    {z.name}
                  </option>
                ))}
              </select>
            </div>

            <div>
              <label
                htmlFor="binding-pzid"
                className="mb-1 block text-sm font-medium text-gray-700 dark:text-gray-300"
              >
                Provider Zone ID
              </label>
              <input
                id="binding-pzid"
                type="text"
                value={providerZoneId}
                onChange={(e) => setProviderZoneId(e.target.value)}
                placeholder="e.g. Z1D633PJN98FT9"
                className="w-full rounded-md border border-gray-300 bg-white px-3 py-2 font-mono text-sm text-gray-900 placeholder-gray-400 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-800 dark:text-white dark:placeholder-gray-500"
              />
              <p className="mt-1 text-xs text-gray-400 dark:text-gray-500">
                The zone identifier in the cloud provider (e.g. Route 53 Hosted
                Zone ID).
              </p>
            </div>

            {formError && (
              <p className="text-sm text-red-600 dark:text-red-400">
                {formError}
              </p>
            )}

            <div className="flex justify-end gap-2 pt-2">
              <button
                type="button"
                onClick={handleClose}
                className="rounded-md px-4 py-2 text-sm font-medium text-gray-600 hover:bg-gray-100 dark:text-gray-400 dark:hover:bg-gray-800"
              >
                Cancel
              </button>
              <button
                type="submit"
                disabled={mutation.isPending}
                className="flex items-center gap-2 rounded-md bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:cursor-not-allowed disabled:opacity-60"
              >
                {mutation.isPending && (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                )}
                Add Binding
              </button>
            </div>
          </form>
        </DialogPanel>
      </div>
    </Dialog>
  );
}

// ── Provider Bindings Section (inline expand) ─────────────────────────────────

function ProviderBindingsSection({ provider }: { provider: Provider }) {
  const queryClient = useQueryClient();
  const [addOpen, setAddOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<ProviderZoneBinding | null>(null);

  const { data: bindings, isLoading } = useQuery({
    queryKey: ["provider-bindings", provider.id],
    queryFn: () => getBindingsByProvider(provider.id),
  });

  const deleteBindingMutation = useMutation({
    mutationFn: (b: ProviderZoneBinding) => deleteBinding(b.zone_id, b.id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["provider-bindings", provider.id] });
      setDeleteTarget(null);
    },
  });

  return (
    <div className="border-t border-gray-100 bg-gray-50 px-6 py-3 dark:border-gray-700 dark:bg-gray-800/50">
      <div className="mb-2 flex items-center justify-between">
        <span className="text-xs font-semibold uppercase tracking-wide text-gray-500 dark:text-gray-400">
          Zone Bindings
        </span>
        <button
          onClick={(e) => {
            e.stopPropagation();
            setAddOpen(true);
          }}
          className="flex items-center gap-1 rounded-md border border-gray-300 bg-white px-2 py-1 text-xs font-medium text-gray-600 hover:bg-gray-50 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-300 dark:hover:bg-gray-700"
        >
          <Plus className="h-3 w-3" />
          Add Zone Binding
        </button>
      </div>

      {isLoading ? (
        <div className="h-5 w-48 animate-pulse rounded bg-gray-200 dark:bg-gray-700" />
      ) : !bindings || bindings.length === 0 ? (
        <p className="py-2 text-xs text-gray-400 dark:text-gray-500">
          No zone bindings yet. Add one to start syncing records.
        </p>
      ) : (
        <table className="w-full text-xs">
          <thead>
            <tr>
              <th className="pb-1 pr-6 text-left font-medium text-gray-500 dark:text-gray-400">
                Zone
              </th>
              <th className="pb-1 pr-6 text-left font-medium text-gray-500 dark:text-gray-400">
                Provider Zone ID
              </th>
              <th className="pb-1 pr-6 text-left font-medium text-gray-500 dark:text-gray-400">
                Status
              </th>
              <th className="pb-1 text-left font-medium text-gray-500 dark:text-gray-400" />
            </tr>
          </thead>
          <tbody className="divide-y divide-gray-100 dark:divide-gray-700/60">
            {(bindings as ProviderZoneBinding[]).map((b) => (
              <tr key={b.id}>
                <td className="py-1.5 pr-6">
                  <Link
                    to={`/zones/${b.zone_id}`}
                    onClick={(e) => e.stopPropagation()}
                    className="font-medium text-blue-600 hover:underline dark:text-blue-400"
                  >
                    {b.zone_name}
                  </Link>
                </td>
                <td className="py-1.5 pr-6 font-mono text-gray-600 dark:text-gray-300">
                  {b.provider_zone_id}
                </td>
                <td className="py-1.5 pr-6">
                  <StatusBadge status={b.status} />
                </td>
                <td className="py-1.5 text-right">
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      setDeleteTarget(b);
                    }}
                    className="rounded p-1 text-gray-400 hover:bg-red-50 hover:text-red-500 dark:hover:bg-red-900/20 dark:hover:text-red-400"
                    title="Remove binding"
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      <AddZoneBindingModal
        open={addOpen}
        onClose={() => setAddOpen(false)}
        providerId={provider.id}
      />

      <ConfirmDeleteDialog
        open={deleteTarget !== null}
        title="Remove Zone Binding"
        message={
          deleteTarget
            ? `Remove binding to zone "${deleteTarget.zone_name}"? This stops record synchronization but does not delete DNS records from the provider.`
            : ""
        }
        isPending={deleteBindingMutation.isPending}
        onConfirm={() => deleteTarget && deleteBindingMutation.mutate(deleteTarget)}
        onCancel={() => setDeleteTarget(null)}
      />
    </div>
  );
}

// ── ProvidersPage ─────────────────────────────────────────────────────────────

export function ProvidersPage() {
  const queryClient = useQueryClient();
  const [addProviderOpen, setAddProviderOpen] = useState(false);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<Provider | null>(null);

  const {
    data: providers,
    isLoading,
    isError,
    error,
  } = useQuery({
    queryKey: ["providers"],
    queryFn: getProviders,
  });

  const deleteProviderMutation = useMutation({
    mutationFn: (p: Provider) => deleteProvider(p.id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["providers"] });
      setDeleteTarget(null);
    },
  });

  const errorMessage =
    error instanceof Error ? error.message : "Failed to load providers.";

  function toggleExpand(id: string) {
    setExpandedId((prev) => (prev === id ? null : id));
  }

  return (
    <div className="space-y-4">
      {/* Page header */}
      <div className="flex items-center justify-between">
        <h1 className="text-xl font-semibold text-gray-900 dark:text-white">
          Providers
        </h1>
        <button
          onClick={() => setAddProviderOpen(true)}
          className="flex items-center gap-1.5 rounded-md bg-blue-600 px-3 py-2 text-sm font-medium text-white hover:bg-blue-700"
        >
          <Plus className="h-4 w-4" />
          Add Provider
        </button>
      </div>

      {/* Content */}
      {isLoading && <TableSkeleton />}

      {isError && <ErrorBanner message={errorMessage} />}

      {!isLoading && !isError && providers && (
        providers.length === 0 ? (
          <div className="rounded-lg border border-dashed border-gray-300 py-12 text-center dark:border-gray-700">
            <p className="text-sm text-gray-500 dark:text-gray-400">
              No providers yet.{" "}
              <button
                onClick={() => setAddProviderOpen(true)}
                className="font-medium text-blue-600 hover:underline dark:text-blue-400"
              >
                Add your first provider.
              </button>
            </p>
          </div>
        ) : (
          <div className="overflow-hidden rounded-lg border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-900">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-gray-200 bg-gray-50 dark:border-gray-700 dark:bg-gray-800">
                  <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-400">
                    Name
                  </th>
                  <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-400">
                    Type
                  </th>
                  <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-400">
                    Status
                  </th>
                  <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-400">
                    Created
                  </th>
                  <th className="px-4 py-3" />
                </tr>
              </thead>
              <tbody>
                {(providers as Provider[]).map((p) => (
                  <>
                    <tr
                      key={p.id}
                      onClick={() => toggleExpand(p.id)}
                      className="cursor-pointer border-b border-gray-200 hover:bg-gray-50 dark:border-gray-700 dark:hover:bg-gray-800"
                    >
                      <td className="px-4 py-3">
                        <div className="flex items-center gap-2">
                          {expandedId === p.id ? (
                            <ChevronDown className="h-4 w-4 flex-shrink-0 text-gray-400" />
                          ) : (
                            <ChevronRight className="h-4 w-4 flex-shrink-0 text-gray-400" />
                          )}
                          <span className="font-medium text-gray-900 dark:text-white">
                            {p.name}
                          </span>
                        </div>
                      </td>
                      <td className="px-4 py-3 text-gray-600 dark:text-gray-400">
                        {PROVIDER_LABELS[p.provider_type]}
                      </td>
                      <td className="px-4 py-3">
                        <StatusBadge status={p.status} />
                      </td>
                      <td className="px-4 py-3 text-gray-500 dark:text-gray-400">
                        {formatDate(p.created_at)}
                      </td>
                      <td className="px-4 py-3 text-right">
                        <button
                          onClick={(e) => {
                            e.stopPropagation();
                            setDeleteTarget(p);
                          }}
                          className="rounded-md p-1.5 text-gray-400 hover:bg-red-50 hover:text-red-500 dark:hover:bg-red-900/20 dark:hover:text-red-400"
                          title="Delete provider"
                        >
                          <Trash2 className="h-4 w-4" />
                        </button>
                      </td>
                    </tr>
                    {expandedId === p.id && (
                      <tr key={`${p.id}-bindings`} className="border-b border-gray-200 dark:border-gray-700">
                        <td colSpan={5} className="p-0">
                          <ProviderBindingsSection provider={p} />
                        </td>
                      </tr>
                    )}
                  </>
                ))}
              </tbody>
            </table>
          </div>
        )
      )}

      <AddProviderModal
        open={addProviderOpen}
        onClose={() => setAddProviderOpen(false)}
      />

      <ConfirmDeleteDialog
        open={deleteTarget !== null}
        title="Delete Provider"
        message={
          deleteTarget
            ? `Delete provider "${deleteTarget.name}"? All zone bindings and sync states for this provider will also be removed.`
            : ""
        }
        isPending={deleteProviderMutation.isPending}
        onConfirm={() => deleteTarget && deleteProviderMutation.mutate(deleteTarget)}
        onCancel={() => setDeleteTarget(null)}
      />
    </div>
  );
}
