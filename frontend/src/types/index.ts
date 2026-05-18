export interface Zone {
  id: string;
  name: string;
  default_ttl: number;
  owner_team_id: string | null;
  created_at: string;
  updated_at: string;
}

export type RecordType = "A" | "AAAA" | "CNAME" | "MX" | "TXT" | "NS";

export type RecordSyncStatus = "pending" | "synced" | "pending_delete";

export interface DnsRecord {
  id: string;
  zone_id: string;
  name: string;
  record_type: RecordType;
  ttl: number;
  values: string[];
  desired_hash: string;
  status: RecordSyncStatus;
  created_at: string;
  updated_at: string;
}

export type ProviderType = "route53" | "azuredns" | "gcloud";
export type ProviderStatus = "active" | "paused" | "error";

export interface Provider {
  id: string;
  name: string;
  provider_type: ProviderType;
  status: ProviderStatus;
  created_at: string;
  updated_at: string;
}

export interface ProviderBinding {
  id: string;
  zone_id: string;
  provider_id: string;
  provider_name: string;
  provider_type: ProviderType;
  provider_zone_id: string;
  status: ProviderStatus;
  created_at: string;
  updated_at: string;
}

export interface ProviderZoneBinding {
  id: string;
  zone_id: string;
  zone_name: string;
  provider_zone_id: string;
  status: ProviderStatus;
  created_at: string;
  updated_at: string;
}

export type ChangesetStatus =
  | "draft"
  | "validated"
  | "applying"
  | "applied"
  | "rolling_back"
  | "rolled_back"
  | "rollback_failed"
  | "frozen";

export interface ChangesetItem {
  record_id: string;
  operation: "create" | "update" | "delete";
  before_value: unknown | null;
  after_value: unknown | null;
}

export interface Changeset {
  id: string;
  created_by: string;
  description: string | null;
  status: ChangesetStatus;
  rollback_policy: string;
  created_at: string;
  updated_at: string;
  items: ChangesetItem[];
}

export interface SyncState {
  record_id: string;
  provider_binding_id: string;
  last_observed_value: string | null;
  last_observed_at: string | null;
  status: "in_sync" | "drift" | "sync_failed";
  last_error: string | null;
  retry_count: number;
  created_at: string;
  updated_at: string;
}

export type ObserveStatus = "in_sync" | "syncing" | "drift" | "sync_failed";

export interface ZoneSyncState {
  record_id: string;
  provider_binding_id: string;
  provider_name: string;
  last_observed_at: string | null;
  status: ObserveStatus;
  last_error: string | null;
  retry_count: number;
}
