import { apiClient } from "./client";
import type { Changeset } from "../types";

export async function getChangesets(): Promise<Changeset[]> {
  const res = await apiClient.get<{ changesets: Changeset[] }>("/changesets");
  return res.data.changesets ?? [];
}

export async function getChangeset(id: string): Promise<Changeset> {
  const res = await apiClient.get<Changeset>(`/changesets/${id}`);
  return res.data;
}

export async function validateChangeset(id: string): Promise<Changeset> {
  const res = await apiClient.post<Changeset>(`/changesets/${id}/validate`);
  return res.data;
}

export async function applyChangeset(id: string): Promise<Changeset> {
  const res = await apiClient.post<Changeset>(`/changesets/${id}/apply`);
  return res.data;
}

export async function rollbackChangeset(id: string): Promise<Changeset> {
  const res = await apiClient.post<Changeset>(`/changesets/${id}/rollback`);
  return res.data;
}
