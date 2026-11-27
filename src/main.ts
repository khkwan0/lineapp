import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import QRCode from "qrcode";
import { applyI18n, intlLocale, languageName, languagePreference, LOCALES, onLocaleChange, setLanguage, t, type MessageKey } from "./i18n";

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
  replyId?: string;
  replyText?: string;
  replyName?: string;
  replyPicture?: string;
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
  localId?: string;
  code?: number | null;
  error?: string;
  replyId?: string;
  kind?: string;
  mediaId?: string;
  animated?: boolean;
  sticker?: string;
  stickerFallback?: string;
  fileName?: string;
  gid?: string;
  seq?: number;
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
const composerInput = document.querySelector<HTMLTextAreaElement>("#composer-input")!;
const attachFile = document.querySelector<HTMLButtonElement>("#attach-file")!;
const stickerOpen = document.querySelector<HTMLButtonElement>("#sticker-open")!;
const stickerPanel = document.querySelector<HTMLElement>("#sticker-panel")!;
const fileInput = document.querySelector<HTMLInputElement>("#file-input")!;
const sendButton = document.querySelector<HTMLButtonElement>("#send-message")!;
const attachPreview = document.querySelector<HTMLElement>("#attach-preview")!;
const sendError = document.querySelector<HTMLParagraphElement>("#send-error")!;
const railMore = document.querySelector<HTMLButtonElement>("#rail-more")!;
const railMenu = document.querySelector<HTMLElement>("#rail-menu")!;
const sheet = document.querySelector<HTMLElement>("#sheet")!;
const sheetTitle = document.querySelector<HTMLElement>("#sheet-title")!;
const sheetBody = document.querySelector<HTMLElement>("#sheet-body")!;
const sheetSettings = document.querySelector<HTMLElement>("#sheet-settings")!;
const sheetAbout = document.querySelector<HTMLElement>("#sheet-about")!;
const aboutBuild = document.querySelector<HTMLElement>("#about-build")!;
const aboutTime = document.querySelector<HTMLElement>("#about-time")!;
const minimizeToTray = document.querySelector<HTMLInputElement>("#minimize-to-tray")!;
const devicesOutput = document.querySelector<HTMLElement>("#devices-output")!;
const devicesInput = document.querySelector<HTMLElement>("#devices-input")!;
const devicesCamera = document.querySelector<HTMLElement>("#devices-camera")!;
const devicesNote = document.querySelector<HTMLElement>("#devices-note")!;
const lightbox = document.querySelector<HTMLElement>("#lightbox")!;
const lightboxImg = document.querySelector<HTMLImageElement>("#lightbox-img")!;
const lightboxVideo = document.querySelector<HTMLVideoElement>("#lightbox-video")!;
const lightboxPrev = document.querySelector<HTMLButtonElement>("#lightbox-prev")!;
const lightboxNext = document.querySelector<HTMLButtonElement>("#lightbox-next")!;
const msgMenu = document.querySelector<HTMLElement>("#msg-menu")!;
const replyBar = document.querySelector<HTMLElement>("#reply-bar")!;
const replyAvatar = document.querySelector<HTMLElement>("#reply-avatar")!;
const replyLabel = document.querySelector<HTMLElement>("#reply-label")!;
const replyText = document.querySelector<HTMLElement>("#reply-text")!;
const sheetList = document.querySelector<HTMLElement>("#sheet-list")!;
const sheetLanguage = document.querySelector<HTMLElement>("#sheet-language")!;
const languageCurrent = document.querySelector<HTMLElement>("#language-current")!;
const languageList = document.querySelector<HTMLUListElement>("#language-list")!;
const languageOpen = document.querySelector<HTMLButtonElement>("#language-open")!;
const languageSave = document.querySelector<HTMLButtonElement>("#language-save")!;
const languageCancel = document.querySelector<HTMLButtonElement>("#language-cancel")!;

let statusKey: MessageKey | "" = "";
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
let openHistory: HistoryPayload | null = null;
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

type ReplyDraft = { id: string; text: string; name: string; picture: string };
type Outgoing = {
  localId: string;
  chatId: string;
  text: string;
  reply: ReplyDraft | null;
  time: number;
  state: "sending" | "failed";
  error: string;
  knownIds: string;
  kind?: string;
  previewUrl?: string;
  sticker?: string;
  stickerFallback?: string;
  albumUrls?: string[];
  file?: { name: string; mime: string; data: string; groupId?: string; groupSeq?: number; groupCount?: number };
  stickerSend?: { packageId: string; stickerId: string; animated: boolean };
};
type AttachmentDraft = {
  chatId: string;
  file: File;
  name: string;
  mime: string;
  kind: "image" | "video" | "audio" | "file";
  previewUrl: string;
};
let replyDraft: ReplyDraft | null = null;
const IMAGE_CAP = 10;
let attachmentDrafts: AttachmentDraft[] = [];
let attachmentEpoch = 0;
let sendBusy = false;
let msgMenuRow: HTMLElement | null = null;
let shareText = "";
const outgoing: Outgoing[] = [];
let outgoingSeq = 0;
let menuOpenedAt = 0;

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
  unsentId?: string;
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

function setStatus(key: MessageKey): void {
  statusKey = key;
  status.textContent = t(key);
}

function setStatusText(text: string): void {
  statusKey = "";
  status.textContent = text;
}

async function onEvent(event: LoginEvent): Promise<void> {
  if (event.type === "qr" && event.url) {
    if (event.url === shownUrl) {
      return;
    }
    shownUrl = event.url;
    await drawQr(event.url);
    setStatus("scanQr");
    return;
  }
  if (event.type === "pin" && event.pin) {
    pin.hidden = false;
    pin.textContent = event.pin;
    setStatus("confirmPin");
    return;
  }
  if (event.type === "status" && event.message) {
    setStatusText(event.message);
    return;
  }
  if (event.type === "saved") {
    setStatus("signedIn");
    refresh.hidden = true;
    await showChats();
    return;
  }
  if (event.type === "error" && event.message) {
    setStatusText(event.message);
  }
}

function clock(ms: number): string {
  if (!ms) {
    return "";
  }
  const date = new Date(ms);
  const now = new Date();
  const day = date.toLocaleDateString(intlLocale(), { timeZone: "Asia/Bangkok" });
  const today = now.toLocaleDateString(intlLocale(), { timeZone: "Asia/Bangkok" });
  const yesterdayDate = new Date(now.getTime() - 24 * 60 * 60 * 1000);
  const yesterday = yesterdayDate.toLocaleDateString(intlLocale(), { timeZone: "Asia/Bangkok" });
  if (day === today) {
    return date.toLocaleTimeString(intlLocale(), {
      hour: "numeric",
      minute: "2-digit",
      hour12: true,
      timeZone: "Asia/Bangkok",
    });
  }
  if (day === yesterday) {
    return t("yesterday");
  }
  return date.toLocaleDateString(intlLocale(), {
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

function openMembers(chat: ChatRow): void {
  void invoke("open_members", { id: chat.id, name: chat.name, picture: chat.picture }).catch((error: unknown) => {
    sendError.hidden = false;
    sendError.textContent = error instanceof Error ? error.message : String(error);
  });
}

function openDirectProfile(chat: ChatRow): void {
  if (!chat.id.startsWith("u")) {
    return;
  }
  void invoke("open_profile", {
    id: chat.id,
    name: chat.name,
    picture: chat.picture,
    mine: chat.id === myMid,
    friend: chat.kind === "friend",
  }).catch((error: unknown) => {
    sendError.hidden = false;
    sendError.textContent = error instanceof Error ? error.message : String(error);
  });
}

function paintThreadHead(chat: ChatRow): void {
  const muted = soundMuted(chat);
  threadHead.replaceChildren();
  const count = chat.members === null ? "" : ` (${chat.members})`;
  const label = `${displayName(chat.name)}${count}`;
  if (chat.kind === "group") {
    const title = document.createElement("button");
    title.type = "button";
    title.className = "thread-title";
    title.textContent = label;
    title.setAttribute("aria-label", t("membersOf", { name: chat.name }));
    title.addEventListener("click", () => openMembers(chat));
    threadHead.append(title);
  } else if (chat.kind === "friend" && chat.id.startsWith("u")) {
    const title = document.createElement("button");
    title.type = "button";
    title.className = "thread-title";
    title.textContent = label;
    title.setAttribute("aria-label", chat.name ? t("profileOf", { name: chat.name }) : t("profile"));
    title.addEventListener("click", () => openDirectProfile(chat));
    threadHead.append(title);
  } else {
    const title = document.createElement("span");
    title.className = "thread-title";
    title.textContent = label;
    threadHead.append(title);
  }
  const button = document.createElement("button");
  button.type = "button";
  button.className = muted ? "thread-mute is-muted" : "thread-mute";
  button.dataset.muted = muted ? "1" : "0";
  button.setAttribute("aria-pressed", muted ? "true" : "false");
  button.setAttribute("aria-label", muted ? t("unmute") : t("mute"));
  button.append(speakerIcon(muted));
  button.addEventListener("click", () => {
    setSoundMuted(chat.id, !soundMuted(chat));
    renderList();
  });
  threadHead.append(button);
  document.body.dataset.sound = muted ? "off" : "on";
}

function patchChat(detail: { id?: string; name?: string; members?: number | null }): void {
  const chat = rows.find((row) => row.id === detail.id);
  if (!chat) {
    return;
  }
  const name = detail.name?.trim() || "";
  const nameChanged = Boolean(name && name !== chat.name);
  const membersChanged = typeof detail.members === "number" && detail.members !== chat.members;
  if (nameChanged) {
    chat.name = name;
    renderList();
  }
  if (membersChanged && typeof detail.members === "number") {
    chat.members = detail.members;
  }
  if ((nameChanged || membersChanged) && chat.id === openId) {
    paintThreadHead(chat);
  }
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
    closeStickerPanel();
    composerInput.disabled = true;
    attachFile.disabled = true;
    stickerOpen.disabled = true;
    discardAttachment();
    sendError.hidden = true;
    threadHead.replaceChildren();
    delete document.body.dataset.sound;
    refreshSendButton();
    return;
  }
  thread.hidden = false;
  paintThreadHead(chat);
  composerInput.disabled = false;
  attachFile.disabled = false;
  stickerOpen.disabled = false;
  if (chat.id !== openId) {
    armedStickers.clear();
    stickerWatch?.disconnect();
    stickerWatch = null;
    closeStickerPanel();
    openId = chat.id;
    openLastId = "";
    paintedHistory = "";
    paintedIds = "";
    openUnread = chat.unread && chat.unread > 0 ? chat.unread : 0;
    unreadFocus = "";
    userMoved = false;
    closeGallery();
    closeReactionPicker();
    closeMsgMenu();
    clearReply();
    discardAttachment();
    sendError.hidden = true;
    const token = ++historyToken;
    threadBody.replaceChildren();
    const note = document.createElement("p");
    note.className = "thread-note";
    note.dataset.kind = "loading";
    note.textContent = t("loadingMessages");
    threadBody.append(note);
    void invoke("load_history", { id: chat.id }).catch((error: unknown) => {
      if (token !== historyToken) {
        return;
      }
      note.textContent = error instanceof Error ? error.message : String(error);
    });
  }
  refreshSendButton();
  if (readingChat(chat.id)) {
    dismissUnread(chat.id);
  }
}

function historyStamp(payload: HistoryPayload): string {
  return payload.messages
    .map((message) => [message.id || "", message.sender || "", message.text, message.status || "", message.kind || "", message.mediaId || "", message.gid || "", String(message.seq || 0), message.sticker || "", message.replyId || "", String(message.readers || 0), String(message.time)].join("\u001f"))
    .join("\u001e");
}

function renderHistory(payload: HistoryPayload): void {
  if (payload.id !== openId) {
    return;
  }
  openHistory = payload;
  const candidate = payload.lastMessageId || newestMessageId(payload.messages.map((message) => message.id || ""));
  if (candidate) {
    noteLatest(payload.id, candidate);
  }
  const stamp = historyStamp(payload);
  if (stamp === paintedHistory && !threadBody.querySelector(".thread-note")) {
    if (placeOutgoing(payload.id) && !userMoved) {
      scrollThreadToEnd();
    }
    markOpenRead(payload.id);
    return;
  }
  const firstPaint = paintedIds === "" || Boolean(threadBody.querySelector(".thread-note"));
  const savedTop = threadBody.scrollTop;
  const previousIds = new Set(paintedIds ? paintedIds.split("\u001f") : []);
  claimOutgoing(payload.id, payload.messages);
  armUnreadStickers(payload.messages, previousIds, firstPaint);
  paintedHistory = stamp;
  paintedIds = payload.messages.map((message) => message.id || "").join("\u001f");
  closeReactionPicker();
  closeMsgMenu();
  threadBody.replaceChildren();
  if (payload.messages.length === 0 && !outgoing.some((item) => item.chatId === payload.id)) {
    const note = document.createElement("p");
    note.className = "thread-note";
    note.dataset.kind = "empty";
    note.textContent = t("noMessages");
    threadBody.append(note);
    document.body.dataset.history = "0";
    unreadFocus = "";
    markOpenRead(payload.id);
    return;
  }
  groupMessages(hydrateReplies(payload.messages)).forEach((message) => threadBody.append(messageBubble(message)));
  const placed = placeOutgoing(payload.id);
  settleThread(payload.messages, previousIds, firstPaint, savedTop);
  if (placed && !userMoved) {
    scrollThreadToEnd();
  }
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

const playedStickers = new Set<string>();
const armedStickers = new Set<string>();
let stickerWatch: IntersectionObserver | null = null;

function transferStickerPlay(from: string, to: string): void {
  if (!from || !to || from === to) {
    return;
  }
  if (playedStickers.has(from)) {
    playedStickers.add(to);
    armedStickers.delete(to);
  } else if (armedStickers.has(from)) {
    armedStickers.add(to);
  }
  armedStickers.delete(from);
}

function armUnreadStickers(messages: HistoryMessage[], previousIds: Set<string>, firstPaint: boolean): void {
  if (firstPaint) {
    if (openUnread <= 0) {
      return;
    }
    const incoming = messages.map(incomingId).filter((id) => id);
    for (const id of incoming.slice(Math.max(0, incoming.length - openUnread))) {
      if (!playedStickers.has(id)) {
        armedStickers.add(id);
      }
    }
    return;
  }
  for (const message of messages) {
    const id = message.id || "";
    if (!id || previousIds.has(id) || playedStickers.has(id) || !message.sticker?.includes("/sticker_animation.png")) {
      continue;
    }
    if (message.mine || incomingId(message)) {
      armedStickers.add(id);
    }
  }
}

function stickerInView(node: HTMLElement): boolean {
  const root = threadBody.getBoundingClientRect();
  const box = node.getBoundingClientRect();
  const visible = Math.min(box.bottom, root.bottom) - Math.max(box.top, root.top);
  return visible > box.height * 0.5;
}

function watchSticker(host: HTMLElement): void {
  if (!stickerWatch) {
    stickerWatch = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) {
          continue;
        }
        const node = entry.target as HTMLElement;
        const id = node.dataset.playId || "";
        if (!id || playedStickers.has(id) || !armedStickers.has(id)) {
          stickerWatch?.unobserve(node);
          continue;
        }
        if (!stickerInView(node)) {
          continue;
        }
        stickerWatch?.unobserve(node);
        startStickerPlay(node, id);
      }
    }, { root: threadBody, threshold: 0.5 });
  }
  stickerWatch.observe(host);
}

function startStickerPlay(host: HTMLElement, id: string): void {
  armedStickers.delete(id);
  playedStickers.add(id);
  const src = host.dataset.src || "";
  void playStickerAnimation(src).then((movie) => {
    if (!movie || !host.isConnected || !stickerInView(host)) {
      movie?.frames.forEach((frame) => frame.image.close());
      if (host.isConnected && !stickerInView(host)) {
        playedStickers.delete(id);
        armedStickers.add(id);
        watchSticker(host);
      }
      return;
    }
    const canvas = document.createElement("canvas");
    canvas.className = "sticker";
    canvas.dataset.src = src;
    canvas.dataset.fallback = host.dataset.fallback || "";
    host.replaceChildren(canvas);
    runStickerMovie(canvas, movie);
  });
}

function stillSticker(src: string, fallback: string): string {
  if (src.includes("/sticker_animation.png")) {
    return src.replace("/sticker_animation.png", "/sticker.png");
  }
  return fallback || src;
}

function stickerImage(
  src: string,
  fallback: string,
  className: string,
  onFail?: () => void,
  messageId = "",
): HTMLElement {
  const still = stillSticker(src, fallback);
  const img = document.createElement("img");
  img.className = className;
  img.alt = "";
  img.dataset.src = src;
  img.dataset.fallback = still;
  img.src = className === "sticker" && src.includes("/sticker_animation.png") ? still : src;
  img.addEventListener("error", () => {
    const next = img.dataset.fallback || "";
    if (next && img.src !== next) {
      img.src = next;
      return;
    }
    if (fallback && img.src !== fallback && fallback !== next) {
      img.src = fallback;
      return;
    }
    img.remove();
    onFail?.();
  });
  if (className !== "sticker" || !src.includes("/sticker_animation.png") || !messageId || playedStickers.has(messageId) || !armedStickers.has(messageId)) {
    return img;
  }
  const host = document.createElement("span");
  host.className = "sticker-play";
  host.dataset.playId = messageId;
  host.dataset.src = src;
  host.dataset.fallback = still;
  host.append(img);
  watchSticker(host);
  return host;
}

type StickerFrame = {
  image: ImageBitmap;
  x: number;
  y: number;
  width: number;
  height: number;
  delay: number;
  dispose: number;
  blend: number;
};

type StickerMovie = { width: number; height: number; plays: number; frames: StickerFrame[] };

function pngCrc(bytes: Uint8Array): number {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ (crc & 1 ? 0xedb88320 : 0);
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function pngChunk(type: string, data: Uint8Array): Uint8Array {
  const out = new Uint8Array(12 + data.length);
  const view = new DataView(out.buffer);
  view.setUint32(0, data.length);
  out.set(new TextEncoder().encode(type), 4);
  out.set(data, 8);
  const covered = out.subarray(4, 8 + data.length);
  view.setUint32(8 + data.length, pngCrc(covered));
  return out;
}

async function playStickerAnimation(url: string): Promise<StickerMovie | null> {
  try {
    const bytes = new Uint8Array(await (await fetch(url)).arrayBuffer());
    const signature = [137, 80, 78, 71, 13, 10, 26, 10];
    if (signature.some((byte, index) => bytes[index] !== byte)) {
      return null;
    }
    let cursor = 8;
    let width = 0;
    let height = 0;
    let plays = 0;
    const prefix: Uint8Array[] = [];
    const frames: { meta: Uint8Array; data: Uint8Array[] }[] = [];
    let current: { meta: Uint8Array; data: Uint8Array[] } | null = null;
    while (cursor + 8 <= bytes.length) {
      const length = new DataView(bytes.buffer, bytes.byteOffset + cursor, 4).getUint32(0);
      const type = String.fromCharCode(...bytes.subarray(cursor + 4, cursor + 8));
      const data = bytes.subarray(cursor + 8, cursor + 8 + length);
      cursor += 12 + length;
      if (type === "IHDR") {
        width = new DataView(data.buffer, data.byteOffset, 4).getUint32(0);
        height = new DataView(data.buffer, data.byteOffset + 4, 4).getUint32(0);
      } else if (type === "acTL") {
        plays = new DataView(data.buffer, data.byteOffset + 4, 4).getUint32(0);
      } else if (type === "fcTL") {
        current = { meta: data.slice(), data: [] };
        frames.push(current);
      } else if (type === "IDAT" && current) {
        current.data.push(data.slice());
      } else if (type === "fdAT" && current) {
        current.data.push(data.subarray(4).slice());
      } else if (type === "IEND") {
        break;
      } else if (!current && type !== "IHDR" && type !== "acTL") {
        prefix.push(pngChunk(type, data.slice()));
      }
    }
    if (!width || !height || frames.length === 0) {
      return null;
    }
    const header = bytes.subarray(16, 29);
    const decoded: StickerFrame[] = [];
    for (const frame of frames) {
      const view = new DataView(frame.meta.buffer, frame.meta.byteOffset, frame.meta.byteLength);
      const frameWidth = view.getUint32(4);
      const frameHeight = view.getUint32(8);
      const ihdr = new Uint8Array(13);
      const ihdrView = new DataView(ihdr.buffer);
      ihdrView.setUint32(0, frameWidth);
      ihdrView.setUint32(4, frameHeight);
      ihdr.set(header.subarray(8), 8);
      const idat = new Uint8Array(frame.data.reduce((sum, part) => sum + part.length, 0));
      let offset = 0;
      for (const part of frame.data) {
        idat.set(part, offset);
        offset += part.length;
      }
      const parts = [Uint8Array.from(signature), pngChunk("IHDR", ihdr), ...prefix, pngChunk("IDAT", idat), pngChunk("IEND", new Uint8Array())];
      const png = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
      let at = 0;
      for (const part of parts) {
        png.set(part, at);
        at += part.length;
      }
      const image = await createImageBitmap(new Blob([png.buffer], { type: "image/png" }));
      const denominator = view.getUint16(22) || 100;
      decoded.push({
        image,
        x: view.getUint32(12),
        y: view.getUint32(16),
        width: frameWidth,
        height: frameHeight,
        delay: Math.max(20, (view.getUint16(20) / denominator) * 1000),
        dispose: frame.meta[24] || 0,
        blend: frame.meta[25] || 0,
      });
    }
    return { width, height, plays, frames: decoded };
  } catch {
    return null;
  }
}

function runStickerMovie(canvas: HTMLCanvasElement, movie: StickerMovie): void {
  const context = canvas.getContext("2d");
  if (!context) {
    return;
  }
  canvas.width = movie.width;
  canvas.height = movie.height;
  const snapshot = document.createElement("canvas");
  snapshot.width = movie.width;
  snapshot.height = movie.height;
  const saved = snapshot.getContext("2d");
  let index = 0;
  let played = 0;
  let previous: StickerFrame | null = null;
  let timer = 0;
  let stopped = false;
  const finish = (): void => {
    if (stopped) {
      return;
    }
    stopped = true;
    window.clearTimeout(timer);
    movie.frames.forEach((frame) => frame.image.close());
  };
  const draw = (): void => {
    if (!canvas.isConnected) {
      finish();
      return;
    }
    const frame = movie.frames[index];
    if (!frame) {
      return;
    }
    if (previous?.dispose === 1) {
      context.clearRect(previous.x, previous.y, previous.width, previous.height);
    } else if (previous?.dispose === 2 && saved) {
      context.clearRect(0, 0, movie.width, movie.height);
      context.drawImage(snapshot, 0, 0);
    }
    if (frame.dispose === 2 && saved) {
      saved.clearRect(0, 0, movie.width, movie.height);
      saved.drawImage(canvas, 0, 0);
    }
    if (frame.blend === 0) {
      context.clearRect(frame.x, frame.y, frame.width, frame.height);
    }
    context.drawImage(frame.image, frame.x, frame.y);
    previous = frame;
    const last = index === movie.frames.length - 1;
    if (last) {
      played += 1;
      if (movie.plays > 0 && played >= movie.plays) {
        finish();
        return;
      }
    }
    index = (index + 1) % movie.frames.length;
    timer = window.setTimeout(draw, frame.delay);
  };
  draw();
  const stop = new MutationObserver(() => {
    if (!canvas.isConnected) {
      finish();
      stop.disconnect();
    }
  });
  stop.observe(canvas.parentElement || document.body, { childList: true });
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
  more.textContent = t("moreLabel");
  more.addEventListener("click", () => {
    body.classList.remove("clamped");
    more.remove();
  });
  body.after(more);
}

const REACTIONS: { emoji: string; label: MessageKey }[] = [
  { emoji: "👍", label: "like" },
  { emoji: "😎", label: "cool" },
  { emoji: "😍", label: "love" },
  { emoji: "😠", label: "angry" },
  { emoji: "🥺", label: "please" },
  { emoji: "😭", label: "cry" },
  { emoji: "😮", label: "wow" },
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
  chip.setAttribute("aria-label", visible && reaction.mine ? t("removeReaction") : t("reaction"));
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
    choice.setAttribute("aria-label", t(reaction.label));
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
  toggle.setAttribute("aria-label", t("addReaction"));
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

function quoteFromHistory(message: HistoryMessage): string {
  if (message.kind === "unsent" || message.kind === "chat update") {
    return t("message");
  }
  const text = message.text.trim();
  if (text && !mediaLabel(text)) {
    return text;
  }
  if (message.sticker) {
    return t("sticker");
  }
  if (message.kind === "video") {
    return t("video");
  }
  if (message.kind === "image") {
    return t("photo");
  }
  return text || t("message");
}

function hydrateReplies(messages: HistoryMessage[]): HistoryMessage[] {
  const byId = new Map<string, HistoryMessage>();
  for (const message of messages) {
    if (message.id) {
      byId.set(message.id, message);
    }
  }
  return messages.map((message) => {
    if (!message.replyId) {
      return message;
    }
    const source = byId.get(message.replyId);
    return {
      ...message,
      replyText: message.replyText || (source ? quoteFromHistory(source) : t("message")),
      replyName: message.replyName || (source ? (source.mine ? (myName || t("you")) : (source.name || t("message"))) : ""),
      replyPicture: message.replyPicture || (source ? quotedPicture(source) : ""),
    };
  });
}

function appendReplyQuote(bubble: HTMLElement, message: BubbleMessage): void {
  if (!message.replyId || message.kind === "unsent" || message.kind === "chat update") {
    return;
  }
  const quote = document.createElement("div");
  quote.className = "reply-quote";
  const who = document.createElement("div");
  who.className = "reply-who";
  who.textContent = message.replyName || t("message");
  const body = document.createElement("div");
  body.className = "reply-body";
  body.textContent = message.replyText || t("message");
  const copy = document.createElement("div");
  copy.append(who, body);
  const face = document.createElement("span");
  face.className = "reply-avatar";
  paintSmallAvatar(face, message.replyPicture || "", message.replyName || "");
  quote.append(face, copy);
  const name = bubble.querySelector(".who");
  if (name) {
    name.after(quote);
  } else {
    bubble.prepend(quote);
  }
}

function quoteText(row: HTMLElement): string {
  const text = row.querySelector(".msg-text")?.textContent?.trim() || "";
  if (text && !mediaLabel(text)) {
    return text;
  }
  if (row.querySelector(".sticker")) {
    return t("sticker");
  }
  if (row.querySelector("video")) {
    return t("video");
  }
  if (row.querySelector(".media-grid")) {
    return t("photo");
  }
  return text || t("message");
}

function copyableText(row: HTMLElement): string {
  const text = row.querySelector(".msg-text")?.textContent?.trim() || "";
  if (!text || mediaLabel(text)) {
    return "";
  }
  return text;
}

function senderLabel(row: HTMLElement): string {
  if (row.classList.contains("mine")) {
    return myName || t("you");
  }
  return row.querySelector(".who")?.textContent?.trim() || rows.find((item) => item.id === openId)?.name || t("message");
}

function unsentLine(mine: boolean, name: string): string {
  if (mine) {
    return t("unsentYou");
  }
  const who = name.trim();
  return who ? t("unsentOther", { name: who }) : t("unsentUnknown");
}

function quotedPicture(source: HistoryMessage): string {
  if (source.mine) {
    return myPicture;
  }
  if (source.picture) {
    return source.picture;
  }
  const chat = rows.find((row) => row.id === openId);
  if (!chat || chat.kind === "group" || chat.kind === "room") {
    return "";
  }
  return chat.picture;
}

function rowPicture(row: HTMLElement): string {
  if (row.classList.contains("mine")) {
    return myPicture;
  }
  return row.querySelector<HTMLImageElement>("button.msg-avatar img")?.currentSrc
    || row.querySelector<HTMLImageElement>("button.msg-avatar img")?.src
    || "";
}

function paintSmallAvatar(host: HTMLElement, picture: string, name: string): void {
  host.replaceChildren();
  if (!picture) {
    host.append(fallback(name));
    return;
  }
  const photo = document.createElement("img");
  photo.alt = "";
  photo.src = picture;
  photo.addEventListener("error", () => {
    host.replaceChildren(fallback(name));
  });
  host.append(photo);
}

function paintReplyBar(): void {
  if (!replyDraft) {
    replyBar.hidden = true;
    replyAvatar.replaceChildren();
    replyLabel.textContent = "";
    replyText.textContent = "";
    return;
  }
  paintSmallAvatar(replyAvatar, replyDraft.picture, replyDraft.name);
  replyLabel.textContent = replyDraft.name;
  replyText.textContent = replyDraft.text;
  replyBar.hidden = false;
  composerInput.focus();
}

function clearReply(): void {
  replyDraft = null;
  paintReplyBar();
}

function closeMsgMenu(): void {
  msgMenu.hidden = true;
  msgMenuRow = null;
}

function openMsgMenu(event: MouseEvent, row: HTMLElement): void {
  event.preventDefault();
  event.stopPropagation();
  closeMenu();
  closeReactionPicker();
  menuOpenedAt = Date.now();
  msgMenuRow = row;
  const mine = row.classList.contains("mine");
  const pending = !messageIdNumber(row.dataset.id || "");
  msgMenu.querySelectorAll<HTMLElement>("[data-for]").forEach((item) => {
    item.hidden = item.dataset.for === "mine" ? !mine : mine;
  });
  if (pending) {
    msgMenu.querySelectorAll<HTMLElement>("[data-action='reply'], [data-action='unsend'], [data-action='report']").forEach((item) => {
      item.hidden = true;
    });
  }
  msgMenu.hidden = false;
  const width = msgMenu.offsetWidth;
  const height = msgMenu.offsetHeight;
  let left = event.clientX;
  let top = event.clientY;
  if (left + width > window.innerWidth - 8) {
    left = window.innerWidth - width - 8;
  }
  if (top + height > window.innerHeight - 8) {
    top = Math.max(8, event.clientY - height);
  }
  msgMenu.style.left = `${Math.max(8, left)}px`;
  msgMenu.style.top = `${Math.max(8, top)}px`;
}

function paintUnsent(row: HTMLElement, mine: boolean, name: string): void {
  const previous = copyableText(row) || quoteText(row);
  const sentence = unsentLine(mine, name);
  const note = document.createElement("div");
  note.className = "chat-update";
  note.dataset.unsent = "1";
  if (row.dataset.id) {
    note.dataset.id = row.dataset.id;
  }
  note.textContent = sentence;
  const chatId = openId;
  row.replaceWith(note);
  const chat = rows.find((item) => item.id === chatId);
  if (chat && previous && chat.lastMessage === previous) {
    chat.lastMessage = sentence;
    chat.sticker = "";
    chat.stickerFallback = "";
    renderList();
  }
  if (replyDraft && note.dataset.id && replyDraft.id === note.dataset.id) {
    clearReply();
  }
}

function messageRow(messageId: string): HTMLElement | null {
  const rowsInThread = threadBody.querySelectorAll<HTMLElement>(".msg[data-id]");
  for (const row of rowsInThread) {
    if (row.dataset.id === messageId) {
      return row;
    }
    const ids = (row.dataset.ids || "").split(",");
    if (ids.length === 1 && ids[0] === messageId) {
      return row;
    }
  }
  return null;
}

async function copyMessage(row: HTMLElement): Promise<void> {
  const text = copyableText(row);
  if (!text) {
    sendError.hidden = false;
    sendError.textContent = t("nothingToCopy");
    return;
  }
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const area = document.createElement("textarea");
    area.value = text;
    area.setAttribute("readonly", "");
    document.body.append(area);
    area.select();
    const copied = document.execCommand("copy");
    area.remove();
    if (!copied) {
      sendError.hidden = false;
      sendError.textContent = t("couldNotCopy");
      return;
    }
  }
  sendError.hidden = true;
}

const REPORT_REASONS: { id: number; label: MessageKey }[] = [
  { id: 1, label: "spam" },
  { id: 2, label: "sexualHarassment" },
  { id: 3, label: "harassment" },
  { id: 0, label: "other" },
];

function messageSender(row: HTMLElement): string {
  const stored = row.dataset.sender || "";
  if (stored.startsWith("u")) {
    return stored;
  }
  return openId.startsWith("u") ? openId : "";
}

function showReport(row: HTMLElement): void {
  const messageId = row.dataset.id || "";
  const sender = messageSender(row);
  const text = copyableText(row);
  if (!openId || !messageId || !sender) {
    sendError.hidden = false;
    sendError.textContent = t("cannotReport");
    return;
  }
  const chatId = openId;
  sheetTitle.textContent = t("report");
  sheetBody.hidden = false;
  sheetBody.textContent = t("whyReport");
  sheetSettings.hidden = true;
  sheetAbout.hidden = true;
  sheetLanguage.hidden = true;
  sheetList.hidden = false;
  sheetList.replaceChildren();
  for (const reason of REPORT_REASONS) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = t(reason.label);
    button.addEventListener("click", () => {
      button.disabled = true;
      void invoke("report_message", { id: chatId, messageId, sender, reason: reason.id, text }).then(() => {
        sheetList.hidden = true;
        sheetBody.hidden = false;
        sheetBody.textContent = t("reportSent");
      }).catch((error: unknown) => {
        button.disabled = false;
        sheetBody.hidden = false;
        sheetBody.textContent = error instanceof Error ? error.message : String(error);
      });
    });
    sheetList.append(button);
  }
  sheet.hidden = false;
}

function showShare(text: string): void {
  shareText = text;
  sheetTitle.textContent = t("share");
  sheetBody.hidden = true;
  sheetSettings.hidden = true;
  sheetAbout.hidden = true;
  sheetLanguage.hidden = true;
  sheetList.hidden = false;
  sheetList.replaceChildren();
  const choices = rows.filter((row) => row.id && row.name);
  if (choices.length === 0) {
    sheetList.hidden = true;
    sheetBody.hidden = false;
    sheetBody.textContent = t("noChatsToShare");
  } else {
    for (const chat of choices) {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = chat.name;
      button.addEventListener("click", () => {
        sheet.hidden = true;
        startSend(chat.id, shareText, null);
      });
      sheetList.append(button);
    }
  }
  sheet.hidden = false;
}

async function unsendRow(row: HTMLElement): Promise<void> {
  const ids = (row.dataset.ids || row.dataset.id || "").split(",").map((id) => id.trim()).filter((id) => id);
  if (ids.length === 0 || !openId) {
    sendError.hidden = false;
    sendError.textContent = t("cannotUnsend");
    return;
  }
  const chatId = openId;
  const mine = row.classList.contains("mine");
  const name = senderLabel(row);
  try {
    for (const messageId of ids) {
      await invoke("unsend_message", { id: chatId, messageId });
    }
  } catch (error) {
    sendError.hidden = false;
    sendError.textContent = error instanceof Error ? error.message : String(error);
    return;
  }
  sendError.hidden = true;
  if (row.isConnected) {
    paintUnsent(row, mine, name);
  }
}

function messageBubble(message: BubbleMessage): HTMLElement {
  if (!message.media?.length && (message.kind === "image" || message.kind === "video") && message.mediaId) {
    message = {
      ...message,
      media: [{ id: message.mediaId, seq: message.seq || 0, animated: Boolean(message.animated) }],
    };
  }
  if (message.kind === "chat update" || message.kind === "unsent") {
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
      appendMessageText(bubble, message.text || t("sticker"));
    }, message.id || ""));
  } else {
    appendMessageText(bubble, message.text);
  }
  appendReplyQuote(bubble, message);
  const row = document.createElement("div");
  row.className = message.mine ? "msg mine" : "msg";
  const ids = [message.id || "", ...(message.media?.map((slot) => slot.id) || [])].filter((id) => id);
  if (message.id) {
    row.dataset.id = message.id;
  }
  if (message.gid) {
    row.dataset.gid = message.gid;
  }
  if (message.sender) {
    row.dataset.sender = message.sender;
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
  const sending = message.mine && message.status === "sending";
  const failed = message.mine && message.status === "failed";
  const read = message.mine && message.status === "read";
  if (!time && !read && !sending && !failed) {
    return null;
  }
  const meta = document.createElement("div");
  meta.className = "msg-meta";
  if (failed) {
    const retry = document.createElement("button");
    retry.type = "button";
    retry.className = "msg-retry";
    retry.textContent = t("notSent");
    meta.append(retry);
  } else if (sending) {
    const label = document.createElement("span");
    label.className = "msg-state";
    label.textContent = t("sending");
    meta.append(label);
  }
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
  mark.textContent = readers > 1 ? t("readCount", { count: readers }) : t("read");
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
  button.setAttribute("aria-label", name ? t("profileOf", { name }) : t("profile"));
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

function playableVideo(src = ""): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "video-play";
  const video = document.createElement("video");
  video.className = "chat-video";
  video.controls = true;
  video.playsInline = true;
  video.preload = "metadata";
  if (src) {
    video.src = src;
  }
  const play = document.createElement("button");
  play.type = "button";
  play.className = "video-play-button";
  play.setAttribute("aria-label", t("video"));
  play.textContent = "▶";
  play.addEventListener("click", (event) => {
    event.stopPropagation();
    play.hidden = true;
    void video.play();
  });
  video.addEventListener("play", () => {
    play.hidden = true;
  });
  video.addEventListener("ended", () => {
    play.hidden = false;
  });
  wrap.append(video, play);
  return wrap;
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
        button.textContent = t("image");
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
    const wrap = playableVideo();
    const video = wrap.querySelector("video");
    stack.append(wrap);
    void warmMedia([item.id], false).then(() => {
      if (!video || token !== historyToken) {
        return;
      }
      const key = mediaKey(item.id, false);
      const url = mediaUrls.get(key);
      const mime = mediaMimes.get(key) || "";
      if (url && (mime.startsWith("video/") || mime === "")) {
        video.src = url;
        return;
      }
      const note = document.createElement("div");
      note.textContent = t("video");
      wrap.replaceWith(note);
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
    name.textContent = displayName(chat.name);
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
  return trimmed && !updateLabel(trimmed) ? trimmed : t("noMessages");
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
  return t("callTime", { time: `${pad(minutes)}:${pad(seconds)}` });
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
  const kept = rows.find((row) => row.id === keepId);
  rows = payload.chats;
  if (kept && !rows.some((row) => row.id === kept.id)) {
    rows.unshift(kept);
  }
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
  setStatus("loadingChats");
  try {
    await invoke("load_chats");
    return true;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    if (message.includes("No LINE session")) {
      return false;
    }
    setStatusText(message);
    return true;
  }
}

async function start(): Promise<void> {
  refresh.disabled = true;
  const internals = (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  if (internals) {
    setStatus("loadingChats");
  } else {
    setStatus("bridgeNotReady");
  }
  if (!internals) {
    window.setTimeout(() => void start(), 200);
    return;
  }
  if (await showChats()) {
    return;
  }
  refresh.hidden = false;
  setStatus("openingQr");
  try {
    await invoke("start_login");
  } catch (error) {
    setStatusText(error instanceof Error ? error.message : String(error));
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

threadBody.addEventListener("click", (event) => {
  const retry = (event.target as HTMLElement).closest<HTMLButtonElement>(".msg-retry");
  if (!retry) {
    return;
  }
  event.preventDefault();
  const localId = retry.closest<HTMLElement>(".msg")?.dataset.localId || "";
  if (localId) {
    retryOutgoing(localId);
  }
});

threadBody.addEventListener("contextmenu", (event) => {
  const row = (event.target as HTMLElement).closest<HTMLElement>(".msg");
  if (!row?.dataset.id) {
    return;
  }
  openMsgMenu(event, row);
});

threadBody.addEventListener("scroll", () => {
  closeMsgMenu();
});

msgMenu.addEventListener("click", (event) => {
  event.stopPropagation();
  const action = (event.target as HTMLElement).closest("button")?.dataset.action;
  const row = msgMenuRow;
  closeMsgMenu();
  if (!action || !row?.isConnected) {
    return;
  }
  if (action === "reply") {
    if (!messageIdNumber(row.dataset.id || "")) {
      return;
    }
    replyDraft = { id: row.dataset.id || "", text: quoteText(row), name: senderLabel(row), picture: rowPicture(row) };
    if (!replyDraft.id) {
      replyDraft = null;
      return;
    }
    paintReplyBar();
  } else if (action === "copy") {
    void copyMessage(row);
  } else if (action === "share") {
    const text = copyableText(row);
    if (!text) {
      sendError.hidden = false;
      sendError.textContent = t("nothingToShare");
      return;
    }
    showShare(text);
  } else if (action === "unsend") {
    void unsendRow(row);
  } else if (action === "report") {
    showReport(row);
  }
});

document.querySelector<HTMLButtonElement>("#reply-cancel")!.addEventListener("click", () => {
  clearReply();
});

await listen<LoginEvent>("login-event", (event) => {
  void onEvent(event.payload);
});

(window as unknown as { __lineRender?: (payload: ChatPayload) => void }).__lineRender = (payload) => {
  try {
    renderChats(payload);
  } catch (error) {
    setStatusText(error instanceof Error ? error.message : String(error));
  }
};

(window as unknown as { __lineHistory?: (payload: HistoryPayload) => void }).__lineHistory = (payload) => {
  try {
    renderHistory(payload);
  } catch (error) {
    setStatusText(error instanceof Error ? error.message : String(error));
  }
};

function sendFailureNote(payload: Pick<SendResult, "code" | "error">): string {
  return payload.code === 82
    ? t("letterSealingBlocked")
    : (payload.error || t("couldNotSend"));
}

function bumpChat(chatId: string, text: string, time: number, sticker = "", fallback = ""): void {
  const chat = rows.find((row) => row.id === chatId);
  if (!chat || !text) {
    return;
  }
  chat.lastMessage = text;
  chat.time = time;
  chat.sticker = sticker;
  chat.stickerFallback = fallback;
  chat.sealed = false;
  rows.sort((left, right) => right.time - left.time);
  const stay = visibleRows().findIndex((row) => row.id === openId);
  if (stay >= 0) {
    selected = stay;
  }
  renderList();
}

function outgoingBubble(item: Outgoing): HTMLElement {
  if (item.kind === "sticker" && item.sticker?.includes("/sticker_animation.png") && !playedStickers.has(item.localId)) {
    armedStickers.add(item.localId);
  }
  const row = messageBubble({
    id: item.localId,
    mine: true,
    name: "",
    text: item.previewUrl ? "" : item.text,
    sealed: false,
    status: item.state,
    time: item.time,
    sticker: item.sticker,
    stickerFallback: item.stickerFallback,
    replyId: item.reply?.id || "",
    replyText: item.reply?.text || "",
    replyName: item.reply?.name || "",
    replyPicture: item.reply?.picture || "",
  });
  const bubble = row.querySelector(".bubble");
  if (item.albumUrls && item.albumUrls.length > 1) {
    bubble?.classList.add("rich");
    bubble?.querySelector(".msg-text")?.remove();
    const grid = document.createElement("div");
    grid.className = "media-grid many";
    for (const url of item.albumUrls) {
      const thumb = document.createElement("span");
      thumb.className = "media-thumb";
      const img = document.createElement("img");
      img.alt = "";
      img.src = url;
      thumb.append(img);
      grid.append(thumb);
    }
    bubble?.prepend(grid);
    if (item.file?.groupId) {
      row.dataset.gid = item.file.groupId;
    }
  } else if (item.previewUrl) {
    const media = item.kind === "video" ? playableVideo(item.previewUrl) : document.createElement("img");
    if (media instanceof HTMLImageElement) {
      media.className = "outgoing-media";
      media.src = item.previewUrl;
    }
    const caption = item.kind === "image" || item.kind === "video" ? "" : item.text;
    if (caption) {
      const text = document.createElement("div");
      text.textContent = caption;
      bubble?.prepend(text);
    }
    bubble?.prepend(media);
    bubble?.querySelectorAll(":scope > div").forEach((node) => {
      if (!node.className && !node.textContent) {
        node.remove();
      }
    });
  }
  row.dataset.localId = item.localId;
  row.dataset.state = item.state;
  const retry = row.querySelector<HTMLButtonElement>(".msg-retry");
  if (retry && item.error) {
    retry.title = item.error;
  }
  return row;
}

function setDelivery(row: HTMLElement, state: "sending" | "failed" | "sent", time: number, serverId = "", error = ""): void {
  if (serverId) {
    transferStickerPlay(row.dataset.localId || "", serverId);
    row.dataset.id = serverId;
    row.dataset.ids = serverId;
    delete row.dataset.localId;
    delete row.dataset.state;
  } else {
    row.dataset.state = state;
    row.dataset.localId = row.dataset.localId || "";
  }
  const next = messageMeta({
    mine: true,
    name: "",
    text: "",
    sealed: false,
    time,
    status: state,
  });
  const retry = next?.querySelector<HTMLButtonElement>(".msg-retry");
  if (retry && error) {
    retry.title = error;
  }
  const current = row.querySelector(".msg-meta");
  if (!next) {
    current?.remove();
    return;
  }
  if (current) {
    current.replaceWith(next);
  } else {
    const bubble = row.querySelector(".bubble");
    if (bubble) {
      row.insertBefore(next, bubble);
    }
  }
}

function placeOutgoing(chatId: string): boolean {
  if (chatId !== openId) {
    return false;
  }
  let placed = false;
  for (const item of outgoing) {
    if (item.chatId !== chatId || messageRow(item.localId) || (item.file?.groupSeq || 0) > 1 || albumHost(item.file?.groupId || "")) {
      continue;
    }
    threadBody.querySelector(".thread-note")?.remove();
    threadBody.append(outgoingBubble(item));
    placed = true;
  }
  return placed;
}

function takeMatchingOutgoing(chatId: string, message: HistoryMessage): Outgoing | null {
  if (!message.mine || !message.id || !messageIdNumber(message.id)) {
    return null;
  }
  const index = outgoing.findIndex((item) =>
    item.chatId === chatId
    && item.state === "sending"
    && item.knownIds !== ""
    && (item.text === message.text || (Boolean(item.kind) && item.kind === (message.kind || "")))
    && (item.reply?.id || "") === (message.replyId || "")
    && !item.knownIds.split("\u001f").includes(message.id || "")
  );
  if (index < 0) {
    return null;
  }
  const [item] = outgoing.splice(index, 1);
  return item;
}

function claimOutgoing(chatId: string, messages: HistoryMessage[]): void {
  for (const message of messages) {
    const claimed = takeMatchingOutgoing(chatId, message);
    if (!claimed) {
      continue;
    }
    if (message.id) {
      transferStickerPlay(claimed.localId, message.id);
    }
    const row = messageRow(claimed.localId);
    if (row && message.id && row !== messageRow(message.id)) {
      row.remove();
    }
  }
}

function finishOutgoing(localId: string, payload: SendResult): void {
  const index = outgoing.findIndex((item) => item.localId === localId);
  const item = index >= 0 ? outgoing[index] : undefined;
  if (!payload.ok) {
    const note = sendFailureNote(payload);
    if (item) {
      item.state = "failed";
      item.error = note;
    }
    const row = localId ? messageRow(localId) : null;
    const host = row || albumHost(item?.file?.groupId || "");
    if (host) {
      setDelivery(host, "failed", item?.time || Date.now(), "", note);
    }
    return;
  }
  if (index >= 0) {
    outgoing.splice(index, 1);
  }
  const sticker = payload.sticker || item?.sticker || "";
  const stickerFallback = payload.stickerFallback || item?.stickerFallback || "";
  if (payload.text) {
    bumpChat(payload.id, payload.text, payload.time || item?.time || Date.now(), sticker, stickerFallback);
  }
  rememberSent(payload, item, sticker, stickerFallback);
  if (payload.id !== openId || !payload.text) {
    return;
  }
  const row = localId ? messageRow(localId) : null;
  const serverRow = payload.messageId ? messageRow(payload.messageId) : null;
  if (row && serverRow && row !== serverRow) {
    row.remove();
    return;
  }
  if (row && payload.messageId) {
    setDelivery(row, "sent", payload.time || item?.time || Date.now(), payload.messageId);
    return;
  }
  if (serverRow || albumHost(payload.gid || item?.file?.groupId || "")) {
    if (payload.gid) {
      showAlbum(payload.gid, payload.kind || item?.kind || "");
    }
    return;
  }
  if (payload.gid && showAlbum(payload.gid, payload.kind || item?.kind || "")) {
    return;
  }
  threadBody.querySelector(".thread-note")?.remove();
  threadBody.append(messageBubble({
    id: payload.messageId,
    mine: true,
    name: "",
    text: payload.text,
    sealed: false,
    status: "sent",
    time: payload.time || Date.now(),
    sticker,
    stickerFallback,
    kind: payload.kind || item?.kind || "",
    mediaId: payload.mediaId || payload.messageId || "",
    gid: payload.gid || "",
    seq: payload.seq || 0,
    replyId: item?.reply?.id || payload.replyId || "",
    replyText: item?.reply?.text || "",
    replyName: item?.reply?.name || "",
    replyPicture: item?.reply?.picture || "",
  }));
  unreadFocus = "";
  scrollThreadToEnd();
}

function rememberSent(payload: SendResult, item: Outgoing | undefined, sticker: string, stickerFallback: string): void {
  if (!openHistory || openHistory.id !== payload.id || !payload.messageId || !payload.text) {
    return;
  }
  const message: HistoryMessage = {
    id: payload.messageId,
    mine: true,
    name: "",
    text: payload.text,
    sealed: false,
    status: "sent",
    time: payload.time || item?.time || Date.now(),
    sticker,
    stickerFallback,
    kind: payload.kind || item?.kind || "",
    mediaId: payload.mediaId || "",
    animated: Boolean(payload.animated),
    gid: payload.gid || "",
    seq: payload.seq || 0,
    replyId: item?.reply?.id || payload.replyId || "",
    replyText: item?.reply?.text || "",
    replyName: item?.reply?.name || "",
    replyPicture: item?.reply?.picture || "",
  };
  const existing = openHistory.messages.findIndex((entry) => entry.id === payload.messageId);
  if (existing >= 0) {
    const prior = openHistory.messages[existing];
    openHistory.messages[existing] = {
      ...prior,
      ...message,
      sticker: message.sticker || prior.sticker,
      stickerFallback: message.stickerFallback || prior.stickerFallback,
      kind: message.kind || prior.kind,
      mediaId: message.mediaId || prior.mediaId,
      gid: message.gid || prior.gid,
      seq: message.seq || prior.seq,
    };
    return;
  }
  openHistory.messages.push(message);
  openHistory.lastMessageId = payload.messageId;
}

function dispatchSend(item: Outgoing): Promise<void> {
  const fail = (error: unknown) => {
    finishOutgoing(item.localId, {
      id: item.chatId,
      ok: false,
      error: error instanceof Error ? error.message : String(error),
    });
  };
  if (item.file) {
    return invoke("send_media", {
      id: item.chatId,
      name: item.file.name,
      mime: item.file.mime,
      data: item.file.data,
      localId: item.localId,
      groupId: item.file.groupId,
      groupSeq: item.file.groupSeq,
      groupCount: item.file.groupCount,
    }).then(() => undefined, fail);
  }
  if (item.stickerSend) {
    return invoke("send_sticker", {
      id: item.chatId,
      packageId: item.stickerSend.packageId,
      stickerId: item.stickerSend.stickerId,
      animated: item.stickerSend.animated,
      localId: item.localId,
    }).then(() => undefined, fail);
  }
  return invoke("send_message", {
    id: item.chatId,
    text: item.text,
    relatedId: item.reply?.id || undefined,
    localId: item.localId,
  }).then(() => undefined, fail);
}

function startSend(chatId: string, text: string, reply: ReplyDraft | null): void {
  const body = text.trim();
  if (!body || !chatId) {
    return;
  }
  const item: Outgoing = {
    localId: `local-${Date.now()}-${outgoingSeq += 1}`,
    chatId,
    text: body,
    reply,
    time: Date.now(),
    state: "sending",
    error: "",
    knownIds: chatId === openId ? paintedIds : "",
  };
  outgoing.push(item);
  bumpChat(chatId, body, item.time);
  if (chatId === openId) {
    threadBody.querySelector(".thread-note")?.remove();
    threadBody.append(outgoingBubble(item));
    unreadFocus = "";
    scrollThreadToEnd();
  }
  dispatchSend(item);
}

function retryOutgoing(localId: string): void {
  const item = outgoing.find((entry) => entry.localId === localId);
  if (!item || item.state !== "failed") {
    return;
  }
  item.state = "sending";
  item.error = "";
  item.knownIds = item.chatId === openId ? paintedIds : item.knownIds;
  const row = messageRow(localId);
  if (row) {
    setDelivery(row, "sending", item.time);
  }
  dispatchSend(item);
}

function onSent(payload: SendResult): void {
  composerInput.disabled = !openId;
  attachFile.disabled = !openId;
  stickerOpen.disabled = !openId;
  refreshSendButton();
  finishOutgoing(payload.localId || "", payload);
}

(window as unknown as { __linePoll?: (payload: PollPayload) => void }).__linePoll = (payload) => {
  try {
    applyPoll(payload);
  } catch (error) {
    setStatusText(error instanceof Error ? error.message : String(error));
  }
};

(window as unknown as { __lineSent?: (payload: SendResult) => void }).__lineSent = (payload) => {
  try {
    onSent(payload);
  } catch (error) {
    setStatusText(error instanceof Error ? error.message : String(error));
  }
};

const MEDIA_LIMIT = 40_000_000;
const STICKER_PACK: { packageId: string; stickerId: string; animated: boolean }[] = [
  ...["1988", "1989", "1990", "1991", "1992"].map((stickerId) => ({ packageId: "446", stickerId, animated: false })),
  ...Array.from({ length: 11 }, (_, index) => ({ packageId: "11537", stickerId: String(52002734 + index), animated: true })),
];

function closeStickerPanel(): void {
  stickerPanel.hidden = true;
  stickerOpen.setAttribute("aria-expanded", "false");
}

function paintStickerPanel(): void {
  if (stickerPanel.childElementCount > 0) {
    return;
  }
  for (const sticker of STICKER_PACK) {
    const button = document.createElement("button");
    button.type = "button";
    const asset = sticker.animated ? "sticker_animation.png" : "sticker.png";
    const img = document.createElement("img");
    img.alt = "";
    img.src = `https://stickershop.line-scdn.net/stickershop/v1/sticker/${sticker.stickerId}/android/${asset}`;
    button.append(img);
    button.addEventListener("click", () => {
      if (!openId || composerInput.disabled) {
        return;
      }
      closeStickerPanel();
      startSticker(openId, sticker);
    });
    stickerPanel.append(button);
  }
}

function fileToBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = String(reader.result || "");
      const comma = result.indexOf(",");
      resolve(comma >= 0 ? result.slice(comma + 1) : result);
    };
    reader.onerror = () => reject(reader.error || new Error("could not read that file"));
    reader.readAsDataURL(file);
  });
}

function albumHost(gid: string): HTMLElement | null {
  if (!gid) {
    return null;
  }
  return threadBody.querySelector<HTMLElement>(`.msg[data-gid="${CSS.escape(gid)}"]`);
}

function queueOutgoing(item: Outgoing, visible = true): Promise<void> {
  outgoing.push(item);
  bumpChat(item.chatId, item.text, item.time, item.sticker || "", item.stickerFallback || "");
  if (visible && item.chatId === openId) {
    threadBody.querySelector(".thread-note")?.remove();
    threadBody.append(outgoingBubble(item));
    unreadFocus = "";
    scrollThreadToEnd();
  }
  return dispatchSend(item);
}

function startSticker(chatId: string, sticker: { packageId: string; stickerId: string; animated: boolean }): void {
  const asset = sticker.animated ? "sticker_animation.png" : "sticker.png";
  const item: Outgoing = {
    localId: `local-${Date.now()}-${outgoingSeq += 1}`,
    chatId,
    text: "Sticker",
    reply: null,
    time: Date.now(),
    state: "sending",
    error: "",
    knownIds: chatId === openId ? paintedIds : "",
    kind: "sticker",
    sticker: `https://stickershop.line-scdn.net/stickershop/v1/sticker/${sticker.stickerId}/android/${asset}`,
    stickerFallback: `https://stickershop.line-scdn.net/stickershop/v1/product/${sticker.packageId}/android/stickers/${sticker.stickerId}.png`,
    stickerSend: sticker,
  };
  queueOutgoing(item);
}

function mediaKind(file: File): AttachmentDraft["kind"] {
  const mime = file.type || "";
  const ext = file.name.split(".").pop()?.toLowerCase() || "";
  const imageExt = ["jpg", "jpeg", "png", "gif", "webp", "heic", "bmp"].includes(ext);
  const videoExt = ["mp4", "mov", "m4v", "webm"].includes(ext);
  const audioExt = ["m4a", "aac", "mp3", "wav"].includes(ext);
  if (mime.startsWith("image/") || imageExt) {
    return "image";
  }
  if (mime.startsWith("audio/") || audioExt) {
    return "audio";
  }
  if (mime.startsWith("video/") || videoExt) {
    return "video";
  }
  return "file";
}

function refreshSendButton(): void {
  const hasText = composerInput.value.trim().length > 0;
  const hasFile = attachmentDrafts.length > 0;
  sendButton.disabled = sendBusy || composerInput.disabled || (!hasText && !hasFile);
}

function paintAttachment(): void {
  attachPreview.replaceChildren();
  if (attachmentDrafts.length === 0) {
    attachPreview.hidden = true;
    refreshSendButton();
    return;
  }
  attachPreview.hidden = false;
  attachmentDrafts.forEach((draft, index) => {
    const item = document.createElement("span");
    item.className = "attach-item";
    if ((draft.kind === "image" || draft.kind === "video") && draft.previewUrl) {
      const thumb = document.createElement(draft.kind === "video" ? "video" : "img");
      thumb.className = "attach-thumb";
      thumb.src = draft.previewUrl;
      if (thumb instanceof HTMLImageElement) {
        thumb.alt = draft.name;
      } else {
        thumb.muted = true;
      }
      item.append(thumb);
    } else {
      const name = document.createElement("span");
      name.className = "attach-name";
      name.textContent = draft.name;
      item.append(name);
    }
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "attach-remove";
    remove.setAttribute("aria-label", t("removeAttachment"));
    remove.textContent = "×";
    remove.addEventListener("click", () => {
      removeAttachment(index);
      composerInput.focus();
    });
    item.append(remove);
    attachPreview.append(item);
  });
  refreshSendButton();
}

function discardAttachment(): void {
  for (const draft of attachmentDrafts) {
    if (draft.previewUrl) {
      URL.revokeObjectURL(draft.previewUrl);
    }
  }
  attachmentDrafts = [];
  attachmentEpoch += 1;
  paintAttachment();
}

function removeAttachment(index: number): void {
  const draft = attachmentDrafts[index];
  if (!draft) {
    return;
  }
  if (draft.previewUrl) {
    URL.revokeObjectURL(draft.previewUrl);
  }
  attachmentDrafts.splice(index, 1);
  attachmentEpoch += 1;
  paintAttachment();
}

function stageFiles(chatId: string, files: File[]): void {
  const picked = files.filter((file) => file && file.size > 0);
  if (!picked.length || !chatId) {
    return;
  }
  const images = picked.filter((file) => mediaKind(file) === "image");
  const batch = images.length > 0 && (images.length === picked.length || picked.length > 1) ? images : [picked[0]];
  const imageBatch = batch.every((file) => mediaKind(file) === "image");
  let tooLarge = false;
  const fitting = batch.filter((file) => {
    if (file.size <= MEDIA_LIMIT) {
      return true;
    }
    tooLarge = true;
    return false;
  });
  if (!fitting.length) {
    sendError.hidden = false;
    sendError.textContent = t("fileTooLarge");
    return;
  }
  const prior = imageBatch && attachmentDrafts.every((draft) => draft.kind === "image") ? attachmentDrafts : [];
  if (prior.length === 0) {
    for (const draft of attachmentDrafts) {
      if (draft.previewUrl) {
        URL.revokeObjectURL(draft.previewUrl);
      }
    }
  }
  const room = imageBatch ? IMAGE_CAP - prior.length : 1;
  const taken = fitting.slice(0, Math.max(room, 0));
  if (!taken.length) {
    sendError.hidden = false;
    sendError.textContent = t("tooManyImages", { count: IMAGE_CAP });
    return;
  }
  const added = taken.map((file) => {
    const kind = mediaKind(file);
    return {
      chatId,
      file,
      name: file.name,
      mime: file.type || "application/octet-stream",
      kind,
      previewUrl: kind === "image" || kind === "video" ? URL.createObjectURL(file) : "",
    };
  });
  attachmentDrafts = imageBatch ? [...prior, ...added] : added;
  attachmentEpoch += 1;
  const overflow = fitting.length > taken.length;
  sendError.hidden = !(tooLarge || overflow);
  sendError.textContent = tooLarge ? t("fileTooLarge") : overflow ? t("tooManyImages", { count: IMAGE_CAP }) : "";
  paintAttachment();
  composerInput.focus();
}

function stageAttachment(chatId: string, file: File): void {
  stageFiles(chatId, [file]);
}

async function submitComposer(): Promise<void> {
  if (sendBusy || !openId || composerInput.disabled) {
    return;
  }
  const text = composerInput.value.trim();
  const drafts = attachmentDrafts.slice();
  const epoch = attachmentEpoch;
  if ((!text && drafts.length === 0) || drafts.some((draft) => draft.chatId !== openId)) {
    return;
  }
  const chatId = openId;
  const reply = replyDraft;
  sendBusy = true;
  refreshSendButton();
  const encoded: string[] = [];
  if (drafts.length > 0) {
    try {
      for (const draft of drafts) {
        encoded.push(await fileToBase64(draft.file));
        if (attachmentEpoch !== epoch) {
          sendBusy = false;
          refreshSendButton();
          return;
        }
      }
    } catch (error: unknown) {
      sendBusy = false;
      refreshSendButton();
      sendError.hidden = false;
      sendError.textContent = error instanceof Error ? error.message : String(error);
      return;
    }
  }
  replyDraft = null;
  paintReplyBar();
  composerInput.value = "";
  if (drafts.length > 0) {
    attachmentDrafts = [];
    attachmentEpoch += 1;
    paintAttachment();
  }
  sendBusy = false;
  sendError.hidden = true;
  refreshSendButton();
  const album = drafts.length > 1 && drafts.every((draft) => draft.kind === "image")
    ? `${Date.now()}${Math.floor(Math.random() * 100000).toString().padStart(5, "0")}`
    : "";
  const albumUrls = album ? drafts.map((draft) => draft.previewUrl) : [];
  for (let index = 0; index < drafts.length; index += 1) {
    const draft = drafts[index];
    const preview = draft.kind === "image" || draft.kind === "video";
    await queueOutgoing({
      localId: `local-${Date.now()}-${outgoingSeq += 1}`,
      chatId,
      text: preview ? (draft.kind === "image" ? "Image" : "Video") : draft.name,
      reply: null,
      time: Date.now(),
      state: "sending",
      error: "",
      knownIds: chatId === openId ? paintedIds : "",
      kind: draft.kind,
      previewUrl: draft.previewUrl,
      albumUrls: index === 0 ? albumUrls : undefined,
      file: {
        name: draft.name,
        mime: draft.mime,
        data: encoded[index],
        groupId: album,
        groupSeq: album ? index + 1 : 0,
        groupCount: album ? drafts.length : 0,
      },
    }, !album || index === 0);
  }
  if (text) {
    startSend(chatId, text, reply);
  }
}

composerInput.addEventListener("keydown", (event) => {
  if (event.key !== "Enter" || event.shiftKey || event.isComposing) {
    return;
  }
  event.preventDefault();
  composerInput.form?.requestSubmit();
});

composerInput.addEventListener("input", () => {
  refreshSendButton();
});

document.querySelector<HTMLFormElement>("#composer")!.addEventListener("submit", (event) => {
  event.preventDefault();
  void submitComposer();
});

attachFile.addEventListener("click", () => {
  if (!openId || composerInput.disabled) {
    return;
  }
  closeStickerPanel();
  fileInput.click();
});

fileInput.addEventListener("change", () => {
  const files = Array.from(fileInput.files || []);
  fileInput.value = "";
  if (!files.length || !openId || composerInput.disabled) {
    return;
  }
  stageFiles(openId, files);
});

const composerForm = document.querySelector<HTMLFormElement>("#composer")!;

function dragHasFile(event: DragEvent): boolean {
  const types = Array.from(event.dataTransfer?.types || []).map((type) => type.toLowerCase());
  return types.includes("files") || types.includes("text/uri-list") || types.includes("text/x-moz-url") || types.includes("application/x-kde-urilist");
}

function droppedFilePath(event: DragEvent): string {
  const list = event.dataTransfer?.getData("text/uri-list") || event.dataTransfer?.getData("text/plain") || "";
  for (const line of list.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#") || !trimmed.toLowerCase().startsWith("file:")) {
      continue;
    }
    try {
      const path = decodeURIComponent(new URL(trimmed).pathname);
      if (path.startsWith("/")) {
        return path;
      }
    } catch {
      return "";
    }
  }
  return "";
}

function fileFromDroppedBytes(name: string, mime: string, data: string): File {
  const binary = atob(data);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i);
  }
  return new File([bytes], name, { type: mime });
}

async function stageDroppedPaths(paths: string[]): Promise<void> {
  const files: File[] = [];
  let failure = "";
  for (const path of paths) {
    try {
      const dropped = await invoke<{ name: string; mime: string; data: string }>("read_dropped_file", { path });
      files.push(fileFromDroppedBytes(dropped.name, dropped.mime, dropped.data));
    } catch (error: unknown) {
      const message = error instanceof Error ? error.message : String(error);
      failure = message.toLowerCase().includes("too large") ? t("fileTooLarge") : message;
    }
  }
  if (files.length > 0) {
    stageFiles(openId, files);
    return;
  }
  if (failure) {
    sendError.hidden = false;
    sendError.textContent = failure;
  }
}

async function stageDroppedFile(event: DragEvent): Promise<void> {
  const pasted = event.dataTransfer?.getData("text/plain")?.trim() || "";
  if (pasted.startsWith("file:") && composerInput.value.includes(pasted)) {
    composerInput.value = composerInput.value.replace(pasted, "");
    refreshSendButton();
  }
  const listed = Array.from(event.dataTransfer?.files || []);
  if (listed.length > 0) {
    stageFiles(openId, listed);
    return;
  }
  const path = droppedFilePath(event);
  if (!path) {
    return;
  }
  await stageDroppedPaths([path]);
}

type DragPlace = { paths?: string[]; position: { x: number; y: number } };

function dragHitsThread(position: { x: number; y: number }): boolean {
  const scale = window.devicePixelRatio || 1;
  const x = position.x / scale;
  const y = position.y / scale;
  if (x < 0 || y < 0 || x > window.innerWidth || y > window.innerHeight) {
    return false;
  }
  const target = document.elementFromPoint(x, y);
  return Boolean(target && thread.contains(target) && openId && !composerInput.disabled);
}

function dragOverThread(event: DragEvent): boolean {
  return event.target instanceof Node && thread.contains(event.target) && Boolean(openId) && !composerInput.disabled;
}

window.addEventListener("dragover", (event) => {
  event.preventDefault();
  const over = dragOverThread(event);
  composerForm.classList.toggle("dropping", over);
  if (event.dataTransfer) {
    event.dataTransfer.dropEffect = over ? "copy" : "none";
  }
}, true);

window.addEventListener("dragleave", (event) => {
  if (!event.relatedTarget) {
    composerForm.classList.remove("dropping");
  }
}, true);

window.addEventListener("drop", (event) => {
  event.preventDefault();
  composerForm.classList.remove("dropping");
  if (!dragOverThread(event)) {
    return;
  }
  const path = droppedFilePath(event);
  if (!dragHasFile(event) && !path && !event.dataTransfer?.files?.length) {
    return;
  }
  closeStickerPanel();
  void stageDroppedFile(event);
}, true);

void listen<DragPlace>("tauri://drag-enter", (event) => {
  composerForm.classList.toggle("dropping", dragHitsThread(event.payload.position));
});
void listen<DragPlace>("tauri://drag-over", (event) => {
  composerForm.classList.toggle("dropping", dragHitsThread(event.payload.position));
});
void listen("tauri://drag-leave", () => {
  composerForm.classList.remove("dropping");
});
void listen<DragPlace>("tauri://drag-drop", (event) => {
  composerForm.classList.remove("dropping");
  const paths = event.payload.paths || [];
  if (!paths.length || !dragHitsThread(event.payload.position)) {
    return;
  }
  closeStickerPanel();
  void stageDroppedPaths(paths);
});

stickerOpen.addEventListener("click", () => {
  if (!openId || composerInput.disabled) {
    return;
  }
  paintStickerPanel();
  stickerPanel.hidden = !stickerPanel.hidden;
  stickerOpen.setAttribute("aria-expanded", stickerPanel.hidden ? "false" : "true");
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
  sheetList.hidden = true;
  sheetAbout.hidden = true;
  sheetLanguage.hidden = true;
  sheet.hidden = false;
}

function buildStamp(iso: string): string {
  if (!iso) {
    return "—";
  }
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) {
    return iso;
  }
  return date.toLocaleString(intlLocale(), {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
    hour12: true,
    timeZone: "Asia/Bangkok",
  });
}

function showAbout(): void {
  sheetTitle.textContent = t("aboutLine");
  aboutBuild.textContent = String(__BUILD_NUMBER__);
  aboutTime.textContent = buildStamp(__BUILD_TIME__);
  sheetBody.hidden = true;
  sheetSettings.hidden = true;
  sheetList.hidden = true;
  sheetAbout.hidden = false;
  sheetLanguage.hidden = true;
  sheet.hidden = false;
}

type ListedDevice = { name: string; detail: string };
type DeviceLists = { outputs: ListedDevice[]; inputs: ListedDevice[]; cameras: ListedDevice[]; audioError: string };

let listedDevices: DeviceLists | null = null;
let pendingLanguage: "auto" | (typeof LOCALES)[number]["id"] = languagePreference();

function displayName(name: string): string {
  return name === "Direct chat" ? t("directChat") : name;
}

function paintLanguageCurrent(): void {
  languageCurrent.textContent = languageName(languagePreference());
}

function paintLanguageList(): void {
  pendingLanguage = languagePreference();
  languageList.replaceChildren();
  const choices: Array<"auto" | (typeof LOCALES)[number]["id"]> = ["auto", ...LOCALES.map((locale) => locale.id)];
  for (const id of choices) {
    const item = document.createElement("li");
    const label = document.createElement("label");
    const input = document.createElement("input");
    input.type = "radio";
    input.name = "app-language";
    input.value = id;
    input.checked = id === pendingLanguage;
    input.addEventListener("change", () => {
      pendingLanguage = id;
    });
    const name = document.createElement("span");
    name.textContent = id === "auto" ? t("autoLanguage") : languageName(id);
    label.append(input, name);
    item.append(label);
    languageList.append(item);
  }
}

function showLanguage(): void {
  paintLanguageList();
  sheetTitle.textContent = t("language");
  sheetBody.hidden = true;
  sheetSettings.hidden = true;
  sheetList.hidden = true;
  sheetAbout.hidden = true;
  sheetLanguage.hidden = false;
  sheet.hidden = false;
}

function refreshLocale(): void {
  applyI18n();
  paintLanguageCurrent();
  if (statusKey) {
    setStatus(statusKey);
  }
  if (!sheet.hidden) {
    if (!sheetLanguage.hidden) {
      const chosen = pendingLanguage;
      paintLanguageList();
      pendingLanguage = chosen;
      languageList.querySelectorAll<HTMLInputElement>("input").forEach((input) => {
        input.checked = input.value === chosen;
      });
      sheetTitle.textContent = t("language");
    } else if (!sheetSettings.hidden) {
      sheetTitle.textContent = t("settings");
      if (listedDevices) {
        fillDevices(devicesOutput, listedDevices.outputs, t("noSpeakers"));
        fillDevices(devicesInput, listedDevices.inputs, t("noMics"));
        fillDevices(devicesCamera, listedDevices.cameras, t("noCameras"));
      }
    } else if (!sheetAbout.hidden) {
      aboutTime.textContent = buildStamp(__BUILD_TIME__);
      sheetTitle.textContent = t("aboutLine");
    }
  }
  if (!signedIn) {
    return;
  }
  renderList();
  if (openHistory && openHistory.id === openId) {
    paintedHistory = "";
    renderHistory(openHistory);
    return;
  }
  const note = threadBody.querySelector<HTMLElement>(".thread-note");
  if (note?.dataset.kind === "loading") {
    note.textContent = t("loadingMessages");
  } else if (note?.dataset.kind === "empty") {
    note.textContent = t("noMessages");
  }
}

function fillDevices(list: HTMLElement, devices: ListedDevice[], empty: string): void {
  list.replaceChildren();
  if (devices.length === 0) {
    const item = document.createElement("li");
    item.className = "device-empty";
    item.textContent = empty;
    list.append(item);
    return;
  }
  for (const device of devices) {
    const item = document.createElement("li");
    const name = document.createElement("span");
    name.textContent = device.name;
    item.append(name);
    if (device.detail) {
      const detail = document.createElement("span");
      detail.className = "device-detail";
      detail.textContent = device.detail;
      item.append(detail);
    }
    list.append(item);
  }
}

async function showSettings(): Promise<void> {
  sheetTitle.textContent = t("settings");
  sheetBody.hidden = true;
  sheetSettings.hidden = false;
  sheetList.hidden = true;
  sheetAbout.hidden = true;
  sheetLanguage.hidden = true;
  sheet.hidden = false;
  paintLanguageCurrent();
  devicesNote.hidden = true;
  fillDevices(devicesOutput, [], t("looking"));
  fillDevices(devicesInput, [], t("looking"));
  fillDevices(devicesCamera, [], t("looking"));
  minimizeToTray.disabled = true;
  try {
    const settings = await invoke<{ minimizeToTray: boolean }>("get_settings");
    minimizeToTray.checked = settings.minimizeToTray;
  } catch (error) {
    setStatusText(error instanceof Error ? error.message : String(error));
  }
  minimizeToTray.disabled = false;
  try {
    const devices = await invoke<DeviceLists>("list_media_devices");
    listedDevices = devices;
    fillDevices(devicesOutput, devices.outputs, t("noSpeakers"));
    fillDevices(devicesInput, devices.inputs, t("noMics"));
    fillDevices(devicesCamera, devices.cameras, t("noCameras"));
    devicesNote.hidden = devices.audioError.length === 0;
    devicesNote.textContent = devices.audioError;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    fillDevices(devicesOutput, [], t("couldntSpeakers"));
    fillDevices(devicesInput, [], t("couldntMics"));
    fillDevices(devicesCamera, [], t("couldntCameras"));
    devicesNote.hidden = false;
    devicesNote.textContent = message;
    listedDevices = null;
  }
}

languageOpen.addEventListener("click", () => {
  showLanguage();
});

languageSave.addEventListener("click", () => {
  sheetLanguage.hidden = true;
  sheetSettings.hidden = false;
  setLanguage(pendingLanguage);
  sheetTitle.textContent = t("settings");
});

languageCancel.addEventListener("click", () => {
  sheet.hidden = true;
});

applyI18n();
paintLanguageCurrent();
onLocaleChange(refreshLocale);

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
    showSheet(t("help"), t("helpMissing"));
  } else if (action === "about") {
    showAbout();
  } else if (action === "logout") {
    void logout();
  } else if (action === "quit") {
    void invoke("quit");
  }
});

document.addEventListener("click", (event) => {
  closeMenu();
  closeReactionPicker();
  if (Date.now() - menuOpenedAt < 400) {
    return;
  }
  if (msgMenu.contains(event.target as Node)) {
    return;
  }
  closeMsgMenu();
});

document.querySelector<HTMLButtonElement>("#sheet-close")!.addEventListener("click", () => {
  sheet.hidden = true;
});

minimizeToTray.addEventListener("change", () => {
  const enabled = minimizeToTray.checked;
  void invoke("set_minimize_to_tray", { enabled }).catch((error: unknown) => {
    minimizeToTray.checked = !enabled;
    setStatusText(error instanceof Error ? error.message : String(error));
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
  if (event.key === "Escape" && !msgMenu.hidden) {
    closeMsgMenu();
    return;
  }
  if (event.key === "Escape" && reactionPicker && lightbox.hidden) {
    closeReactionPicker();
    return;
  }
  if (event.key === "Escape" && !sheet.hidden && lightbox.hidden) {
    sheet.hidden = true;
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

function rememberLive(message: HistoryMessage): void {
  if (!openHistory || !message.id) {
    return;
  }
  const index = openHistory.messages.findIndex((entry) => entry.id === message.id);
  if (index >= 0) {
    const prior = openHistory.messages[index];
    openHistory.messages[index] = {
      ...prior,
      ...message,
      kind: message.kind || prior.kind,
      mediaId: message.mediaId || prior.mediaId,
      gid: message.gid || prior.gid,
      seq: message.seq || prior.seq,
    };
    return;
  }
  openHistory.messages.push(message);
}

function showAlbum(gid: string, kind: string): boolean {
  if (!openHistory || !gid || (kind !== "image" && kind !== "video")) {
    return false;
  }
  const members = openHistory.messages
    .filter((entry) => entry.gid === gid && entry.kind === kind && entry.mediaId)
    .sort((left, right) => (left.seq || 0) - (right.seq || 0) || left.time - right.time);
  const grouped = groupMessages(members)[0];
  if (!grouped?.media?.length) {
    return false;
  }
  const bubble = messageBubble(grouped);
  bubble.dataset.gid = gid;
  const hosts: HTMLElement[] = [];
  const seen = new Set<HTMLElement>();
  const add = (row: HTMLElement | null): void => {
    if (row && !seen.has(row)) {
      seen.add(row);
      hosts.push(row);
    }
  };
  add(albumHost(gid));
  for (const member of members) {
    add(member.id ? messageRow(member.id) : null);
  }
  threadBody.querySelector(".thread-note")?.remove();
  if (hosts.length > 0) {
    hosts[0].replaceWith(bubble);
    for (const extra of hosts.slice(1)) {
      extra.remove();
    }
  } else {
    threadBody.append(bubble);
  }
  return true;
}

function appendLiveMessage(update: PollUpdate, followTail: boolean, batch: { focus: string; end: boolean }): void {
  const message = update.thread;
  if (!message?.text || update.id !== openId) {
    return;
  }
  if (message.kind === "unsent") {
    if (message.id) {
      const row = messageRow(message.id);
      if (row) {
        paintUnsent(row, message.mine, message.name || "");
      }
    }
    return;
  }
  rememberLive(message);
  if (message.gid && showAlbum(message.gid, message.kind || "")) {
    if (message.mine) {
      batch.end = true;
    }
    return;
  }
  if (message.id && messageRow(message.id)) {
    return;
  }
  const claimed = message.mine ? takeMatchingOutgoing(update.id, message) : null;
  if (claimed) {
    const row = messageRow(claimed.localId);
    if (row && message.id) {
      setDelivery(row, "sent", message.time || claimed.time, message.id);
      return;
    }
  }
  if (message.replyId && !message.replyText) {
    const source = messageRow(message.replyId);
    message.replyText = source ? quoteText(source) : t("message");
    message.replyName = source ? senderLabel(source) : "";
    message.replyPicture = source ? rowPicture(source) : "";
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
    if (update.unsentId) {
      const row = messageRow(update.unsentId);
      if (row && (!update.id || update.id === openId)) {
        paintUnsent(row, row.classList.contains("mine"), senderLabel(row));
      }
      continue;
    }
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
    setStatusText(error instanceof Error ? error.message : String(error));
    return;
  }
  rows = [];
  openId = "";
  discardAttachment();
  shownUrl = "";
  app.hidden = true;
  login.hidden = false;
  card.hidden = true;
  pin.hidden = true;
  refresh.hidden = false;
  setStatus("openingQr");
  try {
    await invoke("start_login");
  } catch (error) {
    setStatusText(error instanceof Error ? error.message : String(error));
  }
}

(window as unknown as { __openChat?: (detail: { id?: string; name?: string; picture?: string } | string) => void }).__openChat = (detail) => {
  const id = typeof detail === "string" ? detail : detail.id || "";
  const name = typeof detail === "string" ? "" : detail.name?.trim() || "";
  const picture = typeof detail === "string" ? "" : detail.picture || "";
  if (!id.startsWith("u")) {
    return;
  }
  search.value = "";
  if (filter !== "all") {
    filter = "all";
    document.querySelectorAll("#tabs button").forEach((tab) => {
      tab.classList.toggle("on", (tab as HTMLButtonElement).dataset.filter === "all");
    });
  }
  let chat = rows.find((row) => row.id === id);
  if (!chat) {
    chat = {
      id,
      name: name || "Direct chat",
      lastMessage: "",
      sealed: false,
      time: Date.now(),
      members: null,
      picture,
      muted: false,
      kind: "friend",
      unread: null,
    };
    rows.unshift(chat);
  } else if (name && (!chat.name || chat.name === "Direct chat")) {
    chat.name = name;
    if (!chat.picture && picture) {
      chat.picture = picture;
    }
  }
  const index = visibleRows().findIndex((row) => row.id === id);
  if (index < 0) {
    return;
  }
  selected = index;
  renderList();
  chats.querySelector("li.selected")?.scrollIntoView({ block: "nearest" });
  composerInput.focus();
};

(window as unknown as { __renameChat?: (detail: { id?: string; name?: string }) => void }).__renameChat = (detail) => {
  patchChat(detail);
};

(window as unknown as { __patchChat?: (detail: { id?: string; name?: string; members?: number | null }) => void }).__patchChat = (detail) => {
  patchChat(detail);
};

refresh.addEventListener("click", () => {
  void start();
});

void start();
