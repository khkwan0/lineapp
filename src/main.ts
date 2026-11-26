import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import QRCode from "qrcode";

type LoginEvent = {
  type: string;
  url?: string;
  pin?: string;
  message?: string;
  path?: string;
};

type ChatRow = {
  id: string;
  name: string;
  lastMessage: string;
  sealed: boolean;
  time: number;
  members: number | null;
  picture: string;
  muted: boolean;
  kind: string;
  unread: number | null;
  sticker?: string;
  stickerFallback?: string;
};

type HistoryMessage = {
  mine: boolean;
  name: string;
  picture?: string;
  text: string;
  sealed: boolean;
  time: number;
  sticker?: string;
  stickerFallback?: string;
  kind?: string;
  gid?: string;
  seq?: number;
  mediaId?: string;
  animated?: boolean;
};

type MediaSlot = { id: string; seq: number; animated?: boolean };

type BubbleMessage = HistoryMessage & { media?: MediaSlot[] };

type MediaFile = { id: string; mime?: string; data?: string; error?: string };

type SendResult = {
  id: string;
  ok: boolean;
  text?: string;
  time?: number;
  code?: number | null;
  error?: string;
};

type HistoryPayload = {
  id: string;
  messages: HistoryMessage[];
};

type ChatPayload = {
  type: string;
  profile: { name: string; picture: string };
  chats: ChatRow[];
  failures: { name: string; error: string }[];
  unreadTotal: number | null;
};

const login = document.querySelector<HTMLElement>("#login")!;
const app = document.querySelector<HTMLElement>("#app")!;
const status = document.querySelector<HTMLParagraphElement>("#status")!;
const card = document.querySelector<HTMLElement>("#qr-card")!;
const canvas = document.querySelector<HTMLCanvasElement>("#qr")!;
const pin = document.querySelector<HTMLParagraphElement>("#pin")!;
const refresh = document.querySelector<HTMLButtonElement>("#refresh")!;
const chats = document.querySelector<HTMLUListElement>("#chats")!;
const failures = document.querySelector<HTMLParagraphElement>("#chat-failures")!;
const search = document.querySelector<HTMLInputElement>("#search")!;
const thread = document.querySelector<HTMLElement>("#thread")!;
const threadHead = document.querySelector<HTMLElement>("#thread-head")!;
const threadBody = document.querySelector<HTMLElement>("#thread-body")!;
const railPhoto = document.querySelector<HTMLImageElement>("#rail-photo")!;
const railInitial = document.querySelector<HTMLElement>("#rail-initial")!;
const railCount = document.querySelector<HTMLElement>("#rail-count")!;
const composerInput = document.querySelector<HTMLInputElement>("#composer-input")!;
const sendError = document.querySelector<HTMLParagraphElement>("#send-error")!;
const railMore = document.querySelector<HTMLButtonElement>("#rail-more")!;
const railMenu = document.querySelector<HTMLElement>("#rail-menu")!;
const sheet = document.querySelector<HTMLElement>("#sheet")!;
const sheetTitle = document.querySelector<HTMLElement>("#sheet-title")!;
const sheetBody = document.querySelector<HTMLElement>("#sheet-body")!;
const sheetSettings = document.querySelector<HTMLElement>("#sheet-settings")!;
const minimizeToTray = document.querySelector<HTMLInputElement>("#minimize-to-tray")!;
const lightbox = document.querySelector<HTMLElement>("#lightbox")!;
const lightboxImg = document.querySelector<HTMLImageElement>("#lightbox-img")!;
const lightboxVideo = document.querySelector<HTMLVideoElement>("#lightbox-video")!;
const lightboxPrev = document.querySelector<HTMLButtonElement>("#lightbox-prev")!;
const lightboxNext = document.querySelector<HTMLButtonElement>("#lightbox-next")!;

let shownUrl = "";
let myName = "";
let myPicture = "";
let rows: ChatRow[] = [];
let filter = "all";
let selected = 0;
let openId = "";
let historyToken = 0;
let signedIn = false;
const mediaUrls = new Map<string, string>();
const animatedMedia = new Set<string>();
const mediaMimes = new Map<string, string>();
const mediaJobs = new Map<string, Promise<void>>();
let gallery: { ids: string[]; index: number } | null = null;
let trayBadge = -1;

type PollUpdate = {
  id: string;
  lastMessage: string;
  time: number;
  sealed: boolean;
  sticker?: string;
  stickerFallback?: string;
  unread?: number | null;
  incoming?: boolean;
  read?: boolean;
  refresh?: boolean;
  keepPreview?: boolean;
  name?: string;
  picture?: string;
  kind?: string;
  members?: number | null;
};

type PollPayload = {
  unreadTotal: number | null;
  unreads: { id: string; unread: number }[];
  updates: PollUpdate[];
};

async function drawQr(url: string): Promise<void> {
  await QRCode.toCanvas(canvas, url, {
    width: 320,
    margin: 1,
    errorCorrectionLevel: "M",
    color: { dark: "#000000", light: "#ffffff" },
  });
  card.hidden = false;
  pin.hidden = true;
  document.body.dataset.qr = "ready";
}

async function onEvent(event: LoginEvent): Promise<void> {
  if (event.type === "qr" && event.url) {
    if (event.url === shownUrl) {
      return;
    }
    shownUrl = event.url;
    await drawQr(event.url);
    status.textContent = "Scan this QR with the LINE app on your phone.";
    return;
  }
  if (event.type === "pin" && event.pin) {
    pin.hidden = false;
    pin.textContent = event.pin;
    status.textContent = "Confirm this PIN in the LINE app on your phone.";
    return;
  }
  if (event.type === "status" && event.message) {
    status.textContent = event.message;
    return;
  }
  if (event.type === "saved") {
    status.textContent = "Signed in. Loading chats…";
    refresh.hidden = true;
    await showChats();
    return;
  }
  if (event.type === "error" && event.message) {
    status.textContent = event.message;
  }
}

function clock(ms: number): string {
  if (!ms) {
    return "";
  }
  const date = new Date(ms);
  const now = new Date();
  const day = date.toLocaleDateString("en-US", { timeZone: "Asia/Bangkok" });
  const today = now.toLocaleDateString("en-US", { timeZone: "Asia/Bangkok" });
  const yesterdayDate = new Date(now.getTime() - 24 * 60 * 60 * 1000);
  const yesterday = yesterdayDate.toLocaleDateString("en-US", { timeZone: "Asia/Bangkok" });
  if (day === today) {
    return date.toLocaleTimeString("en-US", {
      hour: "numeric",
      minute: "2-digit",
      hour12: true,
      timeZone: "Asia/Bangkok",
    });
  }
  if (day === yesterday) {
    return "Yesterday";
  }
  return date.toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    timeZone: "Asia/Bangkok",
  });
}

function visibleRows(): ChatRow[] {
  const query = search.value.trim().toLowerCase();
  return rows.filter((chat) => {
    if (filter === "group" && chat.kind !== "group" && chat.kind !== "room") {
      return false;
    }
    if (filter === "friend" && chat.kind !== "friend") {
      return false;
    }
    if (filter === "official" && chat.kind !== "other") {
      return false;
    }
    if (query && !chat.name.toLowerCase().includes(query)) {
      return false;
    }
    return true;
  });
}

const SOUND_MUTE_KEY = "lineapp.soundMuted";

function readSoundMutes(): Record<string, boolean> {
  try {
    const raw = localStorage.getItem(SOUND_MUTE_KEY);
    if (!raw) {
      return {};
    }
    const parsed = JSON.parse(raw) as unknown;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return {};
    }
    const saved: Record<string, boolean> = {};
    for (const [id, value] of Object.entries(parsed)) {
      if (typeof value === "boolean") {
        saved[id] = value;
      }
    }
    return saved;
  } catch {
    return {};
  }
}

function soundMuted(chat: { id: string; muted: boolean }): boolean {
  const saved = readSoundMutes();
  if (Object.prototype.hasOwnProperty.call(saved, chat.id)) {
    return saved[chat.id];
  }
  return chat.muted;
}

function setSoundMuted(id: string, muted: boolean): void {
  const saved = readSoundMutes();
  saved[id] = muted;
  localStorage.setItem(SOUND_MUTE_KEY, JSON.stringify(saved));
}

function notifySound(id: string, serverMuted: boolean): boolean {
  if (soundMuted({ id, muted: serverMuted })) {
    return false;
  }
  return !(id === openId && document.hasFocus());
}

function speakerIcon(muted: boolean): SVGSVGElement {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  const body = document.createElementNS("http://www.w3.org/2000/svg", "path");
  body.setAttribute("fill", "currentColor");
  body.setAttribute("d", "M3.5 9.25h3.2L11 5.4v13.2l-4.3-3.85H3.5a1 1 0 0 1-1-1v-3.5a1 1 0 0 1 1-1z");
  svg.append(body);
  if (muted) {
    const slash = document.createElementNS("http://www.w3.org/2000/svg", "path");
    slash.setAttribute("fill", "none");
    slash.setAttribute("stroke", "currentColor");
    slash.setAttribute("stroke-width", "1.8");
    slash.setAttribute("stroke-linecap", "round");
    slash.setAttribute("d", "M16 9.2l5.2 5.6M21.2 9.2 16 14.8");
    svg.append(slash);
  } else {
    for (const d of ["M15.2 8.6a4.2 4.2 0 0 1 0 6.8", "M17.6 6.2a7.6 7.6 0 0 1 0 11.6"]) {
      const wave = document.createElementNS("http://www.w3.org/2000/svg", "path");
      wave.setAttribute("fill", "none");
      wave.setAttribute("stroke", "currentColor");
      wave.setAttribute("stroke-width", "1.7");
      wave.setAttribute("stroke-linecap", "round");
      wave.setAttribute("d", d);
      svg.append(wave);
    }
  }
  return svg;
}

function paintThreadHead(chat: ChatRow): void {
  const muted = soundMuted(chat);
  threadHead.replaceChildren();
  const title = document.createElement("span");
  title.className = "thread-title";
  const count = chat.members === null ? "" : ` (${chat.members})`;
  title.textContent = `${chat.name}${count}`;
  const button = document.createElement("button");
  button.type = "button";
  button.className = muted ? "thread-mute is-muted" : "thread-mute";
  button.dataset.muted = muted ? "1" : "0";
  button.setAttribute("aria-pressed", muted ? "true" : "false");
  button.setAttribute("aria-label", muted ? "Unmute sound notifications" : "Mute sound notifications");
  button.append(speakerIcon(muted));
  button.addEventListener("click", () => {
    setSoundMuted(chat.id, !soundMuted(chat));
    renderList();
  });
  threadHead.append(title, button);
  document.body.dataset.sound = muted ? "off" : "on";
}

let audioCtx: AudioContext | undefined;

function playChime(): void {
  const Ctx = window.AudioContext;
  if (!Ctx) {
    return;
  }
  if (!audioCtx) {
    audioCtx = new Ctx();
  }
  if (audioCtx.state === "suspended") {
    void audioCtx.resume();
  }
  const ctx = audioCtx;
  const now = ctx.currentTime;
  [784, 1046].forEach((freq, index) => {
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    const start = now + index * 0.09;
    osc.type = "sine";
    osc.frequency.value = freq;
    gain.gain.setValueAtTime(0.0001, start);
    gain.gain.exponentialRampToValueAtTime(0.045, start + 0.015);
    gain.gain.exponentialRampToValueAtTime(0.0001, start + 0.18);
    osc.connect(gain);
    gain.connect(ctx.destination);
    osc.start(start);
    osc.stop(start + 0.2);
  });
}

document.addEventListener("pointerdown", () => {
  const Ctx = window.AudioContext;
  if (!Ctx) {
    return;
  }
  if (!audioCtx) {
    audioCtx = new Ctx();
  }
  if (audioCtx.state === "suspended") {
    void audioCtx.resume();
  }
});

function showThread(chat: ChatRow | undefined): void {
  if (!chat) {
    thread.hidden = true;
    openId = "";
    closeGallery();
    composerInput.disabled = true;
    sendError.hidden = true;
    threadHead.replaceChildren();
    delete document.body.dataset.sound;
    return;
  }
  thread.hidden = false;
  paintThreadHead(chat);
  composerInput.disabled = false;
  if (chat.id === openId) {
    return;
  }
  openId = chat.id;
  closeGallery();
  sendError.hidden = true;
  const token = ++historyToken;
  threadBody.replaceChildren();
  const note = document.createElement("p");
  note.className = "thread-note";
  note.textContent = "Loading messages…";
  threadBody.append(note);
  void invoke("load_history", { id: chat.id }).catch((error: unknown) => {
    if (token !== historyToken) {
      return;
    }
    note.textContent = error instanceof Error ? error.message : String(error);
  });
}

function renderHistory(payload: HistoryPayload): void {
  if (payload.id !== openId) {
    return;
  }
  threadBody.replaceChildren();
  if (payload.messages.length === 0) {
    const note = document.createElement("p");
    note.className = "thread-note";
    note.textContent = "No messages";
    threadBody.append(note);
    document.body.dataset.history = "0";
    return;
  }
  groupMessages(payload.messages).forEach((message) => threadBody.append(messageBubble(message)));
  threadBody.scrollTop = threadBody.scrollHeight;
  document.body.dataset.history = String(payload.messages.length);
}

function stickerImage(
  src: string,
  fallback: string,
  className: string,
  onFail?: () => void,
): HTMLImageElement {
  const img = document.createElement("img");
  img.className = className;
  img.alt = "";
  img.src = src;
  img.addEventListener("error", () => {
    if (fallback && img.src !== fallback) {
      img.src = fallback;
      return;
    }
    img.remove();
    onFail?.();
  });
  return img;
}

function groupMessages(messages: HistoryMessage[]): BubbleMessage[] {
  const out: BubbleMessage[] = [];
  const at = new Map<string, number>();
  for (const message of messages) {
    const visual = (message.kind === "image" || message.kind === "video") && message.mediaId;
    const gid = message.gid || "";
    if (visual && gid) {
      const key = `${message.kind}:${gid}`;
      const index = at.get(key);
      if (index !== undefined) {
        out[index].media?.push({ id: message.mediaId || "", seq: message.seq || 0, animated: Boolean(message.animated) });
        continue;
      }
      at.set(key, out.length);
    }
    out.push({
      ...message,
      media: visual ? [{ id: message.mediaId || "", seq: message.seq || 0, animated: Boolean(message.animated) }] : undefined,
    });
  }
  for (const message of out) {
    message.media?.sort((left, right) => left.seq - right.seq);
  }
  return out;
}

function mediaLabel(text: string): boolean {
  return /^(letter-sealed )?(image|video)$/i.test(text.trim());
}

function mediaKey(id: string, preview: boolean): string {
  return `${preview ? "p" : "f"}:${id}`;
}

function decodeBase64(data: string): ArrayBuffer {
  const raw = atob(data);
  const bytes = new Uint8Array(raw.length);
  for (let index = 0; index < raw.length; index += 1) {
    bytes[index] = raw.charCodeAt(index);
  }
  return bytes.buffer;
}

function warmMedia(ids: string[], preview: boolean): Promise<void> {
  const missing = ids.filter((id) => id && !mediaUrls.has(mediaKey(id, preview)) && !mediaJobs.has(mediaKey(id, preview)));
  if (missing.length === 0) {
    const pending = ids.map((id) => mediaJobs.get(mediaKey(id, preview))).filter((job): job is Promise<void> => Boolean(job));
    return Promise.all(pending).then(() => undefined);
  }
  const chat = openId;
  const animatedIds = missing.filter((id) => animatedMedia.has(id));
  const job = invoke<MediaFile[]>("load_media", { id: chat, messageIds: missing, preview, animatedIds })
    .then((files) => {
      for (const file of files) {
        if (!file.data) {
          continue;
        }
        const mime = file.mime || "application/octet-stream";
        const url = URL.createObjectURL(new Blob([decodeBase64(file.data)], { type: mime }));
        const key = mediaKey(file.id, preview);
        mediaUrls.set(key, url);
        mediaMimes.set(key, mime);
      }
    })
    .catch(() => undefined)
    .finally(() => {
      for (const id of missing) {
        mediaJobs.delete(mediaKey(id, preview));
      }
    });
  for (const id of missing) {
    mediaJobs.set(mediaKey(id, preview), job);
  }
  return job;
}

const COLLAPSED_LINES = 15;

function appendMessageText(bubble: HTMLElement, text: string): void {
  const body = document.createElement("div");
  body.className = "msg-text";
  body.textContent = text;
  bubble.append(body);
  queueMicrotask(() => clampIfTall(body));
}

function clampIfTall(body: HTMLElement, attempt = 0): void {
  if (!body.isConnected || body.classList.contains("clamped") || body.dataset.fit === "1") {
    return;
  }
  const lineHeight = parseFloat(getComputedStyle(body).lineHeight);
  if (!Number.isFinite(lineHeight) || lineHeight <= 0 || body.clientWidth === 0) {
    if (attempt < 5) {
      requestAnimationFrame(() => clampIfTall(body, attempt + 1));
    }
    return;
  }
  if (body.scrollHeight <= lineHeight * (COLLAPSED_LINES + 0.5)) {
    body.dataset.fit = "1";
    return;
  }
  body.classList.add("clamped");
  const more = document.createElement("button");
  more.type = "button";
  more.className = "msg-more";
  more.textContent = "more";
  more.addEventListener("click", () => {
    body.classList.remove("clamped");
    more.remove();
  });
  body.after(more);
}

function messageBubble(message: BubbleMessage): HTMLElement {
  if (message.kind === "chat update") {
    const note = document.createElement("div");
    note.className = "chat-update";
    note.textContent = message.text;
    const label = clock(message.time);
    if (label) {
      const time = document.createElement("div");
      time.className = "when";
      time.textContent = label;
      note.append(time);
    }
    return note;
  }
  const bubble = document.createElement("div");
  const rich = Boolean(message.media?.length);
  bubble.className = message.mine ? "bubble mine" : "bubble";
  if (rich) {
    bubble.classList.add("rich");
  }
  if (message.name && !message.mine) {
    const who = document.createElement("div");
    who.className = "who";
    who.textContent = message.name;
    bubble.append(who);
  }
  const token = historyToken;
  if (message.media?.length && message.kind === "image") {
    bubble.append(imageGrid(message.media, token));
    if (message.text && !mediaLabel(message.text)) {
      appendMessageText(bubble, message.text);
    }
  } else if (message.media?.length && message.kind === "video") {
    bubble.append(videoStack(message.media, token));
    if (message.text && !mediaLabel(message.text)) {
      appendMessageText(bubble, message.text);
    }
  } else if (message.sticker) {
    bubble.append(stickerImage(message.sticker, message.stickerFallback || "", "sticker", () => {
      appendMessageText(bubble, message.text || "Sticker");
    }));
  } else {
    appendMessageText(bubble, message.text);
  }
  if (message.time) {
    const when = document.createElement("div");
    when.className = "when";
    when.textContent = clock(message.time);
    bubble.append(when);
  }
  const row = document.createElement("div");
  row.className = message.mine ? "msg mine" : "msg";
  const mark = senderMark(message);
  if (message.mine) {
    row.append(bubble, mark);
  } else {
    row.append(mark, bubble);
  }
  return row;
}

function senderPicture(message: BubbleMessage): string {
  if (message.mine) {
    return myPicture;
  }
  if (message.picture) {
    return message.picture;
  }
  const chat = rows.find((row) => row.id === openId);
  if (!chat || chat.kind === "group" || chat.kind === "room") {
    return "";
  }
  return chat.picture;
}

function senderName(message: BubbleMessage): string {
  if (message.mine) {
    return myName;
  }
  if (message.name) {
    return message.name;
  }
  const chat = rows.find((row) => row.id === openId);
  if (!chat || chat.kind === "group" || chat.kind === "room") {
    return "";
  }
  return chat.name;
}

function senderMark(message: BubbleMessage): HTMLElement {
  const name = senderName(message);
  const picture = senderPicture(message);
  if (!picture) {
    const circle = fallback(name);
    circle.classList.add("msg-avatar");
    return circle;
  }
  const photo = document.createElement("img");
  photo.className = "msg-avatar";
  photo.alt = "";
  photo.src = picture;
  photo.addEventListener("error", () => {
    const circle = fallback(name);
    circle.classList.add("msg-avatar");
    photo.replaceWith(circle);
  });
  return photo;
}

function showFrame(host: HTMLElement, url: string, mime: string): void {
  if (mime.startsWith("video/")) {
    const video = document.createElement("video");
    video.className = "gif-preview";
    video.src = url;
    video.autoplay = true;
    video.loop = true;
    video.muted = true;
    video.playsInline = true;
    host.replaceChildren(video);
    void video.play().catch(() => undefined);
    return;
  }
  const img = document.createElement("img");
  img.alt = "";
  img.src = url;
  host.replaceChildren(img);
}

function imageGrid(items: MediaSlot[], token: number): HTMLElement {
  const grid = document.createElement("div");
  grid.className = items.length > 1 ? "media-grid many" : "media-grid one";
  const ids = items.map((item) => item.id);
  for (const item of items) {
    if (item.animated && item.id) {
      animatedMedia.add(item.id);
    }
  }
  void warmMedia(items.filter((item) => !item.animated).map((item) => item.id), true);
  void warmMedia(items.filter((item) => item.animated).map((item) => item.id), false);
  items.forEach((item, index) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "media-thumb";
    const img = document.createElement("img");
    img.alt = "";
    button.append(img);
    button.addEventListener("click", () => openGallery(ids, index));
    grid.append(button);
    const thumb = !item.animated;
    void warmMedia([item.id], thumb).then(() => {
      if (token !== historyToken) {
        return;
      }
      const key = mediaKey(item.id, thumb);
      const url = mediaUrls.get(key);
      if (!url) {
        button.textContent = "Image";
        return;
      }
      showFrame(button, url, mediaMimes.get(key) || "");
    });
  });
  return grid;
}

function videoStack(items: MediaSlot[], token: number): HTMLElement {
  const stack = document.createElement("div");
  stack.className = "video-stack";
  const ids = items.map((item) => item.id);
  void warmMedia(ids, false);
  for (const item of items) {
    const video = document.createElement("video");
    video.className = "chat-video";
    video.controls = true;
    video.preload = "metadata";
    video.setAttribute("playsinline", "");
    stack.append(video);
    void warmMedia([item.id], false).then(() => {
      if (token !== historyToken) {
        return;
      }
      const url = mediaUrls.get(mediaKey(item.id, false));
      if (url) {
        video.src = url;
      } else {
        const note = document.createElement("div");
        note.textContent = "Video";
        video.replaceWith(note);
      }
    });
  }
  return stack;
}

function openGallery(ids: string[], index: number): void {
  gallery = { ids, index };
  lightbox.hidden = false;
  paintGallery();
}

function closeGallery(): void {
  gallery = null;
  lightbox.hidden = true;
  lightboxImg.removeAttribute("src");
  lightboxVideo.pause();
  lightboxVideo.removeAttribute("src");
  delete lightboxVideo.dataset.file;
  lightboxVideo.hidden = true;
  lightboxImg.hidden = false;
}

function showLightbox(url: string, mime: string): void {
  if (mime.startsWith("video/")) {
    lightboxImg.hidden = true;
    lightboxVideo.hidden = false;
    if (lightboxVideo.dataset.file !== url) {
      lightboxVideo.dataset.file = url;
      lightboxVideo.src = url;
    }
    void lightboxVideo.play().catch(() => undefined);
    return;
  }
  lightboxVideo.pause();
  lightboxVideo.hidden = true;
  lightboxVideo.removeAttribute("src");
  delete lightboxVideo.dataset.file;
  lightboxImg.hidden = false;
  lightboxImg.src = url;
}

function stepGallery(delta: number): void {
  if (!gallery || gallery.ids.length < 2) {
    return;
  }
  const count = gallery.ids.length;
  gallery.index = (gallery.index + delta + count) % count;
  paintGallery();
}

function paintGallery(): void {
  if (!gallery) {
    return;
  }
  const id = gallery.ids[gallery.index];
  const several = gallery.ids.length > 1;
  lightboxPrev.hidden = !several;
  lightboxNext.hidden = !several;
  const previewKey = mediaKey(id, true);
  const preview = mediaUrls.get(previewKey);
  if (preview) {
    showLightbox(preview, mediaMimes.get(previewKey) || "");
  }
  const token = historyToken;
  void warmMedia([id], false).then(() => {
    if (!gallery || gallery.ids[gallery.index] !== id || token !== historyToken) {
      return;
    }
    const fullKey = mediaKey(id, false);
    const full = mediaUrls.get(fullKey);
    if (full) {
      showLightbox(full, mediaMimes.get(fullKey) || "");
    }
  });
}

function renderList(): void {
  const shown = visibleRows();
  if (selected >= shown.length) {
    selected = 0;
  }
  chats.replaceChildren();
  shown.forEach((chat, index) => {
    const item = document.createElement("li");
    if (index === selected) {
      item.className = "selected";
    }
    item.addEventListener("click", () => {
      selected = index;
      renderList();
    });
    if (chat.picture) {
      const photo = document.createElement("img");
      photo.className = "avatar";
      photo.alt = "";
      photo.src = chat.picture;
      photo.addEventListener("error", () => {
        photo.replaceWith(fallback(chat.name));
      });
      item.append(photo);
    } else {
      item.append(fallback(chat.name));
    }
    const main = document.createElement("div");
    main.className = "row-main";
    const top = document.createElement("div");
    top.className = "row-top";
    const name = document.createElement("strong");
    name.textContent = chat.name;
    top.append(name);
    if (chat.members !== null) {
      const count = document.createElement("span");
      count.className = "count";
      count.textContent = `(${chat.members})`;
      top.append(count);
    }
    if (soundMuted(chat)) {
      const mute = document.createElement("span");
      mute.className = "mute";
      mute.textContent = "🔇";
      top.append(mute);
    }
    const time = document.createElement("span");
    time.className = "time";
    time.textContent = clock(chat.time);
    top.append(time);
    const bottom = document.createElement("div");
    bottom.className = "row-bottom";
    if (chat.sticker) {
      bottom.append(stickerImage(chat.sticker, chat.stickerFallback || "", "preview-sticker"));
    }
    const preview = document.createElement("span");
    preview.className = "preview";
    preview.textContent = previewText(chat.lastMessage);
    bottom.append(preview);
    if (chat.unread !== null && chat.unread > 0) {
      const badge = document.createElement("span");
      badge.className = "badge";
      badge.textContent = String(chat.unread);
      bottom.append(badge);
    }
    main.append(top, bottom);
    item.append(main);
    chats.append(item);
  });
  showThread(shown[selected]);
  document.body.dataset.chats = String(rows.length);
}

function updateLabel(text: string): boolean {
  return /^(letter-sealed )?chat update$/i.test(text.trim());
}

function previewText(text: string): string {
  const trimmed = text.trim();
  return trimmed && !updateLabel(trimmed) ? trimmed : "No messages";
}

function fallback(name: string): HTMLElement {
  const circle = document.createElement("span");
  circle.className = "avatar-fallback";
  circle.textContent = (name.trim()[0] || "?").toUpperCase();
  const hue = [...name].reduce((sum, char) => sum + char.charCodeAt(0), 0) % 360;
  circle.style.background = `hsl(${hue} 42% 42%)`;
  return circle;
}

function paintUnread(): void {
  const total = rows.reduce((sum, row) => sum + (row.unread && row.unread > 0 ? row.unread : 0), 0);
  if (total > 0) {
    railCount.hidden = false;
    railCount.textContent = String(total);
  } else {
    railCount.hidden = true;
    railCount.textContent = "";
  }
  if (total === trayBadge) {
    return;
  }
  trayBadge = total;
  void invoke("set_tray_badge", { count: total }).catch(() => {
    trayBadge = -1;
  });
}

function renderChats(payload: ChatPayload): void {
  const keepId = signedIn ? (openId || visibleRows()[selected]?.id || "") : "";
  signedIn = true;
  login.hidden = true;
  app.hidden = false;
  rows = payload.chats;
  if (keepId) {
    const index = visibleRows().findIndex((row) => row.id === keepId);
    selected = index >= 0 ? index : 0;
    if (index < 0) {
      openId = "";
    }
  } else {
    selected = 0;
    openId = "";
  }
  document.body.dataset.friends = String(rows.filter((chat) => chat.kind === "friend").length);
  const profile = payload.profile;
  myName = profile.name;
  myPicture = profile.picture;
  if (profile.picture) {
    railPhoto.hidden = false;
    railPhoto.src = profile.picture;
    railInitial.hidden = true;
    railPhoto.addEventListener("error", () => {
      railPhoto.hidden = true;
      railInitial.hidden = false;
    });
  }
  railInitial.textContent = (profile.name.trim()[0] || "L").toUpperCase();
  failures.hidden = payload.failures.length === 0;
  failures.textContent = payload.failures.map((failure) => `${failure.name}: ${failure.error}`).join(" ");
  paintUnread();
  renderList();
}

async function showChats(): Promise<boolean> {
  status.textContent = "Loading chats…";
  try {
    await invoke("load_chats");
    return true;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    if (message.includes("No LINE session")) {
      return false;
    }
    status.textContent = message;
    return true;
  }
}

async function start(): Promise<void> {
  refresh.disabled = true;
  const internals = (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  status.textContent = internals ? "Loading chats…" : "Desktop bridge is not ready.";
  if (!internals) {
    window.setTimeout(() => void start(), 200);
    return;
  }
  if (await showChats()) {
    return;
  }
  refresh.hidden = false;
  status.textContent = "Opening a login QR…";
  try {
    await invoke("start_login");
  } catch (error) {
    status.textContent = error instanceof Error ? error.message : String(error);
  } finally {
    refresh.disabled = false;
  }
}

document.querySelectorAll<HTMLButtonElement>("#tabs button").forEach((button) => {
  button.addEventListener("click", () => {
    filter = button.dataset.filter || "all";
    document.querySelectorAll("#tabs button").forEach((tab) => tab.classList.remove("on"));
    button.classList.add("on");
    selected = 0;
    renderList();
  });
});

search.addEventListener("input", () => {
  selected = 0;
  renderList();
});

document.querySelector<HTMLButtonElement>("#rail-chats")!.addEventListener("click", () => {
  document.querySelector<HTMLButtonElement>('#tabs button[data-filter="all"]')!.click();
});
document.querySelector<HTMLButtonElement>("#rail-friends")!.addEventListener("click", () => {
  document.querySelector<HTMLButtonElement>('#tabs button[data-filter="friend"]')!.click();
});

await listen<LoginEvent>("login-event", (event) => {
  void onEvent(event.payload);
});

(window as unknown as { __lineRender?: (payload: ChatPayload) => void }).__lineRender = (payload) => {
  try {
    renderChats(payload);
  } catch (error) {
    status.textContent = error instanceof Error ? error.message : String(error);
  }
};

(window as unknown as { __lineHistory?: (payload: HistoryPayload) => void }).__lineHistory = (payload) => {
  try {
    renderHistory(payload);
  } catch (error) {
    status.textContent = error instanceof Error ? error.message : String(error);
  }
};

function onSent(payload: SendResult): void {
  composerInput.disabled = !openId;
  if (!payload.ok) {
    if (payload.id === openId) {
      sendError.hidden = false;
      sendError.textContent = payload.code === 82
        ? "Letter sealing blocked this send (E2EE_RETRY_ENCRYPT)."
        : (payload.error || "Could not send");
    }
    return;
  }
  const chat = rows.find((row) => row.id === payload.id);
  if (chat && payload.text) {
    chat.lastMessage = payload.text;
    chat.time = payload.time || Date.now();
    chat.sticker = "";
    chat.sealed = false;
    rows.sort((left, right) => right.time - left.time);
    const shown = visibleRows();
    const index = shown.findIndex((row) => row.id === payload.id);
    if (index >= 0) {
      selected = index;
    }
  }
  if (payload.id === openId && payload.text) {
    sendError.hidden = true;
    composerInput.value = "";
    const note = threadBody.querySelector(".thread-note");
    note?.remove();
    threadBody.append(messageBubble({
      mine: true,
      name: "",
      text: payload.text,
      sealed: false,
      time: payload.time || Date.now(),
    }));
    threadBody.scrollTop = threadBody.scrollHeight;
  }
  renderList();
}

(window as unknown as { __linePoll?: (payload: PollPayload) => void }).__linePoll = (payload) => {
  try {
    applyPoll(payload);
  } catch (error) {
    status.textContent = error instanceof Error ? error.message : String(error);
  }
};

(window as unknown as { __lineSent?: (payload: SendResult) => void }).__lineSent = (payload) => {
  try {
    onSent(payload);
  } catch (error) {
    status.textContent = error instanceof Error ? error.message : String(error);
  }
};

document.querySelector<HTMLFormElement>("#composer")!.addEventListener("submit", (event) => {
  event.preventDefault();
  const text = composerInput.value.trim();
  if (!text || !openId || composerInput.disabled) {
    return;
  }
  const id = openId;
  composerInput.disabled = true;
  sendError.hidden = true;
  void invoke("send_message", { id, text }).catch((error: unknown) => {
    composerInput.disabled = !openId;
    if (id !== openId) {
      return;
    }
    sendError.hidden = false;
    sendError.textContent = error instanceof Error ? error.message : String(error);
  });
});

function closeMenu(): void {
  railMenu.hidden = true;
  railMore.setAttribute("aria-expanded", "false");
}

function showSheet(title: string, body: string): void {
  sheetTitle.textContent = title;
  sheetBody.textContent = body;
  sheetBody.hidden = false;
  sheetSettings.hidden = true;
  sheet.hidden = false;
}

async function showSettings(): Promise<void> {
  sheetTitle.textContent = "Settings";
  sheetBody.hidden = true;
  sheetSettings.hidden = false;
  sheet.hidden = false;
  minimizeToTray.disabled = true;
  try {
    const settings = await invoke<{ minimizeToTray: boolean }>("get_settings");
    minimizeToTray.checked = settings.minimizeToTray;
  } catch (error) {
    status.textContent = error instanceof Error ? error.message : String(error);
  }
  minimizeToTray.disabled = false;
}

railMore.addEventListener("click", (event) => {
  event.stopPropagation();
  railMenu.hidden = !railMenu.hidden;
  railMore.setAttribute("aria-expanded", railMenu.hidden ? "false" : "true");
});

railMenu.addEventListener("click", (event) => {
  event.stopPropagation();
  const action = (event.target as HTMLElement).closest("button")?.dataset.action;
  if (!action) {
    return;
  }
  closeMenu();
  if (action === "settings") {
    void showSettings();
  } else if (action === "help") {
    showSheet("Help", "Help is not in this window yet.");
  } else if (action === "about") {
    showSheet("About LINE", "This window uses the LINE desktop protocol. Letter-sealed chats stay sealed.");
  } else if (action === "logout") {
    void logout();
  } else if (action === "quit") {
    void invoke("quit");
  }
});

document.addEventListener("click", () => closeMenu());

document.querySelector<HTMLButtonElement>("#sheet-close")!.addEventListener("click", () => {
  sheet.hidden = true;
});

minimizeToTray.addEventListener("change", () => {
  const enabled = minimizeToTray.checked;
  void invoke("set_minimize_to_tray", { enabled }).catch((error: unknown) => {
    minimizeToTray.checked = !enabled;
    status.textContent = error instanceof Error ? error.message : String(error);
  });
});

lightboxPrev.addEventListener("click", () => stepGallery(-1));
lightboxNext.addEventListener("click", () => stepGallery(1));
document.querySelector<HTMLButtonElement>("#lightbox-close")!.addEventListener("click", () => closeGallery());
lightbox.addEventListener("click", (event) => {
  if (event.target === lightbox) {
    closeGallery();
  }
});
document.addEventListener("keydown", (event) => {
  if (lightbox.hidden) {
    return;
  }
  if (event.key === "Escape") {
    closeGallery();
  } else if (event.key === "ArrowLeft") {
    stepGallery(-1);
  } else if (event.key === "ArrowRight") {
    stepGallery(1);
  }
});

function applyPoll(payload: PollPayload): void {
  if (!signedIn) {
    return;
  }
  const selectedId = visibleRows()[selected]?.id || openId;
  const openTime = rows.find((row) => row.id === openId)?.time || 0;
  for (const item of payload.unreads || []) {
    const chat = rows.find((row) => row.id === item.id);
    if (chat) {
      chat.unread = item.unread;
    }
  }
  let openAdvanced = false;
  let shouldChime = false;
  for (const update of payload.updates || []) {
    const chat = rows.find((row) => row.id === update.id);
    if (!chat) {
      if (update.read) {
        continue;
      }
      rows.push({
        id: update.id,
        name: update.name || "Direct chat",
        lastMessage: update.keepPreview || updateLabel(update.lastMessage) ? "" : update.lastMessage,
        sealed: update.sealed,
        time: update.time,
        members: update.members ?? null,
        picture: update.picture || "",
        muted: false,
        kind: update.kind || "friend",
        unread: update.incoming ? 1 : (update.unread ?? null),
        sticker: update.sticker || "",
        stickerFallback: update.stickerFallback || "",
      });
      if (update.incoming && notifySound(update.id, false)) {
        shouldChime = true;
      }
      continue;
    }
    if (update.read) {
      chat.unread = 0;
    } else if (update.incoming) {
      chat.unread = (chat.unread || 0) + 1;
      if (notifySound(chat.id, chat.muted)) {
        shouldChime = true;
      }
    } else if (typeof update.unread === "number") {
      chat.unread = update.unread;
    }
    const fill = Boolean(update.refresh && update.lastMessage && !update.keepPreview);
    if (update.time > chat.time || fill) {
      const keepTyped = !fill
        && update.sealed
        && Boolean(chat.lastMessage)
        && !chat.lastMessage.startsWith("Letter-sealed")
        && update.time - chat.time < 20000;
      if (update.keepPreview && updateLabel(chat.lastMessage)) {
        chat.lastMessage = "";
        chat.sticker = "";
        chat.stickerFallback = "";
      } else if (!keepTyped && !update.keepPreview) {
        chat.lastMessage = updateLabel(update.lastMessage) ? "" : update.lastMessage;
        chat.sealed = update.sealed;
        chat.sticker = update.sticker || "";
        chat.stickerFallback = update.stickerFallback || "";
        if (chat.id === openId && update.time > openTime) {
          openAdvanced = true;
        }
      } else if (update.keepPreview && chat.id === openId && update.time > openTime) {
        openAdvanced = true;
      }
      if (update.time > chat.time) {
        chat.time = update.time;
      }
    }
  }
  rows.sort((left, right) => right.time - left.time);
  const index = visibleRows().findIndex((row) => row.id === selectedId);
  selected = index >= 0 ? index : 0;
  paintUnread();
  renderList();
  if (shouldChime) {
    playChime();
  }
  if (openAdvanced && openId) {
    void invoke("load_history", { id: openId });
  }
}

async function logout(): Promise<void> {
  signedIn = false;
  trayBadge = -1;
  closeMenu();
  sheet.hidden = true;
  try {
    await invoke("logout");
  } catch (error) {
    signedIn = true;
    status.textContent = error instanceof Error ? error.message : String(error);
    return;
  }
  rows = [];
  openId = "";
  shownUrl = "";
  app.hidden = true;
  login.hidden = false;
  card.hidden = true;
  pin.hidden = true;
  refresh.hidden = false;
  status.textContent = "Opening a login QR…";
  try {
    await invoke("start_login");
  } catch (error) {
    status.textContent = error instanceof Error ? error.message : String(error);
  }
}

refresh.addEventListener("click", () => {
  void start();
});

void start();
