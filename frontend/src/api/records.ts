import { apiClient } from "./client";
import type { DnsRecord, RecordType } from "../types";

export interface PushPendingResponse {
  changeset_id: string;
  record_count: number;
}

export interface CreateRecordParams {
  name: string;
  record_type: RecordType;
  ttl: number;
  values: string[];
}

export interface UpdateRecordParams {
  ttl?: number;
  values?: string[];
}

export async function getRecords(zoneId: string): Promise<DnsRecord[]> {
  const res = await apiClient.get<{ records: DnsRecord[] }>(
    `/zones/${zoneId}/records`
  );
  return res.data.records;
}

export async function createRecord(
  zoneId: string,
  params: CreateRecordParams
): Promise<DnsRecord> {
  const res = await apiClient.post<DnsRecord>(
    `/zones/${zoneId}/records`,
    params
  );
  return res.data;
}

export async function updateRecord(
  zoneId: string,
  recordId: string,
  params: UpdateRecordParams
): Promise<DnsRecord> {
  const res = await apiClient.patch<DnsRecord>(
    `/zones/${zoneId}/records/${recordId}`,
    params
  );
  return res.data;
}

export async function deleteRecord(
  zoneId: string,
  recordId: string
): Promise<void> {
  await apiClient.delete(`/zones/${zoneId}/records/${recordId}`);
}

export async function pushPendingRecords(
  zoneId: string
): Promise<PushPendingResponse> {
  const res = await apiClient.post<PushPendingResponse>(
    `/zones/${zoneId}/push`
  );
  return res.data;
}
