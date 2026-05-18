import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Dialog, DialogPanel, DialogTitle } from "@headlessui/react";
import { Plus, AlertCircle, Loader2, X } from "lucide-react";
import { getZones, createZone } from "../api/zones";
import type { CreateZoneParams } from "../api/zones";

function formatDate(iso: string) {
  return new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  }).format(new Date(iso));
}

// ── Loading skeleton ──────────────────────────────────────────────────────────

function TableSkeleton() {
  return (
    <div className="animate-pulse space-y-2">
      {[...Array(4)].map((_, i) => (
        <div key={i} className="h-12 rounded bg-gray-100 dark:bg-gray-800" />
      ))}
    </div>
  );
}

// ── Error banner ──────────────────────────────────────────────────────────────

function ErrorBanner({ message }: { message: string }) {
  return (
    <div className="flex items-center gap-3 rounded-lg border border-red-200 bg-red-50 p-4 text-red-700 dark:border-red-800 dark:bg-red-900/20 dark:text-red-400">
      <AlertCircle className="h-5 w-5 flex-shrink-0" />
      <p className="text-sm">{message}</p>
    </div>
  );
}

// ── Create Zone modal ─────────────────────────────────────────────────────────

interface CreateZoneModalProps {
  open: boolean;
  onClose: () => void;
}

function CreateZoneModal({ open, onClose }: CreateZoneModalProps) {
  const queryClient = useQueryClient();
  const [name, setName] = useState("");
  const [defaultTtl, setDefaultTtl] = useState(300);
  const [formError, setFormError] = useState<string | null>(null);

  const mutation = useMutation({
    mutationFn: (params: CreateZoneParams) => createZone(params),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["zones"] });
      handleClose();
    },
    onError: (err: unknown) => {
      const msg =
        err instanceof Error ? err.message : "Failed to create zone.";
      setFormError(msg);
    },
  });

  function handleClose() {
    setName("");
    setDefaultTtl(300);
    setFormError(null);
    mutation.reset();
    onClose();
  }

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setFormError(null);
    if (!name.trim()) {
      setFormError("Zone name is required.");
      return;
    }
    if (defaultTtl < 1) {
      setFormError("Default TTL must be at least 1.");
      return;
    }
    mutation.mutate({ name: name.trim(), default_ttl: defaultTtl });
  }

  return (
    <Dialog open={open} onClose={handleClose} className="relative z-50">
      {/* Backdrop */}
      <div
        className="fixed inset-0 bg-black/40 dark:bg-black/60"
        aria-hidden="true"
      />

      {/* Panel wrapper */}
      <div className="fixed inset-0 flex items-center justify-center p-4">
        <DialogPanel className="w-full max-w-md rounded-xl bg-white p-6 shadow-xl dark:bg-gray-900">
          {/* Header */}
          <div className="mb-4 flex items-center justify-between">
            <DialogTitle className="text-base font-semibold text-gray-900 dark:text-white">
              New Zone
            </DialogTitle>
            <button
              onClick={handleClose}
              className="rounded-md p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600 dark:hover:bg-gray-800 dark:hover:text-gray-300"
            >
              <X className="h-4 w-4" />
            </button>
          </div>

          {/* Form */}
          <form onSubmit={handleSubmit} className="space-y-4">
            <div>
              <label
                htmlFor="zone-name"
                className="mb-1 block text-sm font-medium text-gray-700 dark:text-gray-300"
              >
                Zone Name
              </label>
              <input
                id="zone-name"
                type="text"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="example.com"
                className="w-full rounded-md border border-gray-300 bg-white px-3 py-2 text-sm text-gray-900 placeholder-gray-400 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-800 dark:text-white dark:placeholder-gray-500"
                autoFocus
              />
            </div>

            <div>
              <label
                htmlFor="zone-ttl"
                className="mb-1 block text-sm font-medium text-gray-700 dark:text-gray-300"
              >
                Default TTL (seconds)
              </label>
              <input
                id="zone-ttl"
                type="number"
                value={defaultTtl}
                onChange={(e) => setDefaultTtl(Number(e.target.value))}
                min={1}
                className="w-full rounded-md border border-gray-300 bg-white px-3 py-2 text-sm text-gray-900 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-800 dark:text-white"
              />
            </div>

            {formError && (
              <p className="text-sm text-red-600 dark:text-red-400">
                {formError}
              </p>
            )}

            {/* Actions */}
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
                Create
              </button>
            </div>
          </form>
        </DialogPanel>
      </div>
    </Dialog>
  );
}

// ── ZonesPage ─────────────────────────────────────────────────────────────────

export function ZonesPage() {
  const navigate = useNavigate();
  const [modalOpen, setModalOpen] = useState(false);

  const { data: zones, isLoading, isError, error } = useQuery({
    queryKey: ["zones"],
    queryFn: getZones,
  });

  const errorMessage =
    error instanceof Error ? error.message : "Failed to load zones.";

  return (
    <div className="space-y-4">
      {/* Page header */}
      <div className="flex items-center justify-between">
        <h1 className="text-xl font-semibold text-gray-900 dark:text-white">
          Zones
        </h1>
        <button
          onClick={() => setModalOpen(true)}
          className="flex items-center gap-1.5 rounded-md bg-blue-600 px-3 py-2 text-sm font-medium text-white hover:bg-blue-700"
        >
          <Plus className="h-4 w-4" />
          New Zone
        </button>
      </div>

      {/* Content */}
      {isLoading && <TableSkeleton />}

      {isError && <ErrorBanner message={errorMessage} />}

      {!isLoading && !isError && zones && (
        zones.length === 0 ? (
          <div className="rounded-lg border border-dashed border-gray-300 py-12 text-center dark:border-gray-700">
            <p className="text-sm text-gray-500 dark:text-gray-400">
              No zones yet.{" "}
              <button
                onClick={() => setModalOpen(true)}
                className="font-medium text-blue-600 hover:underline dark:text-blue-400"
              >
                Create your first zone.
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
                    Default TTL
                  </th>
                  <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-400">
                    Created
                  </th>
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
                {zones.map((zone) => (
                  <tr
                    key={zone.id}
                    onClick={() => navigate(`/zones/${zone.id}`)}
                    className="cursor-pointer hover:bg-gray-50 dark:hover:bg-gray-800"
                  >
                    <td className="px-4 py-3 font-medium text-gray-900 dark:text-white">
                      {zone.name}
                    </td>
                    <td className="px-4 py-3 tabular-nums text-gray-600 dark:text-gray-400">
                      {zone.default_ttl.toLocaleString()} s
                    </td>
                    <td className="px-4 py-3 text-gray-500 dark:text-gray-400">
                      {formatDate(zone.created_at)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )
      )}

      <CreateZoneModal open={modalOpen} onClose={() => setModalOpen(false)} />
    </div>
  );
}
