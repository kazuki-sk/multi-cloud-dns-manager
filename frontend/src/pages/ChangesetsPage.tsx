import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Dialog, DialogPanel, DialogTitle } from "@headlessui/react";
import {
  AlertCircle,
  CheckCircle2,
  ChevronRight,
  Loader2,
  RefreshCw,
  X,
} from "lucide-react";
import {
  getChangesets,
  validateChangeset,
  applyChangeset,
  rollbackChangeset,
} from "../api/changesets";
import type { Changeset, ChangesetItem, ChangesetStatus } from "../types";

// ── Status config ─────────────────────────────────────────────────────────────

const STATUS_CONFIG: Record<
  ChangesetStatus,
  { label: string; className: string; pulse?: boolean }
> = {
  draft: {
    label: "Draft",
    className:
      "bg-gray-100 text-gray-600 dark:bg-gray-700 dark:text-gray-300",
  },
  validated: {
    label: "Validated",
    className:
      "bg-blue-100 text-blue-700 dark:bg-blue-900/40 dark:text-blue-400",
  },
  applying: {
    label: "Applying",
    className:
      "bg-amber-100 text-amber-700 dark:bg-amber-900/40 dark:text-amber-400",
    pulse: true,
  },
  applied: {
    label: "Applied",
    className:
      "bg-green-100 text-green-700 dark:bg-green-900/40 dark:text-green-400",
  },
  rolling_back: {
    label: "Rolling Back",
    className:
      "bg-amber-100 text-amber-700 dark:bg-amber-900/40 dark:text-amber-400",
    pulse: true,
  },
  rolled_back: {
    label: "Rolled Back",
    className:
      "bg-teal-100 text-teal-700 dark:bg-teal-900/40 dark:text-teal-400",
  },
  rollback_failed: {
    label: "Rollback Failed",
    className: "bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-400",
  },
  frozen: {
    label: "Frozen",
    className:
      "bg-purple-100 text-purple-700 dark:bg-purple-900/40 dark:text-purple-400",
  },
};

const OPERATION_CONFIG: Record<
  string,
  { label: string; className: string }
> = {
  create: {
    label: "CREATE",
    className:
      "bg-green-100 text-green-700 dark:bg-green-900/40 dark:text-green-400",
  },
  update: {
    label: "UPDATE",
    className:
      "bg-blue-100 text-blue-700 dark:bg-blue-900/40 dark:text-blue-400",
  },
  delete: {
    label: "DELETE",
    className: "bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-400",
  },
};

// ── Helpers ───────────────────────────────────────────────────────────────────

function formatDate(iso: string) {
  return new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(iso));
}

function formatPolicy(p: string) {
  if (p === "auto") return "Auto";
  if (p === "manual") return "Manual";
  if (p === "frozen_on_failure") return "Freeze on fail";
  return p;
}

function isInProgress(status: ChangesetStatus) {
  return status === "applying" || status === "rolling_back";
}

// ── Status badge ──────────────────────────────────────────────────────────────

function StatusBadge({ status }: { status: ChangesetStatus }) {
  const cfg = STATUS_CONFIG[status] ?? {
    label: status,
    className: "bg-gray-100 text-gray-600",
  };
  return (
    <span
      className={`inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-xs font-medium ${cfg.className}`}
    >
      {cfg.pulse && (
        <span className="inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-current" />
      )}
      {cfg.label}
    </span>
  );
}

// ── Shared UI ─────────────────────────────────────────────────────────────────

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
      {[...Array(4)].map((_, i) => (
        <div key={i} className="h-14 rounded bg-gray-100 dark:bg-gray-800" />
      ))}
    </div>
  );
}

// ── Changeset Detail Modal ────────────────────────────────────────────────────

interface DetailModalProps {
  changeset: Changeset | null;
  onClose: () => void;
}

function DetailModal({ changeset, onClose }: DetailModalProps) {
  if (!changeset) return null;

  return (
    <Dialog open={!!changeset} onClose={onClose} className="relative z-50">
      <div className="fixed inset-0 bg-black/40 dark:bg-black/60" aria-hidden="true" />
      <div className="fixed inset-0 flex items-center justify-center p-4">
        <DialogPanel className="w-full max-w-2xl rounded-xl bg-white p-6 shadow-xl dark:bg-gray-900 max-h-[85vh] flex flex-col">
          {/* Header */}
          <div className="mb-4 flex items-start justify-between gap-4">
            <div className="min-w-0">
              <DialogTitle className="text-base font-semibold text-gray-900 dark:text-white">
                Changeset Details
              </DialogTitle>
              <p className="mt-0.5 truncate font-mono text-xs text-gray-400">
                {changeset.id}
              </p>
            </div>
            <button
              onClick={onClose}
              className="flex-shrink-0 rounded-md p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600 dark:hover:bg-gray-800 dark:hover:text-gray-300"
            >
              <X className="h-4 w-4" />
            </button>
          </div>

          {/* Metadata */}
          <dl className="mb-4 grid grid-cols-2 gap-x-6 gap-y-2 text-sm sm:grid-cols-4">
            <div>
              <dt className="text-gray-500 dark:text-gray-400">Status</dt>
              <dd className="mt-0.5">
                <StatusBadge status={changeset.status as ChangesetStatus} />
              </dd>
            </div>
            <div>
              <dt className="text-gray-500 dark:text-gray-400">Policy</dt>
              <dd className="mt-0.5 font-medium text-gray-900 dark:text-white">
                {formatPolicy(changeset.rollback_policy)}
              </dd>
            </div>
            <div>
              <dt className="text-gray-500 dark:text-gray-400">Created by</dt>
              <dd className="mt-0.5 font-medium text-gray-900 dark:text-white">
                {changeset.created_by}
              </dd>
            </div>
            <div>
              <dt className="text-gray-500 dark:text-gray-400">Created</dt>
              <dd className="mt-0.5 font-medium text-gray-900 dark:text-white">
                {formatDate(changeset.created_at)}
              </dd>
            </div>
            {changeset.description && (
              <div className="col-span-2 sm:col-span-4">
                <dt className="text-gray-500 dark:text-gray-400">Description</dt>
                <dd className="mt-0.5 text-gray-900 dark:text-white">
                  {changeset.description}
                </dd>
              </div>
            )}
          </dl>

          {/* Items */}
          <div className="min-h-0 flex-1 overflow-y-auto">
            <p className="mb-2 text-sm font-medium text-gray-700 dark:text-gray-300">
              Items ({changeset.items.length})
            </p>
            {changeset.items.length === 0 ? (
              <p className="text-sm text-gray-400">No items.</p>
            ) : (
              <div className="space-y-2">
                {changeset.items.map((item: ChangesetItem, i: number) => {
                  const opCfg = OPERATION_CONFIG[item.operation] ?? {
                    label: item.operation.toUpperCase(),
                    className: "bg-gray-100 text-gray-600",
                  };
                  return (
                    <div
                      key={i}
                      className="rounded-lg border border-gray-200 p-3 dark:border-gray-700"
                    >
                      <div className="mb-2 flex items-center gap-2">
                        <span
                          className={`rounded px-1.5 py-0.5 text-xs font-bold ${opCfg.className}`}
                        >
                          {opCfg.label}
                        </span>
                        <span className="truncate font-mono text-xs text-gray-500 dark:text-gray-400">
                          {item.record_id}
                        </span>
                      </div>
                      {(item.before_value !== null || item.after_value !== null) && (
                        <div className="grid grid-cols-2 gap-2 text-xs">
                          {item.before_value !== null && (
                            <div>
                              <p className="mb-1 text-gray-500 dark:text-gray-400">
                                Before
                              </p>
                              <pre className="overflow-x-auto rounded bg-gray-50 p-2 font-mono text-gray-800 dark:bg-gray-800 dark:text-gray-200">
                                {JSON.stringify(item.before_value, null, 2)}
                              </pre>
                            </div>
                          )}
                          {item.after_value !== null && (
                            <div>
                              <p className="mb-1 text-gray-500 dark:text-gray-400">
                                After
                              </p>
                              <pre className="overflow-x-auto rounded bg-gray-50 p-2 font-mono text-gray-800 dark:bg-gray-800 dark:text-gray-200">
                                {JSON.stringify(item.after_value, null, 2)}
                              </pre>
                            </div>
                          )}
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </DialogPanel>
      </div>
    </Dialog>
  );
}

// ── Action button ─────────────────────────────────────────────────────────────

interface ActionButtonProps {
  changesetId: string;
  status: ChangesetStatus;
}

function ActionButton({ changesetId, status }: ActionButtonProps) {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: () => {
      if (status === "draft") return validateChangeset(changesetId);
      if (status === "validated") return applyChangeset(changesetId);
      if (status === "applied") return rollbackChangeset(changesetId);
      return Promise.reject(new Error("no action"));
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["changesets"] });
    },
  });

  if (isInProgress(status)) {
    return (
      <span className="flex items-center gap-1 text-xs text-amber-600 dark:text-amber-400">
        <Loader2 className="h-3.5 w-3.5 animate-spin" />
        In progress
      </span>
    );
  }

  const actions: Partial<
    Record<ChangesetStatus, { label: string; variant: "blue" | "green" | "red" }>
  > = {
    draft: { label: "Validate", variant: "blue" },
    validated: { label: "Apply", variant: "green" },
    applied: { label: "Rollback", variant: "red" },
  };

  const action = actions[status];
  if (!action) return null;

  const variantClass = {
    blue: "bg-blue-600 hover:bg-blue-700 text-white",
    green: "bg-green-600 hover:bg-green-700 text-white",
    red: "bg-red-100 hover:bg-red-200 text-red-700 dark:bg-red-900/30 dark:hover:bg-red-900/50 dark:text-red-400",
  }[action.variant];

  return (
    <button
      onClick={(e) => {
        e.stopPropagation();
        mutation.mutate();
      }}
      disabled={mutation.isPending}
      className={`flex items-center gap-1 rounded px-2.5 py-1 text-xs font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-60 ${variantClass}`}
    >
      {mutation.isPending ? (
        <Loader2 className="h-3 w-3 animate-spin" />
      ) : status === "applied" ? null : (
        <CheckCircle2 className="h-3 w-3" />
      )}
      {action.label}
    </button>
  );
}

// ── ChangesetsPage ────────────────────────────────────────────────────────────

export function ChangesetsPage() {
  const [detailTarget, setDetailTarget] = useState<Changeset | null>(null);

  const {
    data: changesets,
    isLoading,
    isError,
    error,
    dataUpdatedAt,
  } = useQuery({
    queryKey: ["changesets"],
    queryFn: getChangesets,
    refetchInterval: 5000,
  });

  const hasInProgress = changesets?.some((c) =>
    isInProgress(c.status as ChangesetStatus)
  );

  const errorMessage =
    error instanceof Error ? error.message : "Failed to load changesets.";

  return (
    <div className="space-y-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <h1 className="text-xl font-semibold text-gray-900 dark:text-white">
            Changesets
          </h1>
          {hasInProgress && (
            <span className="flex items-center gap-1 rounded-full bg-amber-100 px-2 py-0.5 text-xs font-medium text-amber-700 dark:bg-amber-900/30 dark:text-amber-400">
              <RefreshCw className="h-3 w-3 animate-spin" />
              Processing
            </span>
          )}
        </div>
        {dataUpdatedAt > 0 && (
          <p className="text-xs text-gray-400">
            Updated {new Date(dataUpdatedAt).toLocaleTimeString()}
          </p>
        )}
      </div>

      {/* Content */}
      {isLoading && <TableSkeleton />}
      {isError && <ErrorBanner message={errorMessage} />}

      {!isLoading && !isError && changesets && (
        changesets.length === 0 ? (
          <div className="rounded-lg border border-dashed border-gray-300 py-16 text-center dark:border-gray-700">
            <p className="text-sm text-gray-500 dark:text-gray-400">
              No changesets yet. Changesets are created when DNS record changes
              are proposed.
            </p>
          </div>
        ) : (
          <div className="overflow-hidden rounded-lg border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-900">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-gray-200 bg-gray-50 dark:border-gray-700 dark:bg-gray-800">
                  <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-400">
                    Status
                  </th>
                  <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-400">
                    Description
                  </th>
                  <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-400">
                    Items
                  </th>
                  <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-400">
                    Policy
                  </th>
                  <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-400">
                    Created
                  </th>
                  <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-400">
                    Action
                  </th>
                  <th className="px-4 py-3" />
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
                {changesets.map((cs) => (
                  <tr
                    key={cs.id}
                    onClick={() => setDetailTarget(cs)}
                    className="cursor-pointer hover:bg-gray-50 dark:hover:bg-gray-800"
                  >
                    <td className="px-4 py-3">
                      <StatusBadge status={cs.status as ChangesetStatus} />
                    </td>
                    <td className="max-w-xs px-4 py-3 text-gray-700 dark:text-gray-300">
                      {cs.description ?? (
                        <span className="text-gray-400">—</span>
                      )}
                    </td>
                    <td className="px-4 py-3 tabular-nums text-gray-600 dark:text-gray-400">
                      {cs.items.length}
                    </td>
                    <td className="px-4 py-3 text-gray-600 dark:text-gray-400">
                      {formatPolicy(cs.rollback_policy)}
                    </td>
                    <td className="whitespace-nowrap px-4 py-3 text-gray-500 dark:text-gray-400">
                      {formatDate(cs.created_at)}
                    </td>
                    <td
                      className="px-4 py-3"
                      onClick={(e) => e.stopPropagation()}
                    >
                      <ActionButton
                        changesetId={cs.id}
                        status={cs.status as ChangesetStatus}
                      />
                    </td>
                    <td className="px-4 py-3 text-right">
                      <ChevronRight className="ml-auto h-4 w-4 text-gray-400" />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )
      )}

      <DetailModal
        changeset={detailTarget}
        onClose={() => setDetailTarget(null)}
      />
    </div>
  );
}
