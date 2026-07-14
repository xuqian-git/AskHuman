import type {
  PermissionDiffHunk,
  PermissionDiffLineKind,
  SnapshotStatus,
} from "../../lib/types";

export function diffLinePrefix(kind: PermissionDiffLineKind): string {
  if (kind === "add") return "+";
  if (kind === "delete") return "−";
  if (kind === "meta") return "·";
  return " ";
}

export function hunkLabel(hunk: PermissionDiffHunk): string {
  const oldStart = hunk.oldStart ?? "?";
  const newStart = hunk.newStart ?? "?";
  const range = `@@ -${oldStart} +${newStart} @@`;
  return hunk.header ? `${range} ${hunk.header}` : range;
}

export function snapshotStatusKey(status: SnapshotStatus): string {
  return `popup.permissionDiff.status.${status}`;
}

export function snapshotTime(ms?: number | null): string {
  if (!ms || !Number.isFinite(ms)) return "";
  return new Date(ms).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function absolutePathParts(path: string): string[] | null {
  if (!path.startsWith("/")) return null;
  const parts: string[] = [];
  for (const part of path.split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") {
      parts.pop();
    } else {
      parts.push(part);
    }
  }
  return parts;
}

export function permissionDisplayPath(path: string, workspace: string): string {
  const pathParts = absolutePathParts(path);
  const workspaceParts = absolutePathParts(workspace);
  if (!pathParts || !workspaceParts) return path;
  if (
    workspaceParts.length > pathParts.length ||
    workspaceParts.some((part, index) => pathParts[index] !== part)
  ) {
    return path;
  }
  return pathParts.slice(workspaceParts.length).join("/") || ".";
}
