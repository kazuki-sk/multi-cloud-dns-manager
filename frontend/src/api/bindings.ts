import { apiClient } from "./client";
import type { ProviderBinding, ProviderZoneBinding } from "../types";

export interface CreateBindingParams {
  provider_id: string;
  provider_zone_id: string;
}

export async function getBindings(zoneId: string): Promise<ProviderBinding[]> {
  const res = await apiClient.get<{ bindings: ProviderBinding[] }>(
    `/zones/${zoneId}/bindings`
  );
  return res.data.bindings ?? [];
}

export async function getBindingsByProvider(
  providerId: string
): Promise<ProviderZoneBinding[]> {
  const res = await apiClient.get<{ bindings: ProviderZoneBinding[] }>(
    `/providers/${providerId}/bindings`
  );
  return res.data.bindings ?? [];
}

export async function createBinding(
  zoneId: string,
  params: CreateBindingParams
): Promise<ProviderBinding> {
  const res = await apiClient.post<ProviderBinding>(
    `/zones/${zoneId}/bindings`,
    params
  );
  return res.data;
}

export async function deleteBinding(
  zoneId: string,
  bindingId: string
): Promise<void> {
  await apiClient.delete(`/zones/${zoneId}/bindings/${bindingId}`);
}
