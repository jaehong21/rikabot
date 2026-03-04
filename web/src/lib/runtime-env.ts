export function readRuntimeEnv(...keys: string[]): string | undefined {
  const importMetaEnv = (import.meta as { env?: Record<string, unknown> }).env;
  for (const key of keys) {
    const fromImportMeta = importMetaEnv?.[key];
    if (typeof fromImportMeta === "string" && fromImportMeta.trim()) {
      return fromImportMeta.trim();
    }
  }

  if (typeof process !== "undefined") {
    for (const key of keys) {
      const fromProcess = process.env?.[key];
      if (typeof fromProcess === "string" && fromProcess.trim()) {
        return fromProcess.trim();
      }
    }
  }

  return undefined;
}

export function isDevelopmentMode(): boolean {
  const importMetaEnv = (import.meta as { env?: Record<string, unknown> }).env;
  if (importMetaEnv?.DEV === true) {
    return true;
  }

  const mode = readRuntimeEnv("MODE", "NODE_ENV", "PUBLIC_NODE_ENV");
  return mode === "development";
}

export function resolveBackendHostPort(): string | undefined {
  const definedHostPort = process.env.RIKA_DEV_BACKEND_HOSTPORT?.trim();
  if (definedHostPort) {
    return definedHostPort;
  }
  return readRuntimeEnv("RIKA_DEV_BACKEND_HOSTPORT");
}
