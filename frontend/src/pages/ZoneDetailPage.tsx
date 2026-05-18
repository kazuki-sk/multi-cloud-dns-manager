import { useState } from "react";
import { useParams, useNavigate } from "react-router-dom";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Dialog, DialogPanel, DialogTitle } from "@headlessui/react";
import {
  ChevronLeft,
  Plus,
  Trash2,
  Pencil,
  AlertCircle,
  Loader2,
  X,
  Upload,
  CheckCircle2,
  AlertTriangle,
  WifiOff,
  Wifi,
  Clock,
} from "lucide-react";
import { getZone, updateZone, deleteZone, getZoneSyncStates } from "../api/zones";
import { getRecords, createRecord, updateRecord, deleteRecord, pushPendingRecords } from "../api/records";
import type { CreateRecordParams, UpdateRecordParams } from "../api/records";
import { getBindings, createBinding, deleteBinding } from "../api/bindings";
import type { CreateBindingParams } from "../api/bindings";
import { getProviders } from "../api/providers";
import type { DnsRecord, ObserveStatus, ProviderBinding, ProviderStatus, ProviderType, RecordType, RecordSyncStatus, ZoneSyncState } from "../types";

// ── Constants ─────────────────────────────────────────────────────────────────

const RECORD_TYPES: RecordType[] = ["A", "AAAA", "CNAME", "MX", "TXT", "NS"];

const PROVIDER_LABELS: Record<ProviderType, string> = {
  route53: "Route 53",
  azuredns: "Azure DNS",
  gcloud: "Google Cloud DNS",
};

// ── Shared helpers ────────────────────────────────────────────────────────────

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

function RecordTypeBadge({ type }: { type: RecordType }) {
  const colours: Record<RecordType, string> = {
    A: "bg-blue-100 text-blue-700 dark:bg-blue-900/40 dark:text-blue-400",
    AAAA: "bg-violet-100 text-violet-700 dark:bg-violet-900/40 dark:text-violet-400",
    CNAME: "bg-amber-100 text-amber-700 dark:bg-amber-900/40 dark:text-amber-400",
    MX: "bg-green-100 text-green-700 dark:bg-green-900/40 dark:text-green-400",
    TXT: "bg-gray-100 text-gray-700 dark:bg-gray-700 dark:text-gray-300",
    NS: "bg-rose-100 text-rose-700 dark:bg-rose-900/40 dark:text-rose-400",
  };
  return (
    <span className={`inline-block rounded px-1.5 py-0.5 text-xs font-medium ${colours[type]}`}>
      {type}
    </span>
  );
}

function SyncStatusBadge({ status }: { status: RecordSyncStatus }) {
  if (status === "synced") return null;
  if (status === "pending_delete") {
    return (
      <span className="inline-flex items-center gap-1 rounded bg-red-100 px-1.5 py-0.5 text-xs font-medium text-red-700 dark:bg-red-900/30 dark:text-red-400">
        <span className="h-1.5 w-1.5 rounded-full bg-red-500 dark:bg-red-400" />
        Pending Delete
      </span>
    );
  }
  return (
    <span className="inline-flex items-center gap-1 rounded bg-amber-100 px-1.5 py-0.5 text-xs font-medium text-amber-700 dark:bg-amber-900/30 dark:text-amber-400">
      <span className="h-1.5 w-1.5 rounded-full bg-amber-500 dark:bg-amber-400" />
      Pending
    </span>
  );
}

// Aggregate multiple sync states (across providers) into the worst status.
function aggregateObserveStatus(states: ZoneSyncState[]): ObserveStatus | null {
  if (states.length === 0) return null;
  const rank: Record<ObserveStatus, number> = { in_sync: 0, syncing: 1, drift: 2, sync_failed: 3 };
  return states.reduce<ObserveStatus>((worst, s) => {
    const status = s.status as ObserveStatus;
    return (rank[status] ?? 0) > (rank[worst] ?? 0) ? status : worst;
  }, "in_sync");
}

function ObserveStatusBadge({ status }: { status: ObserveStatus | null }) {
  if (status === null) {
    return (
      <span className="inline-flex items-center gap-1 text-xs text-gray-400 dark:text-gray-500">
        <Clock className="h-3 w-3" />
        Not observed
      </span>
    );
  }
  const cfg: Record<ObserveStatus, { icon: React.ReactNode; label: string; cls: string }> = {
    in_sync: {
      icon: <Wifi className="h-3 w-3" />,
      label: "In sync",
      cls: "text-green-600 dark:text-green-400",
    },
    syncing: {
      icon: <Loader2 className="h-3 w-3 animate-spin" />,
      label: "Syncing",
      cls: "text-blue-600 dark:text-blue-400",
    },
    drift: {
      icon: <AlertTriangle className="h-3 w-3" />,
      label: "Drift",
      cls: "text-amber-600 dark:text-amber-400",
    },
    sync_failed: {
      icon: <WifiOff className="h-3 w-3" />,
      label: "Sync failed",
      cls: "text-red-600 dark:text-red-400",
    },
  };
  const c = cfg[status];
  return (
    <span className={`inline-flex items-center gap-1 text-xs font-medium ${c.cls}`}>
      {c.icon}
      {c.label}
    </span>
  );
}

const BINDING_STATUS_STYLES: Record<ProviderStatus, string> = {
  active: "bg-green-100 text-green-700 dark:bg-green-900/40 dark:text-green-400",
  paused: "bg-amber-100 text-amber-700 dark:bg-amber-900/40 dark:text-amber-400",
  error: "bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-400",
};

function StatusBadge({ status }: { status: ProviderStatus }) {
  return (
    <span className={`inline-block rounded px-1.5 py-0.5 text-xs font-medium ${BINDING_STATUS_STYLES[status]}`}>
      {status}
    </span>
  );
}

// ── Edit Zone Modal ───────────────────────────────────────────────────────────

interface EditZoneModalProps {
  open: boolean;
  onClose: () => void;
  zoneId: string;
  currentName: string;
  currentTtl: number;
}

function EditZoneModal({ open, onClose, zoneId, currentName, currentTtl }: EditZoneModalProps) {
  const queryClient = useQueryClient();
  const [name, setName] = useState(currentName);
  const [defaultTtl, setDefaultTtl] = useState(currentTtl);
  const [formError, setFormError] = useState<string | null>(null);

  const mutation = useMutation({
    mutationFn: (params: { name: string; default_ttl: number }) =>
      updateZone(zoneId, params),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["zone", zoneId] });
      queryClient.invalidateQueries({ queryKey: ["zones"] });
      handleClose();
    },
    onError: (err: unknown) => {
      setFormError(err instanceof Error ? err.message : "Failed to update zone.");
    },
  });

  function handleClose() {
    setName(currentName);
    setDefaultTtl(currentTtl);
    setFormError(null);
    mutation.reset();
    onClose();
  }

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setFormError(null);
    if (!name.trim()) { setFormError("Zone name is required."); return; }
    if (defaultTtl < 1) { setFormError("Default TTL must be at least 1."); return; }
    mutation.mutate({ name: name.trim(), default_ttl: defaultTtl });
  }

  return (
    <Dialog open={open} onClose={handleClose} className="relative z-50">
      <div className="fixed inset-0 bg-black/40 dark:bg-black/60" aria-hidden="true" />
      <div className="fixed inset-0 flex items-center justify-center p-4">
        <DialogPanel className="w-full max-w-md rounded-xl bg-white p-6 shadow-xl dark:bg-gray-900">
          <div className="mb-4 flex items-center justify-between">
            <DialogTitle className="text-base font-semibold text-gray-900 dark:text-white">Edit Zone</DialogTitle>
            <button onClick={handleClose} className="rounded-md p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600 dark:hover:bg-gray-800 dark:hover:text-gray-300">
              <X className="h-4 w-4" />
            </button>
          </div>
          <form onSubmit={handleSubmit} className="space-y-4">
            <div>
              <label htmlFor="ez-name" className="mb-1 block text-sm font-medium text-gray-700 dark:text-gray-300">Zone Name</label>
              <input id="ez-name" type="text" value={name} onChange={(e) => setName(e.target.value)}
                className="w-full rounded-md border border-gray-300 bg-white px-3 py-2 text-sm text-gray-900 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-800 dark:text-white" />
            </div>
            <div>
              <label htmlFor="ez-ttl" className="mb-1 block text-sm font-medium text-gray-700 dark:text-gray-300">Default TTL (seconds)</label>
              <input id="ez-ttl" type="number" value={defaultTtl} onChange={(e) => setDefaultTtl(Number(e.target.value))} min={1}
                className="w-full rounded-md border border-gray-300 bg-white px-3 py-2 text-sm text-gray-900 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-800 dark:text-white" />
            </div>
            {formError && <p className="text-sm text-red-600 dark:text-red-400">{formError}</p>}
            <div className="flex justify-end gap-2 pt-2">
              <button type="button" onClick={handleClose} className="rounded-md px-4 py-2 text-sm font-medium text-gray-600 hover:bg-gray-100 dark:text-gray-400 dark:hover:bg-gray-800">Cancel</button>
              <button type="submit" disabled={mutation.isPending}
                className="flex items-center gap-2 rounded-md bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:cursor-not-allowed disabled:opacity-60">
                {mutation.isPending && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                Save
              </button>
            </div>
          </form>
        </DialogPanel>
      </div>
    </Dialog>
  );
}

// ── Add Record Modal ──────────────────────────────────────────────────────────

interface AddRecordModalProps {
  open: boolean;
  onClose: () => void;
  zoneId: string;
  defaultTtl: number;
}

function AddRecordModal({ open, onClose, zoneId, defaultTtl }: AddRecordModalProps) {
  const queryClient = useQueryClient();
  const [name, setName] = useState("");
  const [recordType, setRecordType] = useState<RecordType>("A");
  const [ttl, setTtl] = useState(defaultTtl);
  const [values, setValues] = useState<string[]>([""]);
  const [formError, setFormError] = useState<string | null>(null);

  const mutation = useMutation({
    mutationFn: (params: CreateRecordParams) => createRecord(zoneId, params),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["records", zoneId] });
      handleClose();
    },
    onError: (err: unknown) => {
      setFormError(err instanceof Error ? err.message : "Failed to add record.");
    },
  });

  function handleClose() {
    setName(""); setRecordType("A"); setTtl(defaultTtl); setValues([""]); setFormError(null);
    mutation.reset(); onClose();
  }

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault(); setFormError(null);
    if (!name.trim()) { setFormError("Name is required."); return; }
    if (ttl < 1) { setFormError("TTL must be at least 1."); return; }
    const filtered = values.map((v) => v.trim()).filter(Boolean);
    if (filtered.length === 0) { setFormError("At least one value is required."); return; }
    mutation.mutate({ name: name.trim(), record_type: recordType, ttl, values: filtered });
  };

  return (
    <Dialog open={open} onClose={handleClose} className="relative z-50">
      <div className="fixed inset-0 bg-black/40 dark:bg-black/60" aria-hidden="true" />
      <div className="fixed inset-0 flex items-center justify-center p-4">
        <DialogPanel className="w-full max-w-lg rounded-xl bg-white p-6 shadow-xl dark:bg-gray-900">
          <div className="mb-4 flex items-center justify-between">
            <DialogTitle className="text-base font-semibold text-gray-900 dark:text-white">Add Record</DialogTitle>
            <button onClick={handleClose} className="rounded-md p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600 dark:hover:bg-gray-800 dark:hover:text-gray-300"><X className="h-4 w-4" /></button>
          </div>
          <form onSubmit={handleSubmit} className="space-y-4">
            <div>
              <label htmlFor="rec-name" className="mb-1 block text-sm font-medium text-gray-700 dark:text-gray-300">Name</label>
              <input id="rec-name" type="text" value={name} onChange={(e) => setName(e.target.value)} placeholder="www" autoFocus
                className="w-full rounded-md border border-gray-300 bg-white px-3 py-2 text-sm text-gray-900 placeholder-gray-400 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-800 dark:text-white dark:placeholder-gray-500" />
            </div>
            <div className="grid grid-cols-2 gap-3">
              <div>
                <label htmlFor="rec-type" className="mb-1 block text-sm font-medium text-gray-700 dark:text-gray-300">Type</label>
                <select id="rec-type" value={recordType} onChange={(e) => setRecordType(e.target.value as RecordType)}
                  className="w-full rounded-md border border-gray-300 bg-white px-3 py-2 text-sm text-gray-900 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-800 dark:text-white">
                  {RECORD_TYPES.map((t) => <option key={t} value={t}>{t}</option>)}
                </select>
              </div>
              <div>
                <label htmlFor="rec-ttl" className="mb-1 block text-sm font-medium text-gray-700 dark:text-gray-300">TTL (seconds)</label>
                <input id="rec-ttl" type="number" value={ttl} onChange={(e) => setTtl(Number(e.target.value))} min={1}
                  className="w-full rounded-md border border-gray-300 bg-white px-3 py-2 text-sm text-gray-900 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-800 dark:text-white" />
              </div>
            </div>
            <div>
              <span className="mb-1 block text-sm font-medium text-gray-700 dark:text-gray-300">Values</span>
              <div className="space-y-2">
                {values.map((v, i) => (
                  <div key={i} className="flex items-center gap-2">
                    <input type="text" value={v} onChange={(e) => setValues((prev) => prev.map((s, j) => j === i ? e.target.value : s))}
                      placeholder="e.g. 192.0.2.1"
                      className="flex-1 rounded-md border border-gray-300 bg-white px-3 py-2 text-sm text-gray-900 placeholder-gray-400 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-800 dark:text-white dark:placeholder-gray-500" />
                    {values.length > 1 && (
                      <button type="button" onClick={() => setValues((prev) => prev.filter((_, j) => j !== i))}
                        className="rounded-md p-1.5 text-gray-400 hover:bg-gray-100 hover:text-red-500 dark:hover:bg-gray-800"><X className="h-4 w-4" /></button>
                    )}
                  </div>
                ))}
              </div>
              <button type="button" onClick={() => setValues((v) => [...v, ""])}
                className="mt-2 text-xs font-medium text-blue-600 hover:underline dark:text-blue-400">+ Add another value</button>
            </div>
            {formError && <p className="text-sm text-red-600 dark:text-red-400">{formError}</p>}
            <div className="flex justify-end gap-2 pt-2">
              <button type="button" onClick={handleClose} className="rounded-md px-4 py-2 text-sm font-medium text-gray-600 hover:bg-gray-100 dark:text-gray-400 dark:hover:bg-gray-800">Cancel</button>
              <button type="submit" disabled={mutation.isPending}
                className="flex items-center gap-2 rounded-md bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:cursor-not-allowed disabled:opacity-60">
                {mutation.isPending && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                Add Record
              </button>
            </div>
          </form>
        </DialogPanel>
      </div>
    </Dialog>
  );
}

// ── Edit Record Modal ─────────────────────────────────────────────────────────

interface EditRecordModalProps {
  record: DnsRecord | null;
  onClose: () => void;
  zoneId: string;
}

function EditRecordModal({ record, onClose, zoneId }: EditRecordModalProps) {
  const queryClient = useQueryClient();
  const [ttl, setTtl] = useState(record?.ttl ?? 300);
  const [values, setValues] = useState<string[]>(record?.values ?? [""]);
  const [formError, setFormError] = useState<string | null>(null);

  const mutation = useMutation({
    mutationFn: (params: UpdateRecordParams) => updateRecord(zoneId, record!.id, params),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["records", zoneId] });
      handleClose();
    },
    onError: (err: unknown) => {
      setFormError(err instanceof Error ? err.message : "Failed to update record.");
    },
  });

  function handleClose() {
    setTtl(record?.ttl ?? 300);
    setValues(record?.values ?? [""]);
    setFormError(null);
    mutation.reset();
    onClose();
  }

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault(); setFormError(null);
    if (ttl < 1) { setFormError("TTL must be at least 1."); return; }
    const filtered = values.map((v) => v.trim()).filter(Boolean);
    if (filtered.length === 0) { setFormError("At least one value is required."); return; }
    mutation.mutate({ ttl, values: filtered });
  };

  return (
    <Dialog open={record !== null} onClose={handleClose} className="relative z-50">
      <div className="fixed inset-0 bg-black/40 dark:bg-black/60" aria-hidden="true" />
      <div className="fixed inset-0 flex items-center justify-center p-4">
        <DialogPanel className="w-full max-w-lg rounded-xl bg-white p-6 shadow-xl dark:bg-gray-900">
          <div className="mb-4 flex items-center justify-between">
            <div>
              <DialogTitle className="text-base font-semibold text-gray-900 dark:text-white">Edit Record</DialogTitle>
              {record && (
                <p className="mt-0.5 font-mono text-xs text-gray-400">
                  {record.name} ({record.record_type})
                </p>
              )}
            </div>
            <button onClick={handleClose} className="rounded-md p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600 dark:hover:bg-gray-800 dark:hover:text-gray-300"><X className="h-4 w-4" /></button>
          </div>
          <form onSubmit={handleSubmit} className="space-y-4">
            <div>
              <label htmlFor="er-ttl" className="mb-1 block text-sm font-medium text-gray-700 dark:text-gray-300">TTL (seconds)</label>
              <input id="er-ttl" type="number" value={ttl} onChange={(e) => setTtl(Number(e.target.value))} min={1}
                className="w-full rounded-md border border-gray-300 bg-white px-3 py-2 text-sm text-gray-900 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-800 dark:text-white" />
            </div>
            <div>
              <span className="mb-1 block text-sm font-medium text-gray-700 dark:text-gray-300">Values</span>
              <div className="space-y-2">
                {values.map((v, i) => (
                  <div key={i} className="flex items-center gap-2">
                    <input type="text" value={v} onChange={(e) => setValues((prev) => prev.map((s, j) => j === i ? e.target.value : s))}
                      className="flex-1 rounded-md border border-gray-300 bg-white px-3 py-2 text-sm text-gray-900 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-800 dark:text-white" />
                    {values.length > 1 && (
                      <button type="button" onClick={() => setValues((prev) => prev.filter((_, j) => j !== i))}
                        className="rounded-md p-1.5 text-gray-400 hover:bg-gray-100 hover:text-red-500 dark:hover:bg-gray-800"><X className="h-4 w-4" /></button>
                    )}
                  </div>
                ))}
              </div>
              <button type="button" onClick={() => setValues((v) => [...v, ""])}
                className="mt-2 text-xs font-medium text-blue-600 hover:underline dark:text-blue-400">+ Add another value</button>
            </div>
            {formError && <p className="text-sm text-red-600 dark:text-red-400">{formError}</p>}
            <div className="flex justify-end gap-2 pt-2">
              <button type="button" onClick={handleClose} className="rounded-md px-4 py-2 text-sm font-medium text-gray-600 hover:bg-gray-100 dark:text-gray-400 dark:hover:bg-gray-800">Cancel</button>
              <button type="submit" disabled={mutation.isPending}
                className="flex items-center gap-2 rounded-md bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:cursor-not-allowed disabled:opacity-60">
                {mutation.isPending && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                Save
              </button>
            </div>
          </form>
        </DialogPanel>
      </div>
    </Dialog>
  );
}

// ── Confirm Delete Dialog (generic) ──────────────────────────────────────────

interface ConfirmDeleteProps {
  open: boolean;
  title: string;
  description: string;
  onClose: () => void;
  onConfirm: () => void;
  isPending: boolean;
  error: string | null;
}

function ConfirmDelete({ open, title, description, onClose, onConfirm, isPending, error }: ConfirmDeleteProps) {
  return (
    <Dialog open={open} onClose={onClose} className="relative z-50">
      <div className="fixed inset-0 bg-black/40 dark:bg-black/60" aria-hidden="true" />
      <div className="fixed inset-0 flex items-center justify-center p-4">
        <DialogPanel className="w-full max-w-sm rounded-xl bg-white p-6 shadow-xl dark:bg-gray-900">
          <DialogTitle className="mb-2 text-base font-semibold text-gray-900 dark:text-white">{title}</DialogTitle>
          <p className="mb-4 text-sm text-gray-600 dark:text-gray-400">{description}</p>
          {error && <p className="mb-3 text-sm text-red-600 dark:text-red-400">{error}</p>}
          <div className="flex justify-end gap-2">
            <button onClick={onClose} className="rounded-md px-4 py-2 text-sm font-medium text-gray-600 hover:bg-gray-100 dark:text-gray-400 dark:hover:bg-gray-800">Cancel</button>
            <button onClick={onConfirm} disabled={isPending}
              className="flex items-center gap-2 rounded-md bg-red-600 px-4 py-2 text-sm font-medium text-white hover:bg-red-700 disabled:cursor-not-allowed disabled:opacity-60">
              {isPending && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
              Delete
            </button>
          </div>
        </DialogPanel>
      </div>
    </Dialog>
  );
}

// ── Add Binding Modal ─────────────────────────────────────────────────────────

interface AddBindingModalProps {
  open: boolean;
  onClose: () => void;
  zoneId: string;
}

function AddBindingModal({ open, onClose, zoneId }: AddBindingModalProps) {
  const queryClient = useQueryClient();
  const [providerId, setProviderId] = useState("");
  const [providerZoneId, setProviderZoneId] = useState("");
  const [formError, setFormError] = useState<string | null>(null);

  const { data: providers } = useQuery({ queryKey: ["providers"], queryFn: getProviders });

  const mutation = useMutation({
    mutationFn: (params: CreateBindingParams) => createBinding(zoneId, params),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["bindings", zoneId] });
      handleClose();
    },
    onError: (err: unknown) => {
      setFormError(err instanceof Error ? err.message : "Failed to add binding.");
    },
  });

  function handleClose() {
    setProviderId(""); setProviderZoneId(""); setFormError(null); mutation.reset(); onClose();
  }

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault(); setFormError(null);
    if (!providerId) { setFormError("Please select a provider."); return; }
    if (!providerZoneId.trim()) { setFormError("Provider Zone ID is required."); return; }
    mutation.mutate({ provider_id: providerId, provider_zone_id: providerZoneId.trim() });
  }

  const selectedProvider = providers?.find((p) => p.id === providerId);

  return (
    <Dialog open={open} onClose={handleClose} className="relative z-50">
      <div className="fixed inset-0 bg-black/40 dark:bg-black/60" aria-hidden="true" />
      <div className="fixed inset-0 flex items-center justify-center p-4">
        <DialogPanel className="w-full max-w-md rounded-xl bg-white p-6 shadow-xl dark:bg-gray-900">
          <div className="mb-4 flex items-center justify-between">
            <DialogTitle className="text-base font-semibold text-gray-900 dark:text-white">Add Binding</DialogTitle>
            <button onClick={handleClose} className="rounded-md p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600 dark:hover:bg-gray-800 dark:hover:text-gray-300"><X className="h-4 w-4" /></button>
          </div>
          <form onSubmit={handleSubmit} className="space-y-4">
            <div>
              <label htmlFor="bind-provider" className="mb-1 block text-sm font-medium text-gray-700 dark:text-gray-300">Provider</label>
              <select id="bind-provider" value={providerId} onChange={(e) => setProviderId(e.target.value)}
                className="w-full rounded-md border border-gray-300 bg-white px-3 py-2 text-sm text-gray-900 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-800 dark:text-white">
                <option value="" disabled>Select a provider…</option>
                {(providers ?? []).map((p) => (
                  <option key={p.id} value={p.id}>{p.name} ({PROVIDER_LABELS[p.provider_type]})</option>
                ))}
              </select>
            </div>
            <div>
              <label htmlFor="bind-zone-id" className="mb-1 block text-sm font-medium text-gray-700 dark:text-gray-300">Provider Zone ID</label>
              <input id="bind-zone-id" type="text" value={providerZoneId} onChange={(e) => setProviderZoneId(e.target.value)}
                placeholder={
                  selectedProvider?.provider_type === "route53" ? "e.g. Z1PA6795UKMFR9"
                  : selectedProvider?.provider_type === "azuredns" ? "e.g. example.com"
                  : "e.g. example-com"
                }
                className="w-full rounded-md border border-gray-300 bg-white px-3 py-2 text-sm text-gray-900 placeholder-gray-400 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-800 dark:text-white dark:placeholder-gray-500" />
            </div>
            {formError && <p className="text-sm text-red-600 dark:text-red-400">{formError}</p>}
            <div className="flex justify-end gap-2 pt-2">
              <button type="button" onClick={handleClose} className="rounded-md px-4 py-2 text-sm font-medium text-gray-600 hover:bg-gray-100 dark:text-gray-400 dark:hover:bg-gray-800">Cancel</button>
              <button type="submit" disabled={mutation.isPending}
                className="flex items-center gap-2 rounded-md bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:cursor-not-allowed disabled:opacity-60">
                {mutation.isPending && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                Add Binding
              </button>
            </div>
          </form>
        </DialogPanel>
      </div>
    </Dialog>
  );
}

// ── ZoneDetailPage ────────────────────────────────────────────────────────────

export function ZoneDetailPage() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const [editZoneOpen, setEditZoneOpen] = useState(false);
  const [addModalOpen, setAddModalOpen] = useState(false);
  const [addBindingModalOpen, setAddBindingModalOpen] = useState(false);
  const [deleteRecordTarget, setDeleteRecordTarget] = useState<DnsRecord | null>(null);
  const [editRecordTarget, setEditRecordTarget] = useState<DnsRecord | null>(null);
  const [deleteBindingTarget, setDeleteBindingTarget] = useState<ProviderBinding | null>(null);
  const [deleteZoneOpen, setDeleteZoneOpen] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [pushError, setPushError] = useState<string | null>(null);

  const { data: zone } = useQuery({
    queryKey: ["zone", id],
    queryFn: () => getZone(id!),
    enabled: !!id,
  });

  const { data: records, isLoading: recordsLoading, isError: recordsIsError, error: recordsErr } = useQuery({
    queryKey: ["records", id],
    queryFn: () => getRecords(id!),
    enabled: !!id,
  });

  const { data: bindings, isLoading: bindingsLoading, isError: bindingsIsError, error: bindingsErr } = useQuery({
    queryKey: ["bindings", id],
    queryFn: () => getBindings(id!),
    enabled: !!id,
  });

  const { data: syncStates } = useQuery({
    queryKey: ["zone-sync-states", id],
    queryFn: () => getZoneSyncStates(id!),
    enabled: !!id,
    refetchInterval: 30_000,
  });

  // Build a map: record_id → ZoneSyncState[]
  const syncMap = new Map<string, ZoneSyncState[]>();
  for (const s of syncStates ?? []) {
    const arr = syncMap.get(s.record_id) ?? [];
    arr.push(s);
    syncMap.set(s.record_id, arr);
  }

  const pendingCount = records?.filter((r) => r.status === "pending" || r.status === "pending_delete").length ?? 0;

  // ── Mutations ─────────────────────────────────────────────────────────────

  const pushMutation = useMutation({
    mutationFn: () => pushPendingRecords(id!),
    onSuccess: () => {
      setPushError(null);
      queryClient.invalidateQueries({ queryKey: ["records", id] });
      queryClient.invalidateQueries({ queryKey: ["changesets"] });
    },
    onError: (err: unknown) => {
      setPushError(err instanceof Error ? err.message : "Failed to push records.");
    },
  });

  const deleteRecordMutation = useMutation({
    mutationFn: ({ recordId }: { recordId: string }) => deleteRecord(id!, recordId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["records", id] });
      setDeleteRecordTarget(null);
      setDeleteError(null);
    },
    onError: (err: unknown) => {
      setDeleteError(err instanceof Error ? err.message : "Failed to delete record.");
    },
  });

  const deleteBindingMutation = useMutation({
    mutationFn: ({ bindingId }: { bindingId: string }) => deleteBinding(id!, bindingId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["bindings", id] });
      setDeleteBindingTarget(null);
      setDeleteError(null);
    },
    onError: (err: unknown) => {
      setDeleteError(err instanceof Error ? err.message : "Failed to delete binding.");
    },
  });

  const deleteZoneMutation = useMutation({
    mutationFn: () => deleteZone(id!),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["zones"] });
      navigate("/zones");
    },
    onError: (err: unknown) => {
      setDeleteError(err instanceof Error ? err.message : "Failed to delete zone.");
    },
  });

  const recordsErrorMsg = recordsErr instanceof Error ? recordsErr.message : "Failed to load records.";
  const bindingsErrorMsg = bindingsErr instanceof Error ? bindingsErr.message : "Failed to load bindings.";

  return (
    <div className="space-y-6">
      {/* Back nav */}
      <button onClick={() => navigate("/zones")}
        className="flex items-center gap-1 text-sm text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200">
        <ChevronLeft className="h-4 w-4" />
        Back to Zones
      </button>

      {/* Zone info card */}
      <div className="rounded-lg border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-900">
        {zone ? (
          <div className="flex flex-wrap items-center justify-between gap-4">
            <div className="flex flex-wrap items-center gap-6">
              <div>
                <p className="text-xs text-gray-500 dark:text-gray-400">Name</p>
                <p className="font-medium text-gray-900 dark:text-white">{zone.name}</p>
              </div>
              <div>
                <p className="text-xs text-gray-500 dark:text-gray-400">Default TTL</p>
                <p className="tabular-nums font-medium text-gray-900 dark:text-white">{zone.default_ttl.toLocaleString()} s</p>
              </div>
            </div>
            <button onClick={() => setEditZoneOpen(true)}
              className="flex items-center gap-1.5 rounded-md px-3 py-1.5 text-sm font-medium text-gray-600 hover:bg-gray-100 dark:text-gray-400 dark:hover:bg-gray-800">
              <Pencil className="h-3.5 w-3.5" />
              Edit
            </button>
          </div>
        ) : (
          <div className="h-8 w-48 animate-pulse rounded bg-gray-100 dark:bg-gray-800" />
        )}
      </div>

      {/* Records section */}
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <h2 className="text-base font-semibold text-gray-900 dark:text-white">Records</h2>
          <div className="flex items-center gap-2">
            {pendingCount > 0 && !pushMutation.isSuccess && (
              <button onClick={() => pushMutation.mutate()} disabled={pushMutation.isPending}
                className="flex items-center gap-1.5 rounded-md bg-green-600 px-3 py-2 text-sm font-medium text-white hover:bg-green-700 disabled:cursor-not-allowed disabled:opacity-60">
                {pushMutation.isPending ? <Loader2 className="h-4 w-4 animate-spin" /> : <Upload className="h-4 w-4" />}
                Set to DNS ({pendingCount})
              </button>
            )}
            {pushMutation.isSuccess && (
              <span className="flex items-center gap-1.5 text-sm font-medium text-green-600 dark:text-green-400">
                <CheckCircle2 className="h-4 w-4" />
                Submitted to DNS
              </span>
            )}
            <button onClick={() => setAddModalOpen(true)}
              className="flex items-center gap-1.5 rounded-md bg-blue-600 px-3 py-2 text-sm font-medium text-white hover:bg-blue-700">
              <Plus className="h-4 w-4" />
              Add Record
            </button>
          </div>
        </div>
        {pushError && <ErrorBanner message={pushError} />}

        {recordsLoading && <TableSkeleton />}
        {recordsIsError && <ErrorBanner message={recordsErrorMsg} />}

        {!recordsLoading && !recordsIsError && records && (
          records.length === 0 ? (
            <div className="rounded-lg border border-dashed border-gray-300 py-10 text-center dark:border-gray-700">
              <p className="text-sm text-gray-500 dark:text-gray-400">
                No records yet.{" "}
                <button onClick={() => setAddModalOpen(true)} className="font-medium text-blue-600 hover:underline dark:text-blue-400">
                  Add the first record.
                </button>
              </p>
            </div>
          ) : (
            <div className="overflow-hidden rounded-lg border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-900">
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b border-gray-200 bg-gray-50 dark:border-gray-700 dark:bg-gray-800">
                    <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-400">Name</th>
                    <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-400">Type</th>
                    <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-400">TTL</th>
                    <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-400">Values</th>
                    <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-400">DNS Sync</th>
                    <th className="px-4 py-3" />
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
                  {records.map((record) => {
                    const states = syncMap.get(record.id) ?? [];
                    const obsStatus = aggregateObserveStatus(states);
                    const isPendingDelete = record.status === "pending_delete";
                    return (
                      <tr key={record.id} className={`hover:bg-gray-50 dark:hover:bg-gray-800 ${isPendingDelete ? "opacity-60" : ""}`}>
                        <td className="px-4 py-3 font-mono text-gray-900 dark:text-white">{record.name}</td>
                        <td className="px-4 py-3">
                          <div className="flex flex-col items-start gap-1">
                            <RecordTypeBadge type={record.record_type} />
                            <SyncStatusBadge status={record.status} />
                          </div>
                        </td>
                        <td className="px-4 py-3 tabular-nums text-gray-600 dark:text-gray-400">{record.ttl.toLocaleString()}</td>
                        <td className="max-w-xs px-4 py-3">
                          <ul className="space-y-0.5">
                            {record.values.map((v, i) => (
                              <li key={i} className="truncate font-mono text-xs text-gray-700 dark:text-gray-300" title={v}>{v}</li>
                            ))}
                          </ul>
                        </td>
                        <td className="px-4 py-3">
                          <ObserveStatusBadge status={obsStatus} />
                        </td>
                        <td className="px-4 py-3 text-right">
                          <div className="flex items-center justify-end gap-1">
                            {!isPendingDelete && (
                              <button onClick={() => setEditRecordTarget(record)}
                                className="rounded-md p-1.5 text-gray-400 hover:bg-gray-100 hover:text-blue-500 dark:hover:bg-gray-800 dark:hover:text-blue-400"
                                title="Edit record">
                                <Pencil className="h-4 w-4" />
                              </button>
                            )}
                            <button onClick={() => { setDeleteError(null); setDeleteRecordTarget(record); }}
                              className="rounded-md p-1.5 text-gray-400 hover:bg-red-50 hover:text-red-500 dark:hover:bg-red-900/20 dark:hover:text-red-400"
                              title="Delete record">
                              <Trash2 className="h-4 w-4" />
                            </button>
                          </div>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          )
        )}
      </div>

      {/* Bindings section */}
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <h2 className="text-base font-semibold text-gray-900 dark:text-white">Bindings</h2>
          <button onClick={() => setAddBindingModalOpen(true)}
            className="flex items-center gap-1.5 rounded-md bg-blue-600 px-3 py-2 text-sm font-medium text-white hover:bg-blue-700">
            <Plus className="h-4 w-4" />
            Add Binding
          </button>
        </div>

        {bindingsLoading && <TableSkeleton />}
        {bindingsIsError && <ErrorBanner message={bindingsErrorMsg} />}

        {!bindingsLoading && !bindingsIsError && bindings && (
          bindings.length === 0 ? (
            <div className="rounded-lg border border-dashed border-gray-300 py-10 text-center dark:border-gray-700">
              <p className="text-sm text-gray-500 dark:text-gray-400">
                No bindings yet.{" "}
                <button onClick={() => setAddBindingModalOpen(true)} className="font-medium text-blue-600 hover:underline dark:text-blue-400">
                  Add the first binding.
                </button>
              </p>
            </div>
          ) : (
            <div className="overflow-hidden rounded-lg border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-900">
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b border-gray-200 bg-gray-50 dark:border-gray-700 dark:bg-gray-800">
                    <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-400">Provider</th>
                    <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-400">Type</th>
                    <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-400">Provider Zone ID</th>
                    <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-400">Status</th>
                    <th className="px-4 py-3" />
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
                  {(bindings as ProviderBinding[]).map((b) => (
                    <tr key={b.id} className="hover:bg-gray-50 dark:hover:bg-gray-800">
                      <td className="px-4 py-3 font-medium text-gray-900 dark:text-white">{b.provider_name}</td>
                      <td className="px-4 py-3 text-gray-600 dark:text-gray-400">{PROVIDER_LABELS[b.provider_type]}</td>
                      <td className="px-4 py-3 font-mono text-gray-700 dark:text-gray-300">{b.provider_zone_id}</td>
                      <td className="px-4 py-3"><StatusBadge status={b.status} /></td>
                      <td className="px-4 py-3 text-right">
                        <button onClick={() => { setDeleteError(null); setDeleteBindingTarget(b); }}
                          className="rounded-md p-1.5 text-gray-400 hover:bg-red-50 hover:text-red-500 dark:hover:bg-red-900/20 dark:hover:text-red-400"
                          title="Delete binding">
                          <Trash2 className="h-4 w-4" />
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )
        )}
      </div>

      {/* Danger zone */}
      <div className="rounded-lg border border-red-200 bg-red-50/50 p-5 dark:border-red-900 dark:bg-red-900/10">
        <h2 className="mb-1 text-sm font-semibold text-red-700 dark:text-red-400">Danger Zone</h2>
        <p className="mb-3 text-xs text-red-600 dark:text-red-500">
          Deleting this zone removes all records, bindings, and sync states permanently.
        </p>
        <button onClick={() => { setDeleteError(null); setDeleteZoneOpen(true); }}
          className="flex items-center gap-1.5 rounded-md border border-red-300 px-3 py-1.5 text-sm font-medium text-red-600 hover:bg-red-100 dark:border-red-700 dark:text-red-400 dark:hover:bg-red-900/30">
          <Trash2 className="h-3.5 w-3.5" />
          Delete Zone
        </button>
      </div>

      {/* Modals */}
      {zone && (
        <EditZoneModal
          open={editZoneOpen}
          onClose={() => setEditZoneOpen(false)}
          zoneId={id!}
          currentName={zone.name}
          currentTtl={zone.default_ttl}
        />
      )}
      <AddRecordModal open={addModalOpen} onClose={() => setAddModalOpen(false)} zoneId={id!} defaultTtl={zone?.default_ttl ?? 300} />
      <EditRecordModal record={editRecordTarget} onClose={() => setEditRecordTarget(null)} zoneId={id!} />
      <AddBindingModal open={addBindingModalOpen} onClose={() => setAddBindingModalOpen(false)} zoneId={id!} />

      <ConfirmDelete
        open={deleteRecordTarget !== null}
        title="Delete Record"
        description={deleteRecordTarget ? `Delete ${deleteRecordTarget.name} (${deleteRecordTarget.record_type})? ${deleteRecordTarget.status === "synced" ? "This record is live — it will be queued for DNS removal on the next Set to DNS." : "This cannot be undone."}` : ""}
        onClose={() => { setDeleteRecordTarget(null); setDeleteError(null); }}
        onConfirm={() => deleteRecordTarget && deleteRecordMutation.mutate({ recordId: deleteRecordTarget.id })}
        isPending={deleteRecordMutation.isPending}
        error={deleteError}
      />

      <ConfirmDelete
        open={deleteBindingTarget !== null}
        title="Delete Binding"
        description={deleteBindingTarget ? `Remove the binding to "${deleteBindingTarget.provider_name}"? Sync states for this binding will also be removed.` : ""}
        onClose={() => { setDeleteBindingTarget(null); setDeleteError(null); }}
        onConfirm={() => deleteBindingTarget && deleteBindingMutation.mutate({ bindingId: deleteBindingTarget.id })}
        isPending={deleteBindingMutation.isPending}
        error={deleteError}
      />

      <ConfirmDelete
        open={deleteZoneOpen}
        title="Delete Zone"
        description={zone ? `Permanently delete zone "${zone.name}"? All records, bindings, and sync states will be removed.` : ""}
        onClose={() => { setDeleteZoneOpen(false); setDeleteError(null); }}
        onConfirm={() => deleteZoneMutation.mutate()}
        isPending={deleteZoneMutation.isPending}
        error={deleteError}
      />
    </div>
  );
}
