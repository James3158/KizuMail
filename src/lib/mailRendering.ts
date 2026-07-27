import type { MessageAttachment, MessageSummary } from "../types";
import { safeAssetSrc } from "./assets";

export interface ReadableEmailContent {
  summary: string;
  paragraphs: string[];
  links: string[];
}

export interface RemoteEmailMedia {
  url: string;
  kind: "image" | "video" | "pdf";
  label: string;
}

const CSS_NOISE_HINTS = ["font-family", "font-size", "line-height", "mso-", "padding:", "margin:", "{", "}"];

export function buildReadableEmailContent(message: MessageSummary): ReadableEmailContent {
  const rawText = message.bodyText?.trim();
  const fallbackText = message.bodyHtml ? htmlToReadableText(message.bodyHtml) : "";
  const text = cleanEmailText(rawText || fallbackText || message.preview);
  const links = extractLinks([message.bodyHtml ?? "", rawText ?? "", text].join(" "));
  const allParagraphs = splitReadableParagraphs(text);
  const summary = allParagraphs[0] || message.preview || "Diese E-Mail wurde lokal geladen.";
  return {
    summary,
    paragraphs: allParagraphs.length > 1 ? allParagraphs.slice(1) : allParagraphs,
    links
  };
}

export function buildOriginalEmailDocument(message: MessageSummary, allowRemoteImages: boolean) {
  const html = message.bodyHtml?.trim();
  const source = html || `<pre>${escapeHtml(message.bodyText || message.preview || "")}</pre>`;
  const parser = new DOMParser();
  const document = parser.parseFromString(source, "text/html");

  document.querySelectorAll("script, iframe, object, embed, form, input, button, textarea, select, base, meta[http-equiv]").forEach((node) => node.remove());
  document.querySelectorAll("*").forEach((node) => {
    [...node.attributes].forEach((attribute) => {
      const name = attribute.name.toLowerCase();
      if (name.startsWith("on")) node.removeAttribute(attribute.name);
      if (name === "target") node.setAttribute("target", "_blank");
    });
  });

  const cidMap = buildCidMap(message.attachments);
  document.querySelectorAll("img").forEach((image) => {
    const rawSrc = image.getAttribute("src") ?? "";
    const normalized = normalizeContentReference(rawSrc);
    const localAttachment = normalized ? cidMap.get(normalized) : undefined;
    if (localAttachment?.storageRef) {
      image.setAttribute("src", safeAssetSrc(localAttachment.storageRef));
      image.removeAttribute("srcset");
      image.setAttribute("data-kizumail-inline", "true");
      return;
    }
    if (!allowRemoteImages && isRemoteUrl(rawSrc)) {
      image.removeAttribute("src");
      image.setAttribute("data-kizumail-blocked-src", rawSrc);
      image.setAttribute("alt", image.getAttribute("alt") || "Externes Bild blockiert");
    }
    if (!allowRemoteImages && hasRemoteUrl(image.getAttribute("srcset") ?? "")) {
      image.removeAttribute("srcset");
    }
  });

  document.querySelectorAll("video, audio, source").forEach((media) => {
    for (const attribute of ["src", "poster", "srcset"]) {
      const value = media.getAttribute(attribute) ?? "";
      const localAttachment = cidMap.get(normalizeContentReference(value));
      if (localAttachment?.storageRef) {
        media.setAttribute(attribute === "srcset" ? "src" : attribute, safeAssetSrc(localAttachment.storageRef));
        if (attribute === "srcset") media.removeAttribute("srcset");
      } else if (!allowRemoteImages && hasRemoteUrl(value)) {
        media.removeAttribute(attribute);
      }
    }
    if (media instanceof HTMLMediaElement) media.setAttribute("controls", "");
  });

  if (!allowRemoteImages) {
    document.querySelectorAll<HTMLElement>("[style]").forEach((node) => {
      if (hasRemoteUrl(node.getAttribute("style") ?? "")) {
        node.style.removeProperty("background");
        node.style.removeProperty("background-image");
        node.style.removeProperty("list-style-image");
      }
    });
    document.querySelectorAll("style").forEach((style) => {
      style.textContent = (style.textContent ?? "").replace(/url\(\s*(['"]?)https?:\/\/[^)]*\1\s*\)/gi, "none");
    });
  }

  document.querySelectorAll("a").forEach((link) => {
    link.setAttribute("target", "_blank");
    link.setAttribute("rel", "noreferrer");
  });

  const body = document.body.innerHTML || `<pre>${escapeHtml(message.bodyText || message.preview || "")}</pre>`;
  const remoteSources = allowRemoteImages ? " https:" : "";
  return `<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data: blob: asset: http://asset.localhost https://asset.localhost${remoteSources}; media-src data: blob: asset: http://asset.localhost https://asset.localhost${remoteSources}; style-src 'unsafe-inline'; font-src data:${remoteSources};" />
    <style>
      :root { color-scheme: light; }
      html, body { margin: 0; padding: 0; background: transparent; }
      body {
        font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
        color: #111827;
        line-height: 1.55;
        overflow-wrap: anywhere;
      }
      img { max-width: 100%; height: auto; border: 0; }
      table { max-width: 100%; border-collapse: collapse; }
      a { color: #5b35d5; }
      pre { white-space: pre-wrap; font: inherit; }
      [data-kizumail-blocked-src] {
        display: inline-grid;
        place-items: center;
        min-width: 180px;
        min-height: 92px;
        border: 1px dashed #b5a7ff;
        border-radius: 12px;
        background: #f4f1ff;
        color: #5b35d5;
      }
      [data-kizumail-blocked-src]::before { content: "Externes Bild blockiert"; font-weight: 700; }
    </style>
  </head>
  <body>${body}</body>
</html>`;
}

export function extractRemoteEmailMedia(message: MessageSummary): RemoteEmailMedia[] {
  const html = message.bodyHtml?.trim();
  if (!html) return [];
  const document = new DOMParser().parseFromString(html, "text/html");
  const media: RemoteEmailMedia[] = [];
  const seen = new Set<string>();

  const add = (url: string, kind: RemoteEmailMedia["kind"], label: string) => {
    const normalized = url.trim();
    if (!isRemoteUrl(normalized) || seen.has(normalized)) return;
    seen.add(normalized);
    media.push({ url: normalized, kind, label: label.trim() || remoteMediaFallbackLabel(kind) });
  };

  document.querySelectorAll("img").forEach((image) => {
    if (isLikelyTrackingImage(image)) return;
    const src = image.getAttribute("src") || firstSrcsetUrl(image.getAttribute("srcset") ?? "");
    add(src, "image", image.getAttribute("alt") || image.getAttribute("title") || "Bild aus der E-Mail");
  });
  document.querySelectorAll("video").forEach((video) => {
    add(video.getAttribute("src") ?? "", "video", video.getAttribute("title") || "Video aus der E-Mail");
    add(video.getAttribute("poster") ?? "", "image", video.getAttribute("title") || "Video-Vorschau");
  });
  document.querySelectorAll("video source").forEach((source) => {
    add(source.getAttribute("src") ?? "", "video", source.getAttribute("title") || "Video aus der E-Mail");
  });
  document.querySelectorAll("a[href]").forEach((link) => {
    const href = link.getAttribute("href") ?? "";
    if (isPdfUrl(href)) add(href, "pdf", link.textContent || "PDF aus der E-Mail");
  });

  return media.slice(0, 16);
}

export function isBodyLikeAttachment(attachment: MessageAttachment) {
  const filename = attachment.filename.trim().toLowerCase();
  const mimeType = attachment.mimeType.trim().toLowerCase();
  if (attachment.disposition === "body") return true;
  if (mimeType.startsWith("multipart/")) return true;
  if (!attachment.storageRef && (mimeType === "text/html" || mimeType === "text/plain")) return true;
  if (filename === "anhang" || filename === "attachment") {
    return mimeType === "text/html" || mimeType === "text/plain" || mimeType.startsWith("multipart/");
  }
  return mimeType === "text/html" && (filename.endsWith(".html") || filename.endsWith(".htm"));
}

export function attachmentPreviewKind(attachment: MessageAttachment): "image" | "video" | "pdf" | "file" {
  if (attachment.previewKind && attachment.previewKind !== "file") return attachment.previewKind;
  const mimeType = attachment.mimeType.toLowerCase();
  const filename = attachment.filename.toLowerCase();
  if (mimeType.startsWith("image/")) return "image";
  if (mimeType.startsWith("video/")) return "video";
  if (mimeType === "application/pdf" || filename.endsWith(".pdf")) return "pdf";
  if (/\.(png|jpe?g|gif|webp|avif|svg|bmp)$/i.test(filename)) return "image";
  if (/\.(mp4|webm|mov|m4v|ogv)$/i.test(filename)) return "video";
  return "file";
}

export function extractLinks(value: string) {
  const matches = value.match(/https?:\/\/[^\s"'<>]+/gi) ?? [];
  return Array.from(new Set(matches.map((link) => link.replace(/[),.;]+$/g, ""))))
    .filter((link) => !isDecorativeMailAssetLink(link))
    .slice(0, 8);
}

export function shortenLink(link: string) {
  try {
    const url = new URL(link);
    const path = url.pathname === "/" ? "" : url.pathname;
    return `${url.hostname}${path}`.slice(0, 70);
  } catch {
    return link.slice(0, 70);
  }
}

function htmlToReadableText(html: string) {
  if (!html.trim()) return "";
  const document = new DOMParser().parseFromString(html, "text/html");
  document.querySelectorAll("style, script, noscript, meta, link, title").forEach((node) => node.remove());
  document.querySelectorAll("br").forEach((node) => node.replaceWith("\n"));
  document.querySelectorAll("p, div, li, h1, h2, h3, tr").forEach((node) => node.append("\n"));
  return document.body.textContent ?? "";
}

function cleanEmailText(value: string) {
  return decodeHtmlEntities(value)
    .replace(/\/\*[\s\S]*?\*\//g, " ")
    .replace(/@media[\s\S]*?\}\s*\}/gi, " ")
    .replace(/[#.]?[a-z0-9_-]+\s*\{[^}]*\}/gi, " ")
    .replace(/\b(?:font|font-family|font-size|line-height|color|background|margin|padding|display|width|height|border|text-align|mso-[a-z-]+):[^;]+;/gi, " ")
    .replace(/\[[^\]]*ExternalClass[^\]]*\]/gi, " ")
    .replace(/\s*\|?\s*View in browser\s*/gi, "\n")
    .replace(/\s*\|?\s*Im Browser ansehen\s*/gi, "\n")
    .replace(/[ \t]{2,}/g, " ")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

function decodeHtmlEntities(value: string) {
  const textArea = document.createElement("textarea");
  textArea.innerHTML = value;
  return textArea.value;
}

function splitReadableParagraphs(value: string) {
  const cleaned = value
    .split(/\n+/)
    .map((line) => line.trim())
    .filter(Boolean)
    .filter((line) => !looksLikeCssNoise(line));
  const paragraphs: string[] = [];
  for (const line of cleaned) {
    if (line.length <= 260) {
      paragraphs.push(line);
      continue;
    }
    const chunks = line.match(/.{1,240}(?:\s|$)/g) ?? [line];
    paragraphs.push(...chunks.map((chunk) => chunk.trim()).filter(Boolean));
  }
  return paragraphs.slice(0, 60);
}

function looksLikeCssNoise(line: string) {
  const matches = CSS_NOISE_HINTS.filter((hint) => line.toLowerCase().includes(hint)).length;
  return matches >= 2 || (/^[.#]?[a-z0-9_-]+\s*\{/i.test(line) && line.includes(";"));
}

function isDecorativeMailAssetLink(link: string) {
  try {
    const url = new URL(link);
    const host = url.hostname.toLowerCase();
    const path = url.pathname.toLowerCase();
    if (host.includes("fonts.googleapis.com") || host.includes("fonts.gstatic.com")) return true;
    if (/\.(png|jpg|jpeg|gif|webp|svg|css|woff|woff2)$/i.test(path)) return true;
    if (path.includes("/assets/") || path.includes("/images/") || path.includes("/static/")) return true;
    return false;
  } catch {
    return false;
  }
}

function buildCidMap(attachments: MessageAttachment[]) {
  const map = new Map<string, MessageAttachment>();
  for (const attachment of attachments) {
    const id = normalizeContentReference(attachment.contentId ?? "");
    if (id) map.set(id, attachment);
  }
  return map;
}

function normalizeContentReference(value: string) {
  return value.trim().replace(/^cid:/i, "").replace(/^<|>$/g, "").toLowerCase();
}

function isRemoteUrl(value: string) {
  return /^https?:\/\//i.test(value);
}

function hasRemoteUrl(value: string) {
  return /https?:\/\//i.test(value);
}

function firstSrcsetUrl(value: string) {
  return value.split(",")[0]?.trim().split(/\s+/)[0] ?? "";
}

function isLikelyTrackingImage(image: Element) {
  const width = Number.parseInt(image.getAttribute("width") ?? "", 10);
  const height = Number.parseInt(image.getAttribute("height") ?? "", 10);
  if ((Number.isFinite(width) && width <= 2) || (Number.isFinite(height) && height <= 2)) return true;
  const src = image.getAttribute("src") ?? "";
  return /(?:pixel|tracking|open(?:\.gif|\/)|beacon)/i.test(src);
}

function isPdfUrl(value: string) {
  if (!isRemoteUrl(value)) return false;
  try {
    return new URL(value).pathname.toLowerCase().endsWith(".pdf");
  } catch {
    return false;
  }
}

function remoteMediaFallbackLabel(kind: RemoteEmailMedia["kind"]) {
  if (kind === "video") return "Video aus der E-Mail";
  if (kind === "pdf") return "PDF aus der E-Mail";
  return "Bild aus der E-Mail";
}

function escapeHtml(value: string) {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}
