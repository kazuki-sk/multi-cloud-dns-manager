import { apiClient } from "./client";
import type { Provider, ProviderType } from "../types";

export interface CreateProviderParams {
  name: string;
  provider_type: ProviderType;
  credentials: Record<string, string>;
}

export async function getProviders(): Promise<Provider[]> {
  const res = await apiClient.get<{ providers: Provider[] }>("/providers");
  return res.data.providers ?? [];
}

export async function createProvider(
  params: CreateProviderParams
): Promise<Provider> {
  const res = await apiClient.post<Provider>("/providers", params);
  return res.data;
}

export async function deleteProvider(id: string): Promise<void> {
  await apiClient.delete(`/providers/${id}`);
}
