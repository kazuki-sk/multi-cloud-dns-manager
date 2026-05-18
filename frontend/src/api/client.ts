import axios from "axios";

export const DEFAULT_API_BASE_URL = "/api/v1";

function getBaseURL(): string {
  return localStorage.getItem("api-base-url") ?? DEFAULT_API_BASE_URL;
}

export const apiClient = axios.create({
  baseURL: getBaseURL(),
  headers: { "Content-Type": "application/json" },
});
