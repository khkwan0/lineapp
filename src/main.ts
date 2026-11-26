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
  id?: string;
  mine: boolean;
  sender?: string;
  name: string;
  picture?: string;
  text: string;
  sealed: boolean;
  time: number;
  status?: string;
  readers?: number;
  sticker?: string;
  stickerFallback?: string;
  kind?: string;
  gid?: string;
  seq?: number;
  mediaId?: string;
  animated?: boolean;
  reaction?: string;
  reactionImage?: string;
  reactionMine?: boolean;
};

type MediaSlot = { id: string; seq: number; animated?: boolean };

type BubbleMessage = HistoryMessage & { media?: MediaSlot[] };

type MediaFile = { id: string; mime?: string; data?: string; error?: string };

type SendResult = {
  id: string;
  ok: boolean;
  text?: string;
  time?: number;
  messageId?: string;
  code?: number | null;
  error?: string;
};

type ReadMark = { end: string };

type HistoryPayload = {
  id: string;
  lastMessageId?: string;
  messages: HistoryMessage[];
};

type ChatPayload = {
  type: string;
  profile: { name: string; picture: string; mid?: string };
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
let myMid = "";
let rows: ChatRow[] = [];
let filter = "all";
let selected = 0;
let openId = "";
let openLastId = "";
let historyToken = 0;
let paintedHistory = "";
let paintedIds = "";
let openUnread = 0;
let unreadFocus = "";
let placingThread = false;
let userMoved = false;
let placeToken = 0;
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
  messageId?: string;
  incoming?: boolean;
  read?: boolean;
  refresh?: boolean;
  keepPreview?: boolean;
  readMarks?: ReadMark[];
  thread?: HistoryMessage;
  name?: string;
  picture?: string;
  kind?: string;
  members?: number | null;
  reaction?: { messageId: string; emoji?: string; image?: string; mine?: boolean };
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

window.addEventListener("focus", () => catchUpRead());
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") {
    catchUpRead();
  }
});

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

const readHeld = new Map<string, string>();
const readSent = new Map<string, string>();
const readWant = new Map<string, string>();
const latestSeen = new Map<string, string>();
const readBusy = new Set<string>();

function messageIdNumber(id: string): bigint | null {
  if (!/^[0-9]+$/.test(id)) {
    return null;
  }
  try {
    return BigInt(id);
  } catch {
    return null;
  }
}

function coversMessage(mark: string, messageId: string): boolean {
  const left = messageIdNumber(mark);
  const right = messageIdNumber(messageId);
  return left !== null && right !== null && left >= right;
}

function newestMessageId(ids: string[]): string {
  let best = "";
  let bestValue: bigint | null = null;
  for (const id of ids) {
    const value = messageIdNumber(id);
    if (value === null) {
      continue;
    }
    if (bestValue === null || value > bestValue) {
      best = id;
      bestValue = value;
    }
  }
  return best;
}

function readingChat(id: string): boolean {
  return id !== "" && id === openId && document.hasFocus() && document.visibilityState !== "hidden";
}

function dismissUnread(id: string): void {
  const chat = rows.find((row) => row.id === id);
  if (!chat?.unread) {
    return;
  }
  chat.unread = 0;
  paintUnread();
  chats.querySelector(".selected .badge")?.remove();
}

function noteLatest(id: string, messageId: string): void {
  if (!messageIdNumber(messageId)) {
    return;
  }
  const current = latestSeen.get(id) || "";
  if (!current || !coversMessage(current, messageId)) {
    latestSeen.set(id, messageId);
  }
  if (id === openId) {
    openLastId = latestSeen.get(id) || messageId;
  }
}

function holdRead(id: string, messageId: string): void {
  if (!messageIdNumber(messageId)) {
    return;
  }
  noteLatest(id, messageId);
  const coveredBy = readWant.get(id) || readSent.get(id) || "";
  if (coveredBy && coversMessage(coveredBy, messageId)) {
    dismissUnread(id);
    return;
  }
  readHeld.set(id, messageId);
  dismissUnread(id);
  readWant.set(id, messageId);
  void flushRead(id);
}

async function flushRead(id: string): Promise<void> {
  if (readBusy.has(id)) {
    return;
  }
  const target = readWant.get(id);
  if (!target) {
    return;
  }
  readBusy.add(id);
  readWant.delete(id);
  try {
    await invoke("mark_chat_read", { id, messageId: target });
    const sent = readSent.get(id) || "";
    if (!sent || !coversMessage(sent, target)) {
      readSent.set(id, target);
    }
  } catch {
    if (readHeld.get(id) === target) {
      readHeld.delete(id);
    }
  } finally {
    readBusy.delete(id);
  }
  const next = readWant.get(id) || "";
  if (next && !coversMessage(target, next)) {
    await flushRead(id);
  }
}

function catchUpRead(): void {
  if (!readingChat(openId)) {
    return;
  }
  const onScreen = Array.from(threadBody.querySelectorAll<HTMLElement>(".msg[data-id]"), (row) => row.dataset.id || "");
  const latest = newestMessageId([openLastId, ...onScreen]);
  if (latest) {
    holdRead(openId, latest);
  }
}

function showThread(chat: ChatRow | undefined): void {
  if (!chat) {
    thread.hidden = true;
    openId = "";
    openLastId = "";
    paintedIds = "";
    openUnread = 0;
    unreadFocus = "";
    closeGallery();
    closeReactionPicker();
    composerInput.disabled = true;
    sendError.hidden = true;
    threadHead.replaceChildren();
    delete document.body.dataset.sound;
    return;
  }
  thread.hidden = false;
  paintThreadHead(chat);
  composerInput.disabled = false;
  if (chat.id !== openId) {
    openId = chat.id;
    openLastId = "";
    paintedHistory = "";
    paintedIds = "";
    openUnread = chat.unread && chat.unread > 0 ? chat.unread : 0;
    unreadFocus = "";
    userMoved = false;
    closeGallery();
    closeReactionPicker();
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
  if (readingChat(chat.id)) {
    dismissUnread(chat.id);
  }
}

function historyStamp(payload: HistoryPayload): string {
  return payload.messages
    .map((message) => [message.id || "", message.sender || "", message.text, message.status || "", String(message.readers || 0), String(message.time)].join("\u001f"))
    .join("\u001e");
}

function renderHistory(payload: HistoryPayload): void {
  if (payload.id !== openId) {
    return;
  }
  const candidate = payload.lastMessageId || newestMessageId(payload.messages.map((message) => message.id || ""));
  if (candidate) {
    noteLatest(payload.id, candidate);
  }
  const stamp = historyStamp(payload);
  if (stamp === paintedHistory && !threadBody.querySelector(".thread-note")) {
    markOpenRead(payload.id);
    return;
  }
  const firstPaint = paintedIds === "" || Boolean(threadBody.querySelector(".thread-note"));
  const savedTop = threadBody.scrollTop;
  const previousIds = new Set(paintedIds ? paintedIds.split("\u001f") : []);
  paintedHistory = stamp;
  paintedIds = payload.messages.map((message) => message.id || "").join("\u001f");
  closeReactionPicker();
  threadBody.replaceChildren();
  if (payload.messages.length === 0) {
    const note = document.createElement("p");
    note.className = "thread-note";
    note.textContent = "No messages";
    threadBody.append(note);
    document.body.dataset.history = "0";
    unreadFocus = "";
    markOpenRead(payload.id);
    return;
  }
  groupMessages(payload.messages).forEach((message) => threadBody.append(messageBubble(message)));
  settleThread(payload.messages, previousIds, firstPaint, savedTop);
  document.body.dataset.history = String(payload.messages.length);
  markOpenRead(payload.id);
}

function incomingId(message: HistoryMessage): string {
  if (message.mine || message.kind === "chat update" || !message.id) {
    return "";
  }
  return message.id;
}

function firstUnreadId(messages: HistoryMessage[], count: number): string {
  if (count <= 0) {
    return "";
  }
  const incoming = messages.map(incomingId).filter((id) => id);
  if (incoming.length === 0) {
    return "";
  }
  return incoming[Math.max(0, incoming.length - count)] || "";
}

function bubbleById(id: string): HTMLElement | null {
  if (!id) {
    return null;
  }
  for (const row of threadBody.querySelectorAll<HTMLElement>(".msg")) {
    if (row.dataset.id === id || row.dataset.ids?.split(",").includes(id)) {
      return row;
    }
  }
  return null;
}

function threadNearBottom(): boolean {
  return threadBody.scrollHeight - threadBody.scrollTop - threadBody.clientHeight < 48;
}

function settleThread(messages: HistoryMessage[], previousIds: Set<string>, firstPaint: boolean, savedTop: number): void {
  if (firstPaint) {
    unreadFocus = firstUnreadId(messages, openUnread);
    openUnread = 0;
    placeThread();
    return;
  }
  const added = messages.filter((message) => message.id && !previousIds.has(message.id));
  if (added.length === 0 || userMoved) {
    threadBody.scrollTop = savedTop;
    return;
  }
  const incoming = added.map(incomingId).filter((id) => id);
  if (incoming.length > 0) {
    if (!unreadFocus) {
      unreadFocus = incoming[0] || "";
    }
    placeThread();
    return;
  }
  unreadFocus = "";
  scrollThreadToEnd();
}

function markOpenRead(id: string): void {
  if (!readingChat(id) || !openLastId) {
    return;
  }
  holdRead(id, openLastId);
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
  const callLabel = callPreviewLabel(text);
  const body = document.createElement("div");
  body.className = "msg-text";
  if (callLabel) {
    body.classList.add("call");
    body.append(phoneIcon(), document.createTextNode(callLabel));
  } else {
    body.textContent = text;
  }
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

const REACTIONS: { emoji: string; label: string }[] = [
  { emoji: "👍", label: "Like" },
  { emoji: "😎", label: "Cool" },
  { emoji: "😍", label: "Love" },
  { emoji: "😠", label: "Angry" },
  { emoji: "🥺", label: "Please" },
  { emoji: "😭", label: "Cry" },
  { emoji: "😮", label: "Wow" },
];

type ShownReaction = { emoji: string; image: string; mine: boolean };

const reactionOverride = new Map<string, ShownReaction>();

let reactionPicker: { panel: HTMLElement; toggle: HTMLButtonElement; bubble: HTMLElement } | null = null;

function reactionStorageKey(chatId: string, messageId: string): string {
  return `${chatId}:${messageId}`;
}

function shownReaction(chatId: string, messageId: string, message?: HistoryMessage): ShownReaction {
  if (chatId && messageId) {
    const saved = reactionOverride.get(reactionStorageKey(chatId, messageId));
    if (saved) {
      return saved;
    }
  }
  return {
    emoji: message?.reaction || "",
    image: message?.reactionImage || "",
    mine: message?.reactionMine !== false,
  };
}

function rememberReaction(chatId: string, messageId: string, reaction: ShownReaction): void {
  if (!chatId || !messageId) {
    return;
  }
  reactionOverride.set(reactionStorageKey(chatId, messageId), reaction);
}

function sendReaction(chatId: string, messageId: string, emoji: string, previous: ShownReaction, row: HTMLElement, chip: HTMLElement): void {
  const next: ShownReaction = { emoji, image: "", mine: true };
  rememberReaction(chatId, messageId, next);
  paintReaction(row, chip, next);
  void invoke("send_reaction", { id: chatId, messageId, emoji }).catch((error: unknown) => {
    rememberReaction(chatId, messageId, previous);
    paintReaction(row, chip, previous);
    sendError.hidden = false;
    sendError.textContent = error instanceof Error ? error.message : String(error);
  });
}

function closeReactionPicker(): void {
  if (!reactionPicker) {
    return;
  }
  reactionPicker.panel.remove();
  reactionPicker.toggle.setAttribute("aria-expanded", "false");
  reactionPicker.bubble.classList.remove("reacting");
  reactionPicker = null;
}

function paintReaction(row: HTMLElement, chip: HTMLElement, reaction: ShownReaction): void {
  chip.replaceChildren();
  if (reaction.emoji) {
    chip.textContent = reaction.emoji;
  } else if (reaction.image) {
    const img = document.createElement("img");
    img.alt = "";
    img.src = reaction.image;
    chip.append(img);
  }
  const visible = Boolean(reaction.emoji || reaction.image);
  chip.hidden = !visible;
  chip.dataset.mine = reaction.mine ? "1" : "0";
  chip.setAttribute("aria-label", visible && reaction.mine ? "Remove reaction" : "Reaction");
  row.classList.toggle("has-reaction", visible);
}

function applyLiveReaction(chatId: string, update: { messageId: string; emoji?: string; image?: string; mine?: boolean }): void {
  const reaction: ShownReaction = {
    emoji: update.emoji || "",
    image: update.emoji ? "" : (update.image || ""),
    mine: update.mine === true,
  };
  rememberReaction(chatId, update.messageId, reaction);
  if (chatId !== openId) {
    return;
  }
  const row = threadBody.querySelector<HTMLElement>(`.msg[data-id="${CSS.escape(update.messageId)}"]`);
  const bubble = row?.querySelector<HTMLElement>(".bubble");
  if (!row || !bubble) {
    return;
  }
  let chip = bubble.querySelector<HTMLElement>(".msg-reaction");
  if (!chip) {
    chip = document.createElement(row.classList.contains("mine") ? "span" : "button");
    chip.className = "msg-reaction";
    if (chip instanceof HTMLButtonElement) {
      chip.type = "button";
      chip.addEventListener("click", (event) => {
        event.stopPropagation();
        if (chip?.dataset.mine === "0") {
          return;
        }
        const previous = shownReaction(chatId, update.messageId);
        sendReaction(chatId, update.messageId, "", previous, row, chip!);
      });
    }
    bubble.append(chip);
  }
  paintReaction(row, chip, reaction);
}

function smileIcon(): SVGSVGElement {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  const face = document.createElementNS("http://www.w3.org/2000/svg", "circle");
  face.setAttribute("cx", "12");
  face.setAttribute("cy", "12");
  face.setAttribute("r", "8.25");
  face.setAttribute("fill", "none");
  face.setAttribute("stroke", "currentColor");
  face.setAttribute("stroke-width", "1.6");
  const leftEye = document.createElementNS("http://www.w3.org/2000/svg", "circle");
  leftEye.setAttribute("cx", "9");
  leftEye.setAttribute("cy", "10.2");
  leftEye.setAttribute("r", "0.9");
  leftEye.setAttribute("fill", "currentColor");
  const rightEye = document.createElementNS("http://www.w3.org/2000/svg", "circle");
  rightEye.setAttribute("cx", "15");
  rightEye.setAttribute("cy", "10.2");
  rightEye.setAttribute("r", "0.9");
  rightEye.setAttribute("fill", "currentColor");
  const mouth = document.createElementNS("http://www.w3.org/2000/svg", "path");
  mouth.setAttribute("fill", "none");
  mouth.setAttribute("stroke", "currentColor");
  mouth.setAttribute("stroke-width", "1.6");
  mouth.setAttribute("stroke-linecap", "round");
  mouth.setAttribute("d", "M8.7 13.6c.9 1.45 2 2.15 3.3 2.15s2.4-.7 3.3-2.15");
  svg.append(face, leftEye, rightEye, mouth);
  return svg;
}

function openReactionPicker(
  bubble: HTMLElement,
  row: HTMLElement,
  chip: HTMLButtonElement,
  toggle: HTMLButtonElement,
  chatId: string,
  messageId: string,
): void {
  closeReactionPicker();
  const panel = document.createElement("div");
  panel.className = "react-picker";
  panel.setAttribute("role", "menu");
  const current = messageId ? shownReaction(chatId, messageId) : { emoji: chip.textContent || "", image: "", mine: true };
  for (const reaction of REACTIONS) {
    const choice = document.createElement("button");
    choice.type = "button";
    choice.className = "react-choice";
    choice.textContent = reaction.emoji;
    choice.setAttribute("role", "menuitem");
    choice.setAttribute("aria-label", reaction.label);
    if (reaction.emoji === current.emoji) {
      choice.classList.add("on");
      choice.setAttribute("aria-pressed", "true");
    }
    choice.addEventListener("click", (event) => {
      event.stopPropagation();
      const next = reaction.emoji === current.emoji ? "" : reaction.emoji;
      if (messageId) {
        sendReaction(chatId, messageId, next, current, row, chip);
      } else {
        paintReaction(row, chip, { emoji: next, image: "", mine: true });
      }
      closeReactionPicker();
    });
    panel.append(choice);
  }
  panel.addEventListener("click", (event) => event.stopPropagation());
  const bridge = document.createElement("span");
  bridge.className = "react-bridge";
  bridge.setAttribute("aria-hidden", "true");
  panel.append(bridge);
  row.append(panel);
  bubble.classList.add("reacting");
  const smile = toggle.getBoundingClientRect();
  const box = panel.getBoundingClientRect();
  let left = smile.left + smile.width / 2 - box.width / 2;
  let top = smile.top - box.height - 8;
  const below = top < 8;
  if (below) {
    top = smile.bottom + 8;
  }
  left = Math.max(8, Math.min(left, window.innerWidth - box.width - 8));
  panel.style.left = `${left}px`;
  panel.style.top = `${top}px`;
  panel.classList.toggle("below", below);
  const placed = panel.getBoundingClientRect();
  const gap = below ? placed.top - smile.bottom : smile.top - placed.bottom;
  const reachLeft = Math.min(placed.left, smile.left);
  const reachRight = Math.max(placed.right, smile.right);
  bridge.style.left = `${reachLeft - placed.left}px`;
  bridge.style.width = `${Math.max(placed.width, reachRight - reachLeft)}px`;
  bridge.style.height = `${Math.max(10, gap + 12)}px`;
  const meta = toggle.parentElement instanceof HTMLElement ? toggle.parentElement : row;
  panel.addEventListener("mouseleave", (event) => leaveReaction(event, { bubble, meta, toggle }));
  toggle.setAttribute("aria-expanded", "true");
  reactionPicker = { panel, toggle, bubble };
}

function pointerStillChoosing(next: EventTarget | null, bubble: HTMLElement, meta: HTMLElement, toggle: HTMLButtonElement): boolean {
  if (!(next instanceof Node)) {
    return false;
  }
  if (bubble.contains(next) || meta.contains(next)) {
    return true;
  }
  return Boolean(reactionPicker?.toggle === toggle && reactionPicker.panel.contains(next));
}

function leaveReaction(event: MouseEvent, hover: { bubble: HTMLElement; meta: HTMLElement; toggle: HTMLButtonElement }): void {
  if (pointerStillChoosing(event.relatedTarget, hover.bubble, hover.meta, hover.toggle)) {
    return;
  }
  if (reactionPicker?.toggle === hover.toggle) {
    closeReactionPicker();
  }
}

function reactionControls(row: HTMLElement, bubble: HTMLElement, meta: HTMLElement, messageId: string): HTMLButtonElement {
  const chatId = openId;
  const toggle = document.createElement("button");
  toggle.type = "button";
  toggle.className = "react-toggle";
  toggle.setAttribute("aria-label", "Add reaction");
  toggle.setAttribute("aria-haspopup", "menu");
  toggle.setAttribute("aria-expanded", "false");
  toggle.append(smileIcon());
  const chip = document.createElement("button");
  chip.type = "button";
  chip.className = "msg-reaction";
  paintReaction(row, chip, shownReaction(chatId, messageId));
  chip.addEventListener("click", (event) => {
    event.stopPropagation();
    if (!messageId || chip.dataset.mine === "0") {
      return;
    }
    const previous = shownReaction(chatId, messageId);
    sendReaction(chatId, messageId, "", previous, row, chip);
    closeReactionPicker();
  });
  bubble.append(chip);
  const hover = { bubble, meta, toggle };
  bubble.addEventListener("mouseleave", (event) => leaveReaction(event, hover));
  meta.addEventListener("mouseleave", (event) => leaveReaction(event, hover));
  toggle.addEventListener("click", (event) => {
    event.stopPropagation();
    if (toggle.getAttribute("aria-expanded") === "true") {
      closeReactionPicker();
      return;
    }
    openReactionPicker(bubble, row, chip, toggle, chatId, messageId);
  });
  return toggle;
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
  const row = document.createElement("div");
  row.className = message.mine ? "msg mine" : "msg";
  const ids = [message.id || "", ...(message.media?.map((slot) => slot.id) || [])].filter((id) => id);
  if (message.id) {
    row.dataset.id = message.id;
  }
  if (ids.length > 0) {
    row.dataset.ids = ids.join(",");
  }
  const meta = messageMeta(message);
  const mark = senderMark(message);
  const reaction = shownReaction(openId, message.id || "", message);
  if (message.mine) {
    if (meta) {
      row.append(meta);
    }
    row.append(bubble, mark);
    if (reaction.emoji || reaction.image) {
      const chip = document.createElement("span");
      chip.className = "msg-reaction";
      paintReaction(row, chip, reaction);
      bubble.append(chip);
    }
  } else {
    row.append(mark, bubble);
    const slot = meta ?? document.createElement("div");
    if (!meta) {
      slot.className = "msg-meta";
    }
    slot.append(reactionControls(row, bubble, slot, message.id || ""));
    row.append(slot);
  }
  return row;
}

function messageMeta(message: BubbleMessage): HTMLElement | null {
  const time = message.time ? clock(message.time) : "";
  const read = message.mine && message.status === "read";
  if (!time && !read) {
    return null;
  }
  const meta = document.createElement("div");
  meta.className = "msg-meta";
  if (read) {
    meta.append(receiptNode(message.readers || 0));
  }
  if (time) {
    const stamp = document.createElement("span");
    stamp.className = "msg-time";
    stamp.textContent = time;
    meta.append(stamp);
  }
  return meta;
}

function receiptNode(readers: number): HTMLElement {
  const mark = document.createElement("span");
  mark.className = "receipt";
  mark.textContent = readers > 1 ? `Read ${readers}` : "Read";
  return mark;
}

function markReached(end: string, messageId: string): boolean {
  try {
    return BigInt(end) >= BigInt(messageId);
  } catch {
    return false;
  }
}

function applyReadMarks(chatId: string, marks: ReadMark[]): void {
  if (chatId !== openId || marks.length === 0) {
    return;
  }
  threadBody.querySelectorAll<HTMLElement>(".msg.mine[data-id]").forEach((row) => {
    const id = row.dataset.id || "";
    const readers = marks.filter((mark) => markReached(mark.end, id)).length;
    let meta = row.querySelector(".msg-meta");
    if (!meta) {
      const bubble = row.querySelector(".bubble");
      if (!bubble) {
        return;
      }
      meta = document.createElement("div");
      meta.className = "msg-meta";
      row.insertBefore(meta, bubble);
    }
    meta.querySelector(".receipt")?.remove();
    if (readers > 0) {
      const time = meta.querySelector(".msg-time");
      const mark = receiptNode(readers);
      if (time) {
        meta.insertBefore(mark, time);
      } else {
        meta.append(mark);
      }
    }
  });
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

function profileId(message: BubbleMessage): string {
  if (message.sender?.startsWith("u")) {
    return message.sender;
  }
  if (message.mine && myMid.startsWith("u")) {
    return myMid;
  }
  if (!message.mine && openId.startsWith("u")) {
    return openId;
  }
  return "";
}

function openProfile(message: BubbleMessage): void {
  const id = profileId(message);
  if (!id) {
    return;
  }
  const friend = rows.some((row) => row.id === id && row.kind === "friend");
  void invoke("open_profile", {
    id,
    name: senderName(message),
    picture: senderPicture(message),
    mine: message.mine || id === myMid,
    friend,
  }).catch((error: unknown) => {
    sendError.hidden = false;
    sendError.textContent = error instanceof Error ? error.message : String(error);
  });
}

function senderMark(message: BubbleMessage): HTMLElement {
  const name = senderName(message);
  const picture = senderPicture(message);
  const button = document.createElement("button");
  button.type = "button";
  button.className = "msg-avatar";
  button.setAttribute("aria-label", name ? `Profile of ${name}` : "Profile");
  if (picture) {
    const photo = document.createElement("img");
    photo.alt = "";
    photo.src = picture;
    photo.addEventListener("error", () => {
      photo.replaceWith(fallback(name));
    });
    button.append(photo);
  } else {
    button.append(fallback(name));
  }
  button.addEventListener("click", (event) => {
    event.stopPropagation();
    openProfile(message);
  });
  return button;
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
    const callLabel = callPreviewLabel(chat.lastMessage);
    if (callLabel) {
      bottom.append(phoneIcon());
    } else if (chat.sticker) {
      bottom.append(stickerImage(chat.sticker, chat.stickerFallback || "", "preview-sticker"));
    }
    const preview = document.createElement("span");
    preview.className = "preview";
    preview.textContent = callLabel ?? previewText(chat.lastMessage);
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

function callPreviewLabel(text: string): string | null {
  const trimmed = text.trim();
  const duration = trimmed.match(/(\d+(?:\.\d+)?)\s*(milli(?:seconds?|secs?)|ms|seconds?)\b/i);
  const callLog = /call history/i.test(trimmed) || (/\bvideo call\b/i.test(trimmed) && Boolean(duration));
  if (!callLog) {
    return null;
  }
  const amount = duration ? Number(duration[1]) : 0;
  const unit = duration?.[2].toLowerCase() ?? "";
  const total = unit.startsWith("milli") || unit === "ms" ? Math.round(amount / 1000) : Math.round(amount);
  const safe = Math.max(0, total);
  const minutes = Math.floor(safe / 60);
  const seconds = safe % 60;
  const pad = (value: number) => String(value).padStart(2, "0");
  return `Call time ${pad(minutes)}:${pad(seconds)}`;
}

function phoneIcon(): SVGSVGElement {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  svg.classList.add("preview-call");
  const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
  path.setAttribute("fill", "currentColor");
  path.setAttribute(
    "d",
    "M6.62 10.79a15.05 15.05 0 0 0 6.59 6.59l2.2-2.2a1 1 0 0 1 1.02-.24c1.12.37 2.33.57 3.57.57a1 1 0 0 1 1 1V20a1 1 0 0 1-1 1C10.61 21 3 13.39 3 4a1 1 0 0 1 1-1h3.5a1 1 0 0 1 1 1c0 1.25.2 2.45.57 3.57a1 1 0 0 1-.25 1.02l-2.2 2.2z",
  );
  svg.append(path);
  return svg;
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
  if (profile.mid) {
    myMid = profile.mid;
  }
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
      id: payload.messageId,
      mine: true,
      name: "",
      text: payload.text,
      sealed: false,
      status: "sent",
      time: payload.time || Date.now(),
    }));
    unreadFocus = "";
    scrollThreadToEnd();
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

document.addEventListener("click", () => {
  closeMenu();
  closeReactionPicker();
});

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
  if (event.key === "Escape" && reactionPicker && lightbox.hidden) {
    closeReactionPicker();
    return;
  }
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

function appendLiveMessage(update: PollUpdate, followTail: boolean, batch: { focus: string; end: boolean }): void {
  const message = update.thread;
  if (!message?.text || update.id !== openId) {
    return;
  }
  if (message.id && threadBody.querySelector(`.msg[data-id="${CSS.escape(message.id)}"]`)) {
    return;
  }
  threadBody.querySelector(".thread-note")?.remove();
  threadBody.append(messageBubble(message));
  if (message.mine) {
    batch.end = true;
    return;
  }
  const id = incomingId(message);
  if (id && followTail && !batch.focus) {
    batch.focus = id;
  }
}

function scrollThreadToEnd(): void {
  const token = ++placeToken;
  userMoved = false;
  const pin = (): void => {
    if (token !== placeToken) {
      return;
    }
    placingThread = true;
    threadBody.scrollTop = threadBody.scrollHeight;
    requestAnimationFrame(() => {
      placingThread = false;
    });
  };
  pin();
  requestAnimationFrame(pin);
}

function placeThread(): void {
  const token = ++placeToken;
  const id = unreadFocus;
  const target = id ? bubbleById(id) : null;
  userMoved = false;
  if (!target) {
    scrollThreadToEnd();
    return;
  }
  const pin = (): void => {
    if (token !== placeToken || !target.isConnected) {
      return;
    }
    const pad = parseFloat(getComputedStyle(threadBody).paddingTop) || 0;
    const delta = target.getBoundingClientRect().top - threadBody.getBoundingClientRect().top - pad;
    const next = Math.max(0, threadBody.scrollTop + delta);
    const tail = threadBody.scrollHeight - next;
    placingThread = true;
    threadBody.scrollTop = tail <= threadBody.clientHeight + 1 ? threadBody.scrollHeight : next;
    requestAnimationFrame(() => {
      placingThread = false;
    });
  };
  pin();
  requestAnimationFrame(pin);
  threadBody.querySelectorAll("img, video").forEach((node) => {
    if (node instanceof HTMLImageElement && node.complete) {
      return;
    }
    const refresh = (): void => {
      if (token === placeToken && unreadFocus === id && target.isConnected) {
        pin();
      }
    };
    node.addEventListener("load", refresh, { once: true });
    node.addEventListener("loadedmetadata", refresh, { once: true });
  });
}

threadBody.addEventListener("scroll", () => {
  closeReactionPicker();
  if (placingThread) {
    return;
  }
  placeToken += 1;
  if (threadNearBottom()) {
    userMoved = false;
    unreadFocus = "";
  } else {
    userMoved = true;
  }
});

function applyServerUnread(chat: ChatRow, count: number, messageId = ""): void {
  if (messageId) {
    noteLatest(chat.id, messageId);
  }
  const sent = readSent.get(chat.id) || "";
  const held = readHeld.get(chat.id) || "";
  const cursor = coversMessage(held, sent) ? held : (sent || held);
  const newer = Boolean(cursor && messageId && !coversMessage(cursor, messageId));
  if (readingChat(chat.id) && messageId) {
    holdRead(chat.id, messageId);
    return;
  }
  if (readingChat(chat.id)) {
    chat.unread = 0;
    return;
  }
  if (newer) {
    readHeld.delete(chat.id);
    readSent.delete(chat.id);
    chat.unread = count;
    return;
  }
  if (cursor && count > 0) {
    chat.unread = 0;
    return;
  }
  chat.unread = count;
}

function applyPoll(payload: PollPayload): void {
  if (!signedIn) {
    return;
  }
  const selectedId = visibleRows()[selected]?.id || openId;
  const openTime = rows.find((row) => row.id === openId)?.time || 0;
  for (const item of payload.unreads || []) {
    const chat = rows.find((row) => row.id === item.id);
    if (chat) {
      applyServerUnread(chat, item.unread);
    }
  }
  let openAdvanced = false;
  let shouldChime = false;
  const followTail = threadNearBottom();
  const batch = { focus: "", end: false };
  for (const update of payload.updates || []) {
    if (update.reaction?.messageId) {
      applyLiveReaction(update.id, update.reaction);
      continue;
    }
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
    if (update.readMarks?.length) {
      applyReadMarks(update.id, update.readMarks);
    }
    const arrived = update.messageId || update.thread?.id || "";
    if (arrived) {
      noteLatest(chat.id, arrived);
    }
    if (update.read) {
      const seen = latestSeen.get(chat.id) || "";
      if (!(arrived && seen && !coversMessage(arrived, seen))) {
        chat.unread = 0;
        if (arrived) {
          const held = readHeld.get(chat.id) || "";
          if (!held || !coversMessage(held, arrived)) {
            readHeld.set(chat.id, arrived);
          }
          const sent = readSent.get(chat.id) || "";
          if (!sent || !coversMessage(sent, arrived)) {
            readSent.set(chat.id, arrived);
          }
        }
      }
    } else if (update.incoming) {
      if (readingChat(chat.id) && arrived) {
        holdRead(chat.id, arrived);
      } else if (readingChat(chat.id) || coversMessage(readSent.get(chat.id) || readHeld.get(chat.id) || "", arrived)) {
        chat.unread = 0;
      } else {
        readHeld.delete(chat.id);
        if (!arrived || !coversMessage(readSent.get(chat.id) || "", arrived)) {
          readSent.delete(chat.id);
        }
        chat.unread = (chat.unread || 0) + 1;
        if (notifySound(chat.id, chat.muted)) {
          shouldChime = true;
        }
      }
    } else if (typeof update.unread === "number") {
      applyServerUnread(chat, update.unread, arrived);
    }
    const readable = Boolean(update.lastMessage) && !update.keepPreview && !updateLabel(update.lastMessage);
    const newer = update.time > chat.time;
    const fill = Boolean(update.refresh && update.lastMessage && !update.keepPreview);
    if (newer || fill || (readable && update.lastMessage !== chat.lastMessage)) {
      const keepTyped = !fill
        && !readable
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
      }
      if (newer) {
        chat.time = update.time;
      }
      if (chat.id === openId && (newer || readable) && update.time >= openTime) {
        openAdvanced = true;
      }
    }
    appendLiveMessage(update, followTail, batch);
  }
  if (batch.end) {
    unreadFocus = "";
    scrollThreadToEnd();
  } else if (batch.focus) {
    unreadFocus = batch.focus;
    placeThread();
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
  myMid = "";
  readHeld.clear();
  readSent.clear();
  readWant.clear();
  latestSeen.clear();
  openLastId = "";
  paintedIds = "";
  openUnread = 0;
  unreadFocus = "";
  closeMenu();
  closeReactionPicker();
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

(window as unknown as { __openChat?: (id: string) => void }).__openChat = (id: string) => {
  search.value = "";
  if (filter !== "all") {
    filter = "all";
    document.querySelectorAll("#tabs button").forEach((tab) => {
      tab.classList.toggle("on", (tab as HTMLButtonElement).dataset.filter === "all");
    });
  }
  const index = visibleRows().findIndex((row) => row.id === id);
  if (index < 0) {
    return;
  }
  selected = index;
  renderList();
};

(window as unknown as { __renameChat?: (detail: { id?: string; name?: string }) => void }).__renameChat = (detail) => {
  const chat = rows.find((row) => row.id === detail.id);
  const name = detail.name?.trim() || "";
  if (!chat || !name) {
    return;
  }
  chat.name = name;
  renderList();
};

refresh.addEventListener("click", () => {
  void start();
});

void start();
