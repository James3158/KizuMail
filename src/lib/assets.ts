import { convertFileSrc } from "@tauri-apps/api/core";

export function safeAssetSrc(path?: string) {
  if (!path) return "";
  try {
    return convertFileSrc(path);
  } catch {
    return path;
  }
}
