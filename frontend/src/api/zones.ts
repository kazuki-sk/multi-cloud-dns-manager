import { apiClient } from "./client";
import type { Zone, ZoneSyncState } from "../types";

export interface CreateZoneParams {
  name: string;
  default_ttl: number;
}

export interface UpdateZoneParams {
  name?: string;
  default_ttl?: number;
}

export async function getZones(): Promise<Zone[]> {
  const res = await apiClient.get<{ zones: Zone[] }>("/zones");
  return res.data.zones;
}

export async function getZone(id: string): Promise<Zone> {
  const res = await apiClient.get<Zone>(`/zones/${id}`);
  return res.data;
}

export async function createZone(params: CreateZoneParams): Promise<Zone> {
  const res = await apiClient.post<Zone>("/zones", params);
  return res.data;
}

export async function updateZone(id: string, params: UpdateZoneParams): Promise<Zone> {
  const res = await apiClient.patch<Zone>(`/zones/${id}`, params);
  return res.data;
}

export async function deleteZone(id: string): Promise<void> {
  await apiClient.delete(`/zones/${id}`);
}

export async function getZoneSyncStates(zoneId: string): Promise<ZoneSyncState[]> {
  const res = await apiClient.get<{ sync_states: ZoneSyncState[] }>(
    `/zones/${zoneId}/sync-states`
  );
  return res.data.sync_states ?? [];
}
