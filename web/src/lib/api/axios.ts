import axios from "axios";
import { isDevelopmentMode, resolveBackendHostPort } from "@/lib/runtime-env";

function resolveBackendHttpBaseUrl(): string {
  const hostPort = resolveBackendHostPort();
  if (hostPort) {
    return `http://${hostPort}`;
  }

  if (isDevelopmentMode()) {
    return "http://127.0.0.1:4728";
  }

  return window.location.origin;
}

export const axiosInstance = axios.create({
  baseURL: resolveBackendHttpBaseUrl(),
  timeout: 30_000,
});
