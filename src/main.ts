import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import QRCode from "qrcode";
import { FONT_IDS, fontStack, readAppearance, saveAppearance, toggleTheme, type FontId } from "./appearance";
import { applyI18n, intlLocale, languageName, languagePreference, LOCALES, onLocaleChange, resolvedLanguage, setLanguage, t, type MessageKey } from "./i18n";

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
  replyThumb?: string;
  replyMediaId?: string;
  duration?: number;
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
  duration?: number;
};

type ReadMark = { end: string };

type HistoryPayload = {
  id: string;
  lastMessageId?: string;
  messages: HistoryMessage[];
};

type ChatPayload = {
  type: string;
  profile: { name: string; picture: string; mid?: string; status?: string };
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
const autoReplyButton = document.querySelector<HTMLButtonElement>("#auto-reply")!;
const attachFile = document.querySelector<HTMLButtonElement>("#attach-file")!;
const stickerOpen = document.querySelector<HTMLButtonElement>("#sticker-open")!;
const dictateButton = document.querySelector<HTMLButtonElement>("#dictate-message")!;
const voiceButton = document.querySelector<HTMLButtonElement>("#voice-message")!;
const voiceHold = document.querySelector<HTMLElement>("#voice-hold")!;
const voiceHoldTime = document.querySelector<HTMLElement>("#voice-hold-time")!;
const voiceHoldWaves = document.querySelector<HTMLElement>("#voice-hold-waves")!;
const voiceHoldHint = document.querySelector<HTMLElement>("#voice-hold-hint")!;
const dictateMeter = document.querySelector<HTMLElement>("#dictate-meter")!;
const dictateFill = document.querySelector<HTMLElement>("#dictate-meter-fill")!;
const stickerSuggest = document.querySelector<HTMLElement>("#sticker-suggest")!;
const stickerPanel = document.querySelector<HTMLElement>("#sticker-panel")!;
const stickerTabMine = document.querySelector<HTMLButtonElement>("#sticker-tab-mine")!;
const stickerTabShop = document.querySelector<HTMLButtonElement>("#sticker-tab-shop")!;
const stickerPacks = document.querySelector<HTMLElement>("#sticker-packs")!;
const stickerOffer = document.querySelector<HTMLElement>("#sticker-offer")!;
const stickerOfferText = document.querySelector<HTMLElement>("#sticker-offer-text")!;
const stickerBuy = document.querySelector<HTMLButtonElement>("#sticker-buy")!;
const stickerElsewhere = document.querySelector<HTMLElement>("#sticker-elsewhere")!;
const stickerStoreLink = document.querySelector<HTMLButtonElement>("#sticker-store-link")!;
const stickerGrid = document.querySelector<HTMLElement>("#sticker-grid")!;
const stickerNote = document.querySelector<HTMLElement>("#sticker-note")!;
const fileInput = document.querySelector<HTMLInputElement>("#file-input")!;
const sendButton = document.querySelector<HTMLButtonElement>("#send-message")!;
const sendMenu = document.querySelector<HTMLElement>("#send-menu")!;
const sendNote = document.querySelector<HTMLParagraphElement>("#send-note")!;
const attachPreview = document.querySelector<HTMLElement>("#attach-preview")!;
const sendError = document.querySelector<HTMLParagraphElement>("#send-error")!;
const railMore = document.querySelector<HTMLButtonElement>("#rail-more")!;
const railPeople = document.querySelector<HTMLButtonElement>("#rail-people")!;
const railChats = document.querySelector<HTMLButtonElement>("#rail-chats")!;
const railFriends = document.querySelector<HTMLButtonElement>("#rail-friends")!;
const railAgent = document.querySelector<HTMLButtonElement>("#rail-agent")!;
const agentPane = document.querySelector<HTMLElement>("#agent")!;
const agentLog = document.querySelector<HTMLElement>("#agent-log")!;
const agentForm = document.querySelector<HTMLFormElement>("#agent-form")!;
const agentInput = document.querySelector<HTMLTextAreaElement>("#agent-input")!;
const agentSend = document.querySelector<HTMLButtonElement>("#agent-send")!;
const agentSettingsButton = document.querySelector<HTMLButtonElement>("#agent-settings")!;
const agentClear = document.querySelector<HTMLButtonElement>("#agent-clear")!;
const stage = document.querySelector<HTMLElement>("#stage")!;
const peoplePane = document.querySelector<HTMLElement>("#people-pane")!;
const peopleQuery = document.querySelector<HTMLInputElement>("#people-query")!;
const peopleList = document.querySelector<HTMLUListElement>("#people")!;
const peopleNote = document.querySelector<HTMLElement>("#people-note")!;
const addPane = document.querySelector<HTMLElement>("#add-pane")!;
const addHome = document.querySelector<HTMLElement>("#add-home")!;
const addSearchView = document.querySelector<HTMLElement>("#add-search-view")!;
const addGroupView = document.querySelector<HTMLElement>("#add-group-view")!;
const addRecsToggle = document.querySelector<HTMLButtonElement>("#add-recs-toggle")!;
const addRecsLabel = document.querySelector<HTMLElement>("#add-recs-label")!;
const addRecs = document.querySelector<HTMLUListElement>("#add-recs")!;
const addRecsNote = document.querySelector<HTMLElement>("#add-recs-note")!;
const addSearchForm = document.querySelector<HTMLFormElement>("#add-search-form")!;
const addSearchQuery = document.querySelector<HTMLInputElement>("#add-search-query")!;
const addSearchResult = document.querySelector<HTMLElement>("#add-search-result")!;
const addSearchNote = document.querySelector<HTMLElement>("#add-search-note")!;
const addGroupInput = document.querySelector<HTMLInputElement>("#add-group-input")!;
const addGroupQuery = document.querySelector<HTMLInputElement>("#add-group-query")!;
const addGroupList = document.querySelector<HTMLUListElement>("#add-group-list")!;
const addGroupNote = document.querySelector<HTMLElement>("#add-group-note")!;
const addGroupCreate = document.querySelector<HTMLButtonElement>("#add-group-create")!;
const railMenu = document.querySelector<HTMLElement>("#rail-menu")!;
const sheet = document.querySelector<HTMLElement>("#sheet")!;
const sheetTitle = document.querySelector<HTMLElement>("#sheet-title")!;
const sheetBody = document.querySelector<HTMLElement>("#sheet-body")!;
const sheetSettings = document.querySelector<HTMLElement>("#sheet-settings")!;
const sheetAbout = document.querySelector<HTMLElement>("#sheet-about")!;
const aboutBuild = document.querySelector<HTMLElement>("#about-build")!;
const aboutTime = document.querySelector<HTMLElement>("#about-time")!;
const minimizeToTray = document.querySelector<HTMLInputElement>("#minimize-to-tray")!;
const keepOnTop = document.querySelector<HTMLInputElement>("#keep-on-top")!;
const focusOnMessageBox = document.querySelector<HTMLInputElement>("#focus-on-message")!;
let focusOnMessage = false;
const fontFamily = document.querySelector<HTMLSelectElement>("#font-family")!;
const textSize = document.querySelector<HTMLInputElement>("#text-size")!;
const textSizeLabel = document.querySelector<HTMLElement>("#text-size-label")!;
const notificationTones = document.querySelector<HTMLElement>("#notification-tones")!;
const notificationVolume = document.querySelector<HTMLInputElement>("#notification-volume")!;
const notificationVolumeLabel = document.querySelector<HTMLElement>("#notification-volume-label")!;
const TEXT_SIZE_KEYS = ["textSmall", "textDefault", "textLarge", "textLarger", "textLargest"] as const satisfies readonly MessageKey[];
const devicesOutput = document.querySelector<HTMLElement>("#devices-output")!;
const devicesInput = document.querySelector<HTMLElement>("#devices-input")!;
const micTestButton = document.querySelector<HTMLButtonElement>("#mic-test")!;
const micTestMeter = document.querySelector<HTMLElement>("#mic-test-meter")!;
const micTestFill = document.querySelector<HTMLElement>("#mic-test-fill")!;
const micTestResult = document.querySelector<HTMLParagraphElement>("#mic-test-result")!;
const micTestPlayer = document.querySelector<HTMLAudioElement>("#mic-test-player")!;
const devicesCamera = document.querySelector<HTMLElement>("#devices-camera")!;
const devicesNote = document.querySelector<HTMLElement>("#devices-note")!;
const lightbox = document.querySelector<HTMLElement>("#lightbox")!;
const lightboxImg = document.querySelector<HTMLImageElement>("#lightbox-img")!;
const lightboxVideo = document.querySelector<HTMLVideoElement>("#lightbox-video")!;
const lightboxPrev = document.querySelector<HTMLButtonElement>("#lightbox-prev")!;
const lightboxNext = document.querySelector<HTMLButtonElement>("#lightbox-next")!;
const lightboxNav = document.querySelector<HTMLElement>("#lightbox-nav")!;
const lightboxCount = document.querySelector<HTMLElement>("#lightbox-count")!;
const msgMenu = document.querySelector<HTMLElement>("#msg-menu")!;
const replyBar = document.querySelector<HTMLElement>("#reply-bar")!;
const replyAvatar = document.querySelector<HTMLElement>("#reply-avatar")!;
const replyLabel = document.querySelector<HTMLElement>("#reply-label")!;
const replyText = document.querySelector<HTMLElement>("#reply-text")!;
const replyThumb = document.querySelector<HTMLImageElement>("#reply-thumb")!;
const sheetList = document.querySelector<HTMLElement>("#sheet-list")!;
const sheetLanguage = document.querySelector<HTMLElement>("#sheet-language")!;
const sheetProfile = document.querySelector<HTMLElement>("#sheet-profile")!;
const profileOpen = document.querySelector<HTMLButtonElement>("#profile-open")!;
const profileCurrent = document.querySelector<HTMLElement>("#profile-current")!;
const profileCoverImg = document.querySelector<HTMLImageElement>("#profile-cover-img")!;
const profileFaceImg = document.querySelector<HTMLImageElement>("#profile-face-img")!;
const profileFaceLetter = document.querySelector<HTMLElement>("#profile-face-letter")!;
const profilePhotoChange = document.querySelector<HTMLButtonElement>("#profile-photo-change")!;
const profilePhotoRemove = document.querySelector<HTMLButtonElement>("#profile-photo-remove")!;
const profileCoverChange = document.querySelector<HTMLButtonElement>("#profile-cover-change")!;
const profileNameInput = document.querySelector<HTMLInputElement>("#profile-name")!;
const profileStatusInput = document.querySelector<HTMLInputElement>("#profile-status")!;
const profileNote = document.querySelector<HTMLElement>("#profile-note")!;
const profileSave = document.querySelector<HTMLButtonElement>("#profile-save")!;
const profileCancel = document.querySelector<HTMLButtonElement>("#profile-cancel")!;
const profileFile = document.querySelector<HTMLInputElement>("#profile-file")!;
const railMe = document.querySelector<HTMLButtonElement>("#rail-me")!;
const languageCurrent = document.querySelector<HTMLElement>("#language-current")!;
const languageList = document.querySelector<HTMLUListElement>("#language-list")!;
const languageOpen = document.querySelector<HTMLButtonElement>("#language-open")!;
const languageSave = document.querySelector<HTMLButtonElement>("#language-save")!;
const languageCancel = document.querySelector<HTMLButtonElement>("#language-cancel")!;
const sheetAgent = document.querySelector<HTMLElement>("#sheet-agent")!;
const sheetChatLlm = document.querySelector<HTMLElement>("#sheet-chat-llm")!;
const chatLlmButton = document.querySelector<HTMLButtonElement>("#chat-llm")!;
const chatLlmModelLabel = document.querySelector<HTMLElement>("#chat-llm-name")!;
const aiMark = document.querySelector<HTMLElement>("#ai-mark")!;
const agentOpen = document.querySelector<HTMLButtonElement>("#agent-open")!;
const agentCurrent = document.querySelector<HTMLElement>("#agent-current")!;
const agentEndpoint = document.querySelector<HTMLInputElement>("#agent-endpoint")!;
const agentModel = document.querySelector<HTMLInputElement>("#agent-model")!;
const agentApiKey = document.querySelector<HTMLInputElement>("#agent-api-key")!;
const agentProtocol = document.querySelector<HTMLSelectElement>("#agent-protocol")!;
const agentTemperature = document.querySelector<HTMLInputElement>("#agent-temperature")!;
const agentTopP = document.querySelector<HTMLInputElement>("#agent-top-p")!;
const agentMaxTokens = document.querySelector<HTMLInputElement>("#agent-max-tokens")!;
const agentThinking = document.querySelector<HTMLSelectElement>("#agent-thinking")!;
const agentStream = document.querySelector<HTMLInputElement>("#agent-stream")!;
const agentExtra = document.querySelector<HTMLTextAreaElement>("#agent-extra")!;
const agentSettingsNote = document.querySelector<HTMLElement>("#agent-settings-note")!;
const replyTemperature = document.querySelector<HTMLInputElement>("#reply-temperature")!;
const replyTopP = document.querySelector<HTMLInputElement>("#reply-top-p")!;
const replyMaxTokens = document.querySelector<HTMLInputElement>("#reply-max-tokens")!;
const replyThinking = document.querySelector<HTMLSelectElement>("#reply-thinking")!;
const agentAutoContext = document.querySelector<HTMLTextAreaElement>("#agent-auto-context")!;
const replyExtra = document.querySelector<HTMLTextAreaElement>("#reply-extra")!;
const chatLlmProtocol = document.querySelector<HTMLSelectElement>("#chat-llm-protocol")!;
const chatLlmEndpoint = document.querySelector<HTMLInputElement>("#chat-llm-endpoint")!;
const chatLlmModel = document.querySelector<HTMLInputElement>("#chat-llm-model")!;
const chatLlmKey = document.querySelector<HTMLInputElement>("#chat-llm-key")!;
const chatLlmTemperature = document.querySelector<HTMLInputElement>("#chat-llm-temperature")!;
const chatLlmTopP = document.querySelector<HTMLInputElement>("#chat-llm-top-p")!;
const chatLlmMaxTokens = document.querySelector<HTMLInputElement>("#chat-llm-max-tokens")!;
const chatLlmThinking = document.querySelector<HTMLSelectElement>("#chat-llm-thinking")!;
const chatLlmLanguage = document.querySelector<HTMLSelectElement>("#chat-llm-language")!;
const chatLlmHistory = document.querySelector<HTMLInputElement>("#chat-llm-history")!;
const chatLlmContext = document.querySelector<HTMLTextAreaElement>("#chat-llm-context")!;
const chatLlmMarkAi = document.querySelector<HTMLInputElement>("#chat-llm-mark-ai")!;
const chatLlmInterrupt = document.querySelector<HTMLInputElement>("#chat-llm-interrupt")!;
const chatLlmExtra = document.querySelector<HTMLTextAreaElement>("#chat-llm-extra")!;
const autoReplyCancelled = document.querySelector<HTMLElement>("#auto-reply-cancelled")!;
const autoReplyError = document.querySelector<HTMLElement>("#auto-reply-error")!;
const chatLlmUnsaved = document.querySelector<HTMLElement>("#chat-llm-unsaved")!;
const chatLlmNote = document.querySelector<HTMLElement>("#chat-llm-note")!;
const chatLlmSave = document.querySelector<HTMLButtonElement>("#chat-llm-save")!;
const chatLlmClear = document.querySelector<HTMLButtonElement>("#chat-llm-clear")!;
const chatLlmCancel = document.querySelector<HTMLButtonElement>("#chat-llm-cancel")!;
const chatLlmCopy = document.querySelector<HTMLSelectElement>("#chat-llm-copy")!;
const chatLlmCopyApply = document.querySelector<HTMLButtonElement>("#chat-llm-copy-apply")!;
const autoReplyWait = document.querySelector<HTMLElement>("#auto-reply-wait")!;
const autoReplyElapsed = document.querySelector<HTMLElement>("#auto-reply-elapsed")!;
const agentSave = document.querySelector<HTMLButtonElement>("#agent-save")!;
const agentCancel = document.querySelector<HTMLButtonElement>("#agent-cancel")!;

let statusKey: MessageKey | "" = "";
let shownUrl = "";
let myName = "";
let myStatus = "";
let myPicture = "";
let myCover = "";
let myMid = "";
let profileBusy = false;
let profileFromSettings = false;
let profilePick: "photo" | "cover" = "photo";
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
let expectedTop = -1;
let userMoved = false;
let placeToken = 0;
let layoutWatch: { token: number; kind: "tail" | "anchor"; stop: () => void } | null = null;
let signedIn = false;
type RailMode = "chats" | "friends" | "add" | "agent";
type DirectoryPerson = { id: string; name: string; picture: string; favorite?: boolean; friend?: boolean; status?: string };
type Recommendation = { id: string; name: string; picture: string; reason?: string };
let railMode: RailMode = "chats";
let directoryPeople: DirectoryPerson[] = [];
let peopleToken = 0;
let peopleReady = false;
let peopleError = "";
let recommendations: Recommendation[] = [];
let recsToken = 0;
let recsOpen = true;
let recsReady = false;
let recsError = "";
const groupPicked = new Set<string>();
const mediaUrls = new Map<string, string>();
const animatedMedia = new Set<string>();
const mediaMimes = new Map<string, string>();
const mediaJobs = new Map<string, Promise<void>>();
let gallery: { ids: string[]; index: number } | null = null;
let localGallery: { urls: string[]; index: number } | null = null;
let trayBadge = -1;

type ReplyDraft = { id: string; text: string; name: string; picture: string; thumb?: string; mediaId?: string };
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
  silent?: boolean;
  duration?: number;
  peaks?: number[];
  voice?: boolean;
};
type AttachmentDraft = {
  chatId: string;
  file: File;
  name: string;
  mime: string;
  kind: "image" | "video" | "audio" | "file";
  previewUrl: string;
  voice?: boolean;
  duration?: number;
  peaks?: number[];
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
const SOUND_VOLUME_KEY = "lineapp.notificationVolume";
const SOUND_TONE_KEY = "lineapp.notificationTone";
const NOTIFICATION_PEAK = 1;
const DEFAULT_NOTIFICATION_VOLUME = 75;
const TONE_IDS = ["doorbell", "sparkle", "piano", "harp", "kalimba", "whistle", "ripple", "arcade", "gong", "bell", "marimba", "pop", "knock", "beep"] as const;
type ToneId = (typeof TONE_IDS)[number];
const DEFAULT_TONE: ToneId = "doorbell";

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

function readNotificationVolume(): number {
  const raw = localStorage.getItem(SOUND_VOLUME_KEY);
  if (raw === null) {
    return DEFAULT_NOTIFICATION_VOLUME;
  }
  const value = Number(raw);
  if (!Number.isFinite(value)) {
    return DEFAULT_NOTIFICATION_VOLUME;
  }
  return Math.min(100, Math.max(0, Math.round(value)));
}

function setNotificationVolume(volume: number): void {
  localStorage.setItem(SOUND_VOLUME_KEY, String(volume));
}

function notificationGain(volume: number): number {
  if (volume <= 0) {
    return 0;
  }
  return (volume / 100) ** 2 * NOTIFICATION_PEAK;
}

function paintNotificationVolume(): void {
  const volume = readNotificationVolume();
  const label = `${volume}%`;
  notificationVolume.value = String(volume);
  notificationVolumeLabel.textContent = label;
  notificationVolume.setAttribute("aria-valuetext", label);
}

function readNotificationTone(): ToneId {
  const raw = localStorage.getItem(SOUND_TONE_KEY);
  return (TONE_IDS as readonly string[]).includes(raw ?? "") ? (raw as ToneId) : DEFAULT_TONE;
}

function setNotificationTone(tone: ToneId): void {
  localStorage.setItem(SOUND_TONE_KEY, tone);
}

function paintNotificationTone(): void {
  const tone = readNotificationTone();
  notificationTones.querySelectorAll<HTMLButtonElement>("button[data-tone]").forEach((button) => {
    const selected = button.dataset.tone === tone;
    button.classList.toggle("is-selected", selected);
    button.setAttribute("aria-selected", selected ? "true" : "false");
  });
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
let notificationBus: WaveShaperNode | undefined;

function notificationBusOf(ctx: AudioContext): WaveShaperNode {
  if (notificationBus && notificationBus.context === ctx) {
    return notificationBus;
  }
  const shape = ctx.createWaveShaper();
  const samples = 2048;
  const curve = new Float32Array(samples);
  const drive = 3.2;
  const limit = Math.tanh(drive);
  for (let i = 0; i < samples; i += 1) {
    const x = (i / (samples - 1)) * 2 - 1;
    curve[i] = Math.tanh(x * drive) / limit;
  }
  shape.curve = curve;
  shape.oversample = "2x";
  shape.connect(ctx.destination);
  notificationBus = shape;
  return shape;
}

async function playChime(): Promise<void> {
  const Ctx = window.AudioContext;
  if (!Ctx) {
    return;
  }
  const peak = notificationGain(readNotificationVolume());
  if (peak <= 0) {
    return;
  }
  if (!audioCtx) {
    audioCtx = new Ctx();
  }
  if (audioCtx.state === "suspended") {
    void audioCtx.resume().catch(() => undefined);
  }
  const now = audioCtx.currentTime + 0.02;
  const tone = readNotificationTone();
  if (tone === "sparkle") {
    playSparkle(audioCtx, now, peak);
  } else if (tone === "piano") {
    playPiano(audioCtx, now, peak);
  } else if (tone === "harp") {
    playHarp(audioCtx, now, peak);
  } else if (tone === "kalimba") {
    playKalimba(audioCtx, now, peak);
  } else if (tone === "whistle") {
    playWhistle(audioCtx, now, peak);
  } else if (tone === "ripple") {
    playRipple(audioCtx, now, peak);
  } else if (tone === "arcade") {
    playArcade(audioCtx, now, peak);
  } else if (tone === "gong") {
    playGong(audioCtx, now, peak);
  } else if (tone === "bell") {
    playBell(audioCtx, now, peak);
  } else if (tone === "beep") {
    playBeep(audioCtx, now, peak);
  } else if (tone === "pop") {
    playPop(audioCtx, now, peak);
  } else if (tone === "knock") {
    playKnock(audioCtx, now, peak);
  } else if (tone === "marimba") {
    playMarimba(audioCtx, now, peak);
  } else {
    playDoorbell(audioCtx, now, peak);
  }
}

function note(
  ctx: AudioContext,
  start: number,
  dur: number,
  peak: number,
  freq: number,
  type: OscillatorType,
  endFreq?: number,
  attack = 0.012,
): void {
  const osc = ctx.createOscillator();
  const gain = ctx.createGain();
  const level = Math.max(peak, 0.0002);
  const attackAt = start + attack;
  const held = attackAt + Math.min(0.07, dur * 0.55);
  const ended = Math.max(start + dur, held + 0.05);
  osc.type = type;
  osc.frequency.setValueAtTime(Math.max(freq, 1), start);
  if (endFreq && endFreq > 0) {
    osc.frequency.exponentialRampToValueAtTime(endFreq, start + dur);
  }
  gain.gain.setValueAtTime(0.0001, start);
  gain.gain.exponentialRampToValueAtTime(level, attackAt);
  gain.gain.setValueAtTime(level, held);
  gain.gain.exponentialRampToValueAtTime(0.0001, ended);
  osc.connect(gain);
  gain.connect(notificationBusOf(ctx));
  osc.start(start);
  osc.stop(ended + 0.02);
}

function noiseTap(ctx: AudioContext, start: number, dur: number, peak: number, freq: number, q: number): void {
  const length = Math.max(1, Math.floor(ctx.sampleRate * dur));
  const buffer = ctx.createBuffer(1, length, ctx.sampleRate);
  const data = buffer.getChannelData(0);
  for (let i = 0; i < length; i += 1) {
    data[i] = Math.random() * 2 - 1;
  }
  const src = ctx.createBufferSource();
  src.buffer = buffer;
  const filter = ctx.createBiquadFilter();
  filter.type = "bandpass";
  filter.frequency.value = freq;
  filter.Q.value = q;
  const gain = ctx.createGain();
  const level = Math.max(peak, 0.0002);
  const attackAt = start + 0.004;
  const held = attackAt + Math.min(0.03, dur * 0.45);
  const ended = Math.max(attackAt + dur, held + 0.03);
  gain.gain.setValueAtTime(0.0001, start);
  gain.gain.exponentialRampToValueAtTime(level, attackAt);
  gain.gain.setValueAtTime(level, held);
  gain.gain.exponentialRampToValueAtTime(0.0001, ended);
  src.connect(filter);
  filter.connect(gain);
  gain.connect(notificationBusOf(ctx));
  src.start(start);
  src.stop(ended + 0.02);
}

function playBeep(ctx: AudioContext, now: number, peak: number): void {
  [784, 1046].forEach((freq, index) => {
    note(ctx, now + index * 0.09, 0.16, peak, freq, "sine", undefined, 0.015);
  });
}

function playBell(ctx: AudioContext, now: number, peak: number): void {
  const partials: [number, number, number][] = [
    [880, 0.55, 0.62],
    [1174.7, 0.22, 0.48],
    [1760, 0.12, 0.36],
    [2349.3, 0.07, 0.24],
  ];
  for (const [freq, share, dur] of partials) {
    note(ctx, now, dur, peak * share, freq, "sine", undefined, 0.006);
  }
}

function playPop(ctx: AudioContext, now: number, peak: number): void {
  note(ctx, now, 0.11, peak * 0.85, 720, "sine", 180, 0.004);
  noiseTap(ctx, now, 0.028, peak * 0.28, 1600, 0.7);
}

function playKnock(ctx: AudioContext, now: number, peak: number): void {
  for (const offset of [0, 0.11]) {
    noiseTap(ctx, now + offset, 0.045, peak * 0.62, 240, 1.6);
    note(ctx, now + offset, 0.06, peak * 0.28, 180, "sine", 90, 0.003);
  }
}

function playMarimba(ctx: AudioContext, now: number, peak: number): void {
  [523.25, 784].forEach((freq, index) => {
    note(ctx, now + index * 0.09, 0.22, peak * 0.75, freq * 1.03, "triangle", freq, 0.005);
  });
}

function playDoorbell(ctx: AudioContext, now: number, peak: number): void {
  const ding: [number, number, number][] = [
    [659.25, 0.62, 0.32],
    [1318.5, 0.2, 0.22],
    [1975.5, 0.08, 0.16],
  ];
  const dong: [number, number, number][] = [
    [523.25, 0.66, 0.48],
    [1046.5, 0.18, 0.32],
    [1568, 0.07, 0.2],
  ];
  for (const [freq, share, dur] of ding) {
    note(ctx, now, dur, peak * share, freq, "sine", undefined, 0.005);
  }
  for (const [freq, share, dur] of dong) {
    note(ctx, now + 0.2, dur, peak * share, freq, "sine", undefined, 0.005);
  }
}

function playSparkle(ctx: AudioContext, now: number, peak: number): void {
  [1318.5, 1568, 1975.5, 2637].forEach((freq, index) => {
    const start = now + index * 0.05;
    note(ctx, start, 0.2, peak * 0.42, freq, "sine", undefined, 0.003);
    note(ctx, start, 0.14, peak * 0.14, freq * 1.007, "sine", undefined, 0.003);
  });
}

function playPiano(ctx: AudioContext, now: number, peak: number): void {
  noiseTap(ctx, now, 0.02, peak * 0.16, 2200, 0.6);
  for (const freq of [261.63, 329.63, 392, 523.25]) {
    note(ctx, now, 0.48, peak * 0.24, freq, "triangle", freq * 0.992, 0.004);
    note(ctx, now, 0.1, peak * 0.08, freq * 2, "sine", undefined, 0.002);
  }
}

function playHarp(ctx: AudioContext, now: number, peak: number): void {
  [261.63, 329.63, 392, 493.88, 523.25, 659.25].forEach((freq, index) => {
    const start = now + index * 0.038;
    note(ctx, start, 0.4, peak * 0.3, freq, "triangle", freq * 0.996, 0.004);
    note(ctx, start, 0.07, peak * 0.06, freq * 2, "sine", undefined, 0.002);
  });
}

function playKalimba(ctx: AudioContext, now: number, peak: number): void {
  [392, 523.25, 659.25, 784].forEach((freq, index) => {
    const start = now + index * 0.075;
    note(ctx, start, 0.3, peak * 0.5, freq, "sine", undefined, 0.003);
    note(ctx, start, 0.16, peak * 0.16, freq * 3.2, "sine", undefined, 0.002);
  });
}

function playWhistle(ctx: AudioContext, now: number, peak: number): void {
  note(ctx, now, 0.16, peak * 0.62, 740, "sine", 1174, 0.018);
  note(ctx, now + 0.15, 0.24, peak * 0.58, 988, "sine", 1568, 0.02);
}

function playRipple(ctx: AudioContext, now: number, peak: number): void {
  [280, 390, 520, 680].forEach((freq, index) => {
    note(ctx, now + index * 0.065, 0.1, peak * 0.62, freq, "sine", freq * 0.5, 0.003);
  });
}

function playArcade(ctx: AudioContext, now: number, peak: number): void {
  note(ctx, now, 0.07, peak * 0.16, 523.25, "square");
  note(ctx, now + 0.08, 0.07, peak * 0.16, 659.25, "square");
  note(ctx, now + 0.16, 0.14, peak * 0.14, 1046.5, "square");
}

function playGong(ctx: AudioContext, now: number, peak: number): void {
  note(ctx, now, 0.72, peak * 0.38, 185, "sine", undefined, 0.012);
  note(ctx, now, 0.72, peak * 0.26, 189.2, "sine", undefined, 0.012);
  note(ctx, now, 0.46, peak * 0.14, 370, "triangle", undefined, 0.01);
  noiseTap(ctx, now, 0.08, peak * 0.14, 320, 0.5);
}

window.addEventListener("focus", () => {
  catchUpRead();
  releaseAttention();
});
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
  stage.hidden = Boolean(chat) || !signedIn;
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
    closeStickerPanel();
    abandonDictation();
    void abandonVoice();
    stopVoicePlayback();
    attachFile.disabled = true;
    stickerOpen.disabled = true;
    discardAttachment();
    sendError.hidden = true;
    sendNote.hidden = true;
    threadHead.replaceChildren();
    delete document.body.dataset.sound;
    refreshSendButton();
    paintAutoReply("");
    return;
  }
  thread.hidden = false;
  paintThreadHead(chat);
  composerInput.disabled = false;
  attachFile.disabled = false;
  stickerOpen.disabled = false;
  paintDictate();
  if (chat.id !== openId) {
    abandonDictation();
    void abandonVoice();
    stopVoicePlayback();
    armedStickers.clear();
    stickerWatch?.disconnect();
    stickerWatch = null;
    closeStickerPanel();
    openId = chat.id;
    openLastId = "";
    paintedHistory = "";
    paintedIds = "";
    openUnread = chat.unread && chat.unread > 0 ? chat.unread : 0;
    sendNote.hidden = true;
    unreadFocus = "";
    userMoved = false;
    stopLayoutWatch();
    placeToken += 1;
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
  if (!sheetChatLlm.hidden && chatLlmEditing !== chat.id) {
    sheetChatLlm.hidden = true;
    sheetProfile.hidden = true;
    sheet.hidden = true;
  }
  void rememberChatLlm(chat.id).then(() => {
    if (openId === chat.id) {
      paintAutoReply(chat.id);
    }
  });
  paintAutoReply(chat.id);
  if (readingChat(chat.id)) {
    dismissUnread(chat.id);
  }
}

function historyStamp(payload: HistoryPayload): string {
  return payload.messages
    .map((message) => [message.id || "", message.sender || "", message.text, message.status || "", message.kind || "", message.mediaId || "", message.gid || "", String(message.seq || 0), message.sticker || "", message.replyId || "", String(message.readers || 0), String(message.time), String(message.duration || 0)].join("\u001f"))
    .join("\u001e");
}

function renderHistory(payload: HistoryPayload): void {
  if (payload.id !== openId) {
    return;
  }
  openHistory = payload;
  replaceAutoThread(payload.id, payload.messages);
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
  placingThread = true;
  threadBody.replaceChildren();
  if (payload.messages.length === 0 && !outgoing.some((item) => item.chatId === payload.id)) {
    const note = document.createElement("p");
    note.className = "thread-note";
    note.dataset.kind = "empty";
    note.textContent = t("noMessages");
    threadBody.append(note);
    document.body.dataset.history = "0";
    unreadFocus = "";
    stopLayoutWatch();
    placingThread = false;
    markOpenRead(payload.id);
    return;
  }
  groupMessages(hydrateReplies(payload.messages)).forEach((message) => threadBody.append(messageBubble(message)));
  const placed = placeOutgoing(payload.id);
  settleThread(payload.messages, previousIds, firstPaint, savedTop);
  if (placed && !userMoved) {
    scrollThreadToEnd();
  }
  if (!layoutWatch) {
    requestAnimationFrame(() => {
      placingThread = false;
    });
  }
  document.body.dataset.history = String(payload.messages.length);
  markOpenRead(payload.id);
  paintAutoReply();
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
  for (const message of added) {
    if (incomingId(message)) {
      queueAutoReply(openId, message.id || "");
    }
  }
  if (userMoved) {
    assignThreadScroll(savedTop);
    return;
  }
  if (added.length === 0) {
    if (unreadFocus) {
      placeThread();
    } else {
      scrollThreadToEnd();
    }
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

function downloadName(name: string, mime: string): string {
  const cleaned = name.replace(/[\\/:*?"<>|\u0000-\u001f]/g, " ").replace(/\s+/g, " ").trim().slice(0, 120);
  if (cleaned && !/^letter-sealed\b/i.test(cleaned) && cleaned.toLowerCase() !== "file") {
    return cleaned;
  }
  const subtype = mime.split("/")[1]?.split(";")[0]?.replace(/[^a-z0-9]/gi, "") || "bin";
  return `download.${subtype || "bin"}`;
}

async function saveDownload(name: string, mime: string, data: string, button?: HTMLButtonElement): Promise<void> {
  if (button) {
    button.disabled = true;
  }
  try {
    const path = await invoke<string | null>("save_download", { name: downloadName(name, mime), data, title: t("save") });
    if (!path) {
      return;
    }
    sendError.hidden = true;
    sendNote.hidden = false;
    sendNote.textContent = t("savedFile", { path });
  } catch (error) {
    sendError.hidden = false;
    sendError.textContent = error instanceof Error ? error.message : String(error);
  } finally {
    if (button) {
      button.disabled = false;
    }
  }
}

async function downloadMessageFile(id: string, name: string, button?: HTMLButtonElement): Promise<void> {
  if (!openId || !messageIdNumber(id)) {
    return;
  }
  if (button) {
    button.disabled = true;
  }
  try {
    const files = await invoke<MediaFile[]>("load_media", { id: openId, messageIds: [id], preview: false, animatedIds: [] });
    const file = files.find((item) => item.id === id) || files[0];
    if (!file?.data) {
      throw new Error(file?.error || t("couldNotDownload"));
    }
    await saveDownload(name, file.mime || "", file.data, button);
  } catch (error) {
    sendError.hidden = false;
    sendError.textContent = error instanceof Error ? error.message : String(error);
    if (button) {
      button.disabled = false;
    }
  }
}

function fileIcon(): SVGSVGElement {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
  path.setAttribute("fill", "currentColor");
  path.setAttribute("d", "M6 2.5h7.2L19.5 8.8V20a1.5 1.5 0 0 1-1.5 1.5H6A1.5 1.5 0 0 1 4.5 20V4A1.5 1.5 0 0 1 6 2.5zm6.8 1.6V9h4.9l-4.9-4.9zM11 13.2v4.1l-1.4-1.4-1.1 1.1 3 3 3-3-1.1-1.1-1.4 1.4v-4.1H11z");
  svg.append(path);
  return svg;
}

function fileCard(label: string, onClick: (button: HTMLButtonElement) => void): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "file-card";
  const name = document.createElement("span");
  name.className = "file-name";
  name.textContent = label.trim() || t("message");
  button.append(fileIcon(), name);
  button.addEventListener("click", (event) => {
    event.stopPropagation();
    onClick(button);
  });
  return button;
}

const VOICE_BARS = 28;
let activeVoice: HTMLAudioElement | null = null;

function voiceClock(ms: number): string {
  const total = Math.max(0, Math.round(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

function voiceWidth(ms: number): number {
  const seconds = Math.max(1, Math.min(60, (ms || 1000) / 1000));
  return Math.round(128 + (seconds - 1) * 2.4);
}

function placeholderPeaks(seed: string): number[] {
  let state = 2166136261;
  const text = seed || "voice";
  for (let index = 0; index < text.length; index += 1) {
    state ^= text.charCodeAt(index);
    state = Math.imul(state, 16777619);
  }
  const peaks: number[] = [];
  for (let index = 0; index < VOICE_BARS; index += 1) {
    state = Math.imul(state, 1664525) + 1013904223;
    peaks.push(20 + (state >>> 0) % 75);
  }
  return peaks;
}

function voiceGlyph(playing: boolean): SVGSVGElement {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
  path.setAttribute("d", playing ? "M7 5h3.2v14H7zm6.8 0H17v14h-3.2z" : "M8.2 5.2v13.6L19 12z");
  svg.append(path);
  return svg;
}

function paintVoiceBars(wave: HTMLElement, peaks: number[], progress: number): void {
  const bars = wave.querySelectorAll<HTMLElement>("i");
  bars.forEach((bar, index) => {
    const peak = peaks[index] ?? 30;
    bar.style.height = `${Math.round(5 + (peak / 100) * 16)}px`;
    bar.classList.toggle("on", progress > 0 && index / Math.max(bars.length, 1) < progress);
  });
}

function voiceMime(bytes: Uint8Array, header: string): string {
  const mime = header.split(";")[0]?.trim().toLowerCase() || "";
  if (mime.startsWith("audio/")) {
    return mime;
  }
  if (bytes.length > 11 && bytes[0] === 0x52 && bytes[1] === 0x49 && bytes[2] === 0x46 && bytes[3] === 0x46) {
    return "audio/wav";
  }
  if (bytes.length > 3 && bytes[0] === 0x49 && bytes[1] === 0x44 && bytes[2] === 0x33) {
    return "audio/mpeg";
  }
  if (bytes.length > 1 && bytes[0] === 0xff && (bytes[1] & 0xe0) === 0xe0) {
    return "audio/mpeg";
  }
  return "audio/mp4";
}

async function decodePeaks(url: string): Promise<number[]> {
  const Ctx = window.AudioContext || (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
  if (!Ctx) {
    return [];
  }
  try {
    const response = await fetch(url);
    const encoded = await response.arrayBuffer();
    const context = new Ctx();
    const audio = await context.decodeAudioData(encoded.slice(0));
    await context.close();
    const channel = audio.getChannelData(0);
    const size = Math.max(1, Math.floor(channel.length / VOICE_BARS));
    const raw: number[] = [];
    let loudest = 0.001;
    for (let bar = 0; bar < VOICE_BARS; bar += 1) {
      const start = bar * size;
      const end = bar === VOICE_BARS - 1 ? channel.length : Math.min(channel.length, start + size);
      let peak = 0;
      for (let index = start; index < end; index += 16) {
        peak = Math.max(peak, Math.abs(channel[index] || 0));
      }
      loudest = Math.max(loudest, peak);
      raw.push(peak);
    }
    return raw.map((peak) => Math.round(Math.max(14, Math.min(100, (peak / loudest) * 100))));
  } catch {
    return [];
  }
}

function stopVoicePlayback(): void {
  activeVoice?.pause();
}

function voiceCard(id: string, duration: number, src = "", label = "", peaks: number[] = []): HTMLElement {
  const card = document.createElement("div");
  card.className = "voice-card";
  card.style.width = `${voiceWidth(duration)}px`;
  card.dataset.mediaId = id;
  card.dataset.duration = String(Math.max(0, duration));
  const caption = label.trim();
  if (caption && !/^(audio|voice message|voice\.m4a|letter-sealed audio)$/i.test(caption)) {
    card.dataset.label = caption;
  }
  const play = document.createElement("button");
  play.type = "button";
  play.className = "voice-play";
  play.setAttribute("aria-label", t("playVoice"));
  play.append(voiceGlyph(false));
  const wave = document.createElement("button");
  wave.type = "button";
  wave.className = "voice-wave";
  wave.tabIndex = -1;
  wave.setAttribute("aria-hidden", "true");
  for (let index = 0; index < VOICE_BARS; index += 1) {
    wave.append(document.createElement("i"));
  }
  let bars = peaks.length === VOICE_BARS ? peaks.slice() : placeholderPeaks(id || caption || "voice");
  paintVoiceBars(wave, bars, 0);
  const length = document.createElement("button");
  length.type = "button";
  length.className = "voice-length";
  length.textContent = voiceClock(duration);
  const audio = document.createElement("audio");
  audio.preload = "none";
  if (src) {
    audio.src = src;
  }
  card.append(play, wave, length, audio);
  let rate = 1;
  const known = (): number => {
    if (Number.isFinite(audio.duration) && audio.duration > 0) {
      return Math.round(audio.duration * 1000);
    }
    return Number(card.dataset.duration) || 0;
  };
  const paintTime = (): void => {
    const total = known();
    if (total > 0) {
      card.dataset.duration = String(total);
      card.style.width = `${voiceWidth(total)}px`;
    }
    const left = audio.paused || audio.ended ? total : Math.max(0, total - audio.currentTime * 1000);
    length.textContent = rate === 1 ? voiceClock(left || total) : `${rate}×`;
  };
  const paintPlay = (): void => {
    const playing = !audio.paused && !audio.ended;
    play.setAttribute("aria-label", t(playing ? "pauseVoice" : "playVoice"));
    play.replaceChildren(voiceGlyph(playing));
    card.classList.toggle("playing", playing);
  };
  const tick = (): void => {
    if (!card.isConnected) {
      audio.pause();
      return;
    }
    const seconds = known() / 1000;
    paintVoiceBars(wave, bars, seconds > 0 ? audio.currentTime / seconds : 0);
    paintTime();
    if (!audio.paused && !audio.ended) {
      requestAnimationFrame(tick);
    }
  };
  const applyPeaks = (next: number[]): void => {
    if (next.length !== VOICE_BARS || !card.isConnected) {
      return;
    }
    bars = next;
    const seconds = known() / 1000;
    paintVoiceBars(wave, bars, seconds > 0 && !audio.paused ? audio.currentTime / seconds : 0);
  };
  audio.addEventListener("loadedmetadata", paintTime);
  audio.addEventListener("pause", paintPlay);
  audio.addEventListener("play", () => {
    paintPlay();
    requestAnimationFrame(tick);
  });
  audio.addEventListener("ended", () => {
    audio.currentTime = 0;
    paintPlay();
    paintVoiceBars(wave, bars, 0);
    paintTime();
  });
  play.addEventListener("click", (event) => {
    event.stopPropagation();
    void toggleVoice(card, applyPeaks);
  });
  wave.addEventListener("click", (event) => {
    event.stopPropagation();
    const seconds = known() / 1000;
    if (!seconds || !audio.src) {
      void toggleVoice(card, applyPeaks);
      return;
    }
    const rect = wave.getBoundingClientRect();
    const ratio = rect.width > 0 ? Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width)) : 0;
    const rtl = document.documentElement.dir === "rtl";
    audio.currentTime = seconds * (rtl ? 1 - ratio : ratio);
    if (audio.paused) {
      void toggleVoice(card, applyPeaks);
      return;
    }
    paintVoiceBars(wave, bars, audio.currentTime / seconds);
    paintTime();
  });
  length.addEventListener("click", (event) => {
    event.stopPropagation();
    rate = rate === 1 ? 1.5 : rate === 1.5 ? 2 : 1;
    audio.playbackRate = rate;
    paintTime();
  });
  if (src && peaks.length !== VOICE_BARS) {
    void decodePeaks(src).then(applyPeaks);
  }
  return card;
}

async function toggleVoice(card: HTMLElement, applyPeaks: (peaks: number[]) => void): Promise<void> {
  const audio = card.querySelector("audio");
  const play = card.querySelector<HTMLButtonElement>(".voice-play");
  if (!audio) {
    return;
  }
  if (!audio.paused && !audio.ended) {
    audio.pause();
    return;
  }
  if (!audio.src) {
    const id = card.dataset.mediaId || "";
    if (!openId || !messageIdNumber(id)) {
      return;
    }
    if (play) {
      play.disabled = true;
    }
    try {
      const files = await invoke<MediaFile[]>("load_media", { id: openId, messageIds: [id], preview: false, animatedIds: [] });
      const file = files.find((item) => item.id === id) || files[0];
      if (!file?.data) {
        throw new Error(file?.error || t("couldNotDownload"));
      }
      const bytes = new Uint8Array(decodeBase64(file.data));
      const url = URL.createObjectURL(new Blob([bytes], { type: voiceMime(bytes, file.mime || "") }));
      audio.src = url;
      void decodePeaks(url).then(applyPeaks);
    } catch (error) {
      sendError.hidden = false;
      sendError.textContent = error instanceof Error ? error.message : String(error);
      if (play) {
        play.disabled = false;
      }
      return;
    }
    if (play) {
      play.disabled = false;
    }
  }
  if (activeVoice && activeVoice !== audio) {
    activeVoice.pause();
  }
  activeVoice = audio;
  try {
    await audio.play();
  } catch (error) {
    sendError.hidden = false;
    sendError.textContent = error instanceof Error ? error.message : String(error);
  }
}

function imageThumb(url: string, onClick: () => void): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "media-thumb";
  const img = document.createElement("img");
  img.alt = "";
  img.src = url;
  button.append(img);
  button.addEventListener("click", (event) => {
    event.stopPropagation();
    onClick();
  });
  return button;
}

const COLLAPSED_LINES = 15;

type HttpsLink = { start: number; end: number; open: string };
type LinkPreview = { url: string; title: string; description: string; image: string; site: string };

const linkPreviewCache = new Map<string, LinkPreview | null>();
const linkPreviewJobs = new Map<string, Promise<LinkPreview | null>>();

function trimUrlTail(raw: string): string {
  let text = raw;
  while (text.length > "https://".length) {
    const last = text.at(-1) || "";
    if (last === ")") {
      const opens = text.split("(").length - 1;
      const closes = text.split(")").length - 1;
      if (closes <= opens) {
        break;
      }
      text = text.slice(0, -1);
      continue;
    }
    if (last === "]") {
      const opens = text.split("[").length - 1;
      const closes = text.split("]").length - 1;
      if (closes <= opens) {
        break;
      }
      text = text.slice(0, -1);
      continue;
    }
    if (".,;:!?\"'".includes(last)) {
      text = text.slice(0, -1);
      continue;
    }
    break;
  }
  return text;
}

function canonicalHttps(raw: string): string | null {
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    return null;
  }
  if (url.protocol !== "https:") {
    return null;
  }
  if (url.username || url.password) {
    return null;
  }
  const host = url.hostname.replace(/\.$/, "").toLowerCase();
  if (!host || host === "localhost" || host.endsWith(".localhost") || host.endsWith(".local") || host.endsWith(".internal")) {
    return null;
  }
  if (!host.includes(".") && !host.includes(":")) {
    return null;
  }
  return url.href;
}

function httpsLinks(text: string): HttpsLink[] {
  const found: HttpsLink[] = [];
  const re = /https:\/\/[^\s<>"']+/gi;
  let match: RegExpExecArray | null;
  while ((match = re.exec(text)) !== null) {
    const cleaned = trimUrlTail(match[0]);
    const open = canonicalHttps(cleaned);
    if (!open) {
      continue;
    }
    found.push({ start: match.index, end: match.index + cleaned.length, open });
  }
  return found;
}

function soleHttpsUrl(text: string): string | null {
  const trimmed = text.trim();
  const links = httpsLinks(trimmed);
  if (links.length !== 1 || links[0].start !== 0) {
    return null;
  }
  const after = trimmed.slice(links[0].end);
  if (after && !/^[\s.)\],;:!?'"]+$/.test(after)) {
    return null;
  }
  return links[0].open;
}

function openHttps(url: string): void {
  const open = canonicalHttps(url);
  if (!open) {
    return;
  }
  void invoke("open_link", { url: open }).catch((error: unknown) => {
    sendError.hidden = false;
    sendError.textContent = error instanceof Error ? error.message : String(error);
  });
}

function bindExternalLink(anchor: HTMLAnchorElement, url: string): void {
  const open = (event: MouseEvent) => {
    event.preventDefault();
    event.stopPropagation();
    openHttps(url);
  };
  anchor.addEventListener("click", open);
  anchor.addEventListener("auxclick", (event) => {
    if (event.button === 1) {
      open(event);
    }
  });
}

function fillMessageText(body: HTMLElement, text: string): void {
  const links = httpsLinks(text);
  if (links.length === 0) {
    body.textContent = text;
    return;
  }
  let cursor = 0;
  for (const link of links) {
    if (link.start > cursor) {
      body.append(document.createTextNode(text.slice(cursor, link.start)));
    }
    const anchor = document.createElement("a");
    anchor.className = "msg-link";
    anchor.href = link.open;
    anchor.rel = "noopener noreferrer";
    anchor.textContent = text.slice(link.start, link.end);
    bindExternalLink(anchor, link.open);
    body.append(anchor);
    cursor = link.end;
  }
  if (cursor < text.length) {
    body.append(document.createTextNode(text.slice(cursor)));
  }
}

function paintLinkCard(card: HTMLAnchorElement, preview: LinkPreview): void {
  card.hidden = false;
  card.replaceChildren();
  if (preview.image.startsWith("https://")) {
    const img = document.createElement("img");
    img.alt = "";
    img.src = preview.image;
    img.addEventListener("error", () => img.remove());
    card.append(img);
  }
  const copy = document.createElement("span");
  copy.className = "link-card-copy";
  if (preview.title) {
    const title = document.createElement("strong");
    title.className = "link-card-title";
    title.textContent = preview.title;
    copy.append(title);
  }
  if (preview.description) {
    const desc = document.createElement("span");
    desc.className = "link-card-desc";
    desc.textContent = preview.description;
    copy.append(desc);
  }
  const site = document.createElement("span");
  site.className = "link-card-site";
  site.textContent = preview.site || "";
  copy.append(site);
  card.append(copy);
}

function showLinkCard(card: HTMLAnchorElement, url: string): void {
  const apply = (preview: LinkPreview | null) => {
    if (!preview || !card.isConnected || (!preview.title && !preview.image)) {
      return;
    }
    paintLinkCard(card, preview);
  };
  const cached = linkPreviewCache.get(url);
  if (cached !== undefined) {
    apply(cached);
    return;
  }
  let job = linkPreviewJobs.get(url);
  if (!job) {
    job = invoke<LinkPreview>("link_preview", { url })
      .then((preview) => {
        const stored = preview.title || preview.image ? preview : null;
        linkPreviewCache.set(url, stored);
        return stored;
      })
      .catch(() => {
        linkPreviewCache.set(url, null);
        return null;
      })
      .finally(() => {
        linkPreviewJobs.delete(url);
      });
    linkPreviewJobs.set(url, job);
  }
  void job.then(apply);
}

function appendMessageText(bubble: HTMLElement, text: string): void {
  const callLabel = callPreviewLabel(text);
  const body = document.createElement("div");
  body.className = "msg-text";
  if (callLabel) {
    body.classList.add("call");
    body.append(phoneIcon(), document.createTextNode(callLabel));
  } else {
    fillMessageText(body, text);
  }
  bubble.append(body);
  const sole = callLabel ? null : soleHttpsUrl(text);
  if (sole) {
    const card = document.createElement("a");
    card.className = "link-card";
    card.hidden = true;
    card.href = sole;
    card.rel = "noopener noreferrer";
    bindExternalLink(card, sole);
    bubble.append(card);
    showLinkCard(card, sole);
  }
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
  if (message.kind === "audio") {
    return voiceQuote(message.text);
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

function voiceQuote(text: string): string {
  const name = text.trim();
  if (name && !/^(audio|voice message|voice\.m4a)$/i.test(name)) {
    return name;
  }
  return t("voiceMessage");
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
      replyThumb: message.replyThumb || "",
      replyMediaId: message.replyMediaId || imageReplyId(source),
    };
  });
}

function imageReplyId(source: HistoryMessage | undefined): string {
  if (!source || source.kind !== "image") {
    return "";
  }
  const id = source.mediaId || source.id || "";
  return messageIdNumber(id) ? id : "";
}

function showReplyThumb(img: HTMLImageElement, url: string, mediaId: string): void {
  const show = (src: string) => {
    if (!src || !img.isConnected) {
      return;
    }
    img.hidden = false;
    img.src = src;
  };
  if (url) {
    show(url);
  }
  if (!mediaId || !messageIdNumber(mediaId)) {
    return;
  }
  void warmMedia([mediaId], true).then(() => {
    const key = mediaKey(mediaId, true);
    const mime = mediaMimes.get(key) || "";
    const found = mediaUrls.get(key) || "";
    if (found && !mime.startsWith("video/")) {
      show(found);
    }
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
  const quotedId = message.replyId;
  quote.tabIndex = 0;
  quote.setAttribute("role", "button");
  const showOriginal = () => revealQuotedMessage(quotedId);
  quote.addEventListener("click", (event) => {
    event.stopPropagation();
    showOriginal();
  });
  quote.addEventListener("keydown", (event) => {
    if (event.key !== "Enter" && event.key !== " ") {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    showOriginal();
  });
  const thumbUrl = message.replyThumb || "";
  const thumbId = message.replyMediaId && messageIdNumber(message.replyMediaId) ? message.replyMediaId : "";
  if (thumbUrl || thumbId) {
    const img = document.createElement("img");
    img.className = "reply-thumb";
    img.alt = "";
    img.hidden = true;
    img.addEventListener("error", () => {
      img.hidden = true;
      img.removeAttribute("src");
    });
    quote.append(img);
    showReplyThumb(img, thumbUrl, thumbId);
  }
  const name = bubble.querySelector(".who");
  if (name) {
    name.after(quote);
  } else {
    bubble.prepend(quote);
  }
}

function quotedMessageRow(id: string): HTMLElement | null {
  if (!id) {
    return null;
  }
  const direct = messageRow(id);
  if (direct) {
    return direct;
  }
  const rows = threadBody.querySelectorAll<HTMLElement>(".msg[data-id], .chat-update[data-id]");
  for (const row of rows) {
    if (row.dataset.id === id) {
      return row;
    }
    const ids = (row.dataset.ids || "").split(",");
    if (ids.includes(id)) {
      return row;
    }
  }
  return null;
}

function revealQuotedMessage(id: string): void {
  const row = quotedMessageRow(id);
  if (!row) {
    return;
  }
  const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  row.scrollIntoView({ block: "center", behavior: reduce ? "auto" : "smooth" });
  threadBody.querySelectorAll(".jump-target").forEach((item) => item.classList.remove("jump-target"));
  row.classList.remove("jump-target");
  void row.offsetWidth;
  row.classList.add("jump-target");
  const clear = () => {
    row.classList.remove("jump-target");
    row.removeEventListener("animationend", clear);
  };
  row.addEventListener("animationend", clear);
}

function rowImageQuote(row: HTMLElement): { url: string; mediaId: string } {
  const thumbs = [...row.querySelectorAll<HTMLElement>(".media-thumb")];
  const preferred = thumbs.find((thumb) => thumb.dataset.id && thumb.dataset.id === row.dataset.id) || thumbs[0];
  const frame = preferred?.querySelector<HTMLImageElement | HTMLVideoElement>("img, video")
    || row.querySelector<HTMLImageElement>("img.outgoing-media");
  if (!frame) {
    return { url: "", mediaId: "" };
  }
  const raw = frame instanceof HTMLImageElement ? (frame.currentSrc || frame.src) : (frame.poster || "");
  const url = raw && raw !== window.location.href ? raw : "";
  const mediaId = preferred?.dataset.id || "";
  return { url, mediaId: messageIdNumber(mediaId) ? mediaId : "" };
}

function quoteText(row: HTMLElement): string {
  const text = row.querySelector(".msg-text")?.textContent?.trim() || "";
  if (text && !mediaLabel(text)) {
    return text;
  }
  if (row.querySelector(".sticker")) {
    return t("sticker");
  }
  if (row.querySelector(".video-stack, .chat-video")) {
    return t("video");
  }
  if (row.querySelector(".media-grid")) {
    return t("photo");
  }
  const file = row.querySelector(".file-name")?.textContent?.trim() || "";
  if (file) {
    return file;
  }
  const voice = row.querySelector<HTMLElement>(".voice-card");
  if (voice) {
    return voiceQuote(voice.dataset.label || "");
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
    replyThumb.hidden = true;
    replyThumb.removeAttribute("src");
    return;
  }
  paintSmallAvatar(replyAvatar, replyDraft.picture, replyDraft.name);
  replyLabel.textContent = replyDraft.name;
  replyText.textContent = replyDraft.text;
  replyThumb.hidden = true;
  replyThumb.removeAttribute("src");
  showReplyThumb(replyThumb, replyDraft.thumb || "", replyDraft.mediaId || "");
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
  closeSendMenu();
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
  sheetAgent.hidden = true;
  sheetChatLlm.hidden = true;
  sheetProfile.hidden = true;
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
  sheetAgent.hidden = true;
  sheetChatLlm.hidden = true;
  sheetProfile.hidden = true;
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
  } else if (message.kind === "audio" && (message.mediaId || message.id)) {
    bubble.append(voiceCard(message.mediaId || message.id || "", message.duration || 0, "", message.text));
  } else if (message.kind === "file" && message.id) {
    const name = message.text;
    const id = message.id;
    bubble.append(fileCard(name, (button) => {
      void downloadMessageFile(id, name, button);
    }));
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
    button.dataset.id = item.id;
    const img = document.createElement("img");
    img.alt = "";
    button.append(img);
    button.addEventListener("click", (event) => {
      event.stopPropagation();
      openGallery(ids, index);
    });
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
  localGallery = null;
  gallery = { ids, index };
  lightbox.hidden = false;
  paintGallery();
}

function openLocalGallery(urls: string[], index: number): void {
  const shown = urls.filter((url) => url);
  if (shown.length === 0) {
    return;
  }
  gallery = null;
  localGallery = { urls: shown, index: Math.min(Math.max(index, 0), shown.length - 1) };
  lightbox.hidden = false;
  paintLocalGallery();
}

function paintLocalGallery(): void {
  if (!localGallery) {
    return;
  }
  paintLightboxChrome();
  showLightbox(localGallery.urls[localGallery.index] || "", "");
}

function closeGallery(): void {
  gallery = null;
  localGallery = null;
  lightbox.hidden = true;
  lightboxImg.removeAttribute("src");
  lightboxVideo.pause();
  lightboxVideo.removeAttribute("src");
  delete lightboxVideo.dataset.file;
  lightboxVideo.hidden = true;
  lightboxImg.hidden = false;
}

function paintLightboxChrome(): void {
  const count = localGallery?.urls.length ?? gallery?.ids.length ?? 0;
  const index = localGallery?.index ?? gallery?.index ?? 0;
  const several = count > 1;
  lightboxNav.hidden = !several;
  if (several) {
    lightboxCount.textContent = t("galleryPosition", { current: index + 1, total: count });
  }
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
  if (localGallery && localGallery.urls.length > 1) {
    const count = localGallery.urls.length;
    localGallery.index = (localGallery.index + delta + count) % count;
    paintLocalGallery();
    return;
  }
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
  paintLightboxChrome();
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
  if (railMode === "chats") {
    showThread(shown[selected]);
  }
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
  if (total === 0) {
    releaseAttention();
  }
  void invoke("set_tray_badge", { count: total }).catch(() => {
    trayBadge = -1;
  });
}

let attentionOn = false;

function requestAttention(): void {
  if (attentionOn || document.hasFocus() || focusOnMessage) {
    return;
  }
  attentionOn = true;
  void invoke("attention_for_message").catch(() => {
    attentionOn = false;
  });
}

function releaseAttention(): void {
  if (!attentionOn) {
    return;
  }
  attentionOn = false;
  void invoke("clear_attention").catch(() => undefined);
}

function renderChats(payload: ChatPayload): void {
  const keepId = signedIn ? (openId || visibleRows()[selected]?.id || "") : "";
  signedIn = true;
  ensureAgentProbe();
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
  if (typeof profile.status === "string") {
    myStatus = profile.status;
  }
  if (profile.mid) {
    myMid = profile.mid;
  }
  paintRailMe();
  paintProfileCurrent();
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

function directoryError(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  if (message.includes("No one was found")) {
    return t("nobodyFound");
  }
  if (message.includes("Could not search") || message.includes("could not search")) {
    return t("couldNotSearch");
  }
  if (message.includes("could not create") || message.includes("LINE could not create")) {
    return t("couldNotCreateGroup");
  }
  return message;
}

function recommendationText(reason: string | undefined): string {
  switch (reason) {
    case "phone":
      return t("addedByPhone");
    case "id":
      return t("addedById");
    case "qr":
      return t("addedByQr");
    case "group":
      return t("addedByGroup");
    case "share":
      return t("addedByShare");
    case "added":
      return t("addedYou");
    default:
      return "";
  }
}

function personFace(name: string, picture: string): HTMLElement {
  if (!picture) {
    return fallback(name);
  }
  const photo = document.createElement("img");
  photo.className = "avatar";
  photo.alt = "";
  photo.src = picture;
  photo.addEventListener("error", () => photo.replaceWith(fallback(name)));
  return photo;
}

function syncChatTabs(): void {
  document.querySelectorAll<HTMLButtonElement>("#tabs button").forEach((tab) => {
    tab.classList.toggle("on", (tab.dataset.filter || "all") === filter);
  });
}

function rememberChat(chat: ChatRow): void {
  if (!rows.some((row) => row.id === chat.id)) {
    rows.unshift(chat);
  }
  search.value = "";
  filter = "all";
  syncChatTabs();
  const index = visibleRows().findIndex((row) => row.id === chat.id);
  selected = index >= 0 ? index : 0;
}

function showPersonChat(person: DirectoryPerson): void {
  let chat = rows.find((row) => row.id === person.id);
  if (!chat) {
    chat = {
      id: person.id,
      name: person.name || t("directChat"),
      lastMessage: "",
      sealed: false,
      time: Date.now(),
      members: null,
      picture: person.picture,
      muted: false,
      kind: "friend",
      unread: null,
    };
  } else if (person.name) {
    chat.name = person.name;
    if (person.picture) {
      chat.picture = person.picture;
    }
  }
  rememberChat(chat);
  showThread(chat);
  paintPeople();
  composerInput.focus();
}

function paintPeople(): void {
  const needle = peopleQuery.value.trim().toLowerCase();
  const shown = directoryPeople.filter((person) => !needle || person.name.toLowerCase().includes(needle));
  peopleList.replaceChildren();
  peopleNote.hidden = true;
  if (!peopleReady) {
    peopleNote.hidden = false;
    peopleNote.textContent = peopleError || t("loadingFriends");
    return;
  }
  if (directoryPeople.length === 0) {
    peopleNote.hidden = false;
    peopleNote.textContent = t("noFriendsYet");
    return;
  }
  if (shown.length === 0) {
    peopleNote.hidden = false;
    peopleNote.textContent = t("noFriendsNamed");
    return;
  }
  const favorites = shown.filter((person) => person.favorite);
  const rest = shown.filter((person) => !person.favorite);
  if (favorites.length > 0) {
    peopleList.append(peopleHeading(t("favorites")));
    favorites.forEach((person) => peopleList.append(peopleItem(person)));
  }
  let letter = "";
  rest.forEach((person) => {
    const next = (person.name.trim()[0] || "#").toLocaleUpperCase();
    if (next !== letter) {
      letter = next;
      peopleList.append(peopleHeading(next));
    }
    peopleList.append(peopleItem(person));
  });
}

function peopleHeading(label: string): HTMLElement {
  const item = document.createElement("li");
  item.className = "people-letter";
  item.textContent = label;
  return item;
}

function peopleItem(person: DirectoryPerson): HTMLElement {
  const item = document.createElement("li");
  const button = document.createElement("button");
  button.type = "button";
  button.className = "people-row";
  if (person.id === openId) {
    button.classList.add("selected");
  }
  button.append(personFace(person.name, person.picture));
  const copy = document.createElement("span");
  copy.className = "people-copy";
  const name = document.createElement("strong");
  name.textContent = person.name;
  copy.append(name);
  button.append(copy);
  button.addEventListener("click", () => showPersonChat(person));
  item.append(button);
  return item;
}

async function loadPeople(): Promise<void> {
  const token = ++peopleToken;
  peopleError = "";
  if (!peopleReady) {
    paintPeople();
  }
  try {
    const loaded = await invoke<DirectoryPerson[]>("list_friends");
    if (token !== peopleToken) {
      return;
    }
    directoryPeople = Array.isArray(loaded) ? loaded : [];
    peopleReady = true;
    peopleError = "";
    paintPeople();
    if (!addGroupView.hidden) {
      paintGroupPicks();
    }
  } catch (error) {
    if (token !== peopleToken || peopleReady) {
      return;
    }
    peopleError = directoryError(error);
    paintPeople();
  }
}

function paintRecommendations(): void {
  addRecsToggle.classList.toggle("is-shut", !recsOpen);
  addRecsToggle.setAttribute("aria-expanded", recsOpen ? "true" : "false");
  addRecs.hidden = !recsOpen;
  addRecs.replaceChildren();
  addRecsNote.hidden = true;
  const count = recsReady && !recsError ? ` ${recommendations.length}` : "";
  addRecsLabel.textContent = `${t("friendRecommendations")}${count}`;
  if (!recsOpen) {
    return;
  }
  if (recsError) {
    addRecsNote.hidden = false;
    addRecsNote.textContent = recsError;
    return;
  }
  if (!recsReady) {
    addRecsNote.hidden = false;
    addRecsNote.textContent = t("loadingRecommendations");
    return;
  }
  if (recommendations.length === 0) {
    addRecsNote.hidden = false;
    addRecsNote.textContent = t("noRecommendations");
    return;
  }
  recommendations.forEach((person) => {
    const item = document.createElement("li");
    const row = document.createElement("div");
    row.className = "rec-row";
    row.append(personFace(person.name, person.picture));
    const copy = document.createElement("div");
    copy.className = "rec-copy";
    const name = document.createElement("strong");
    name.textContent = person.name;
    copy.append(name);
    const reason = recommendationText(person.reason);
    if (reason) {
      const sub = document.createElement("span");
      sub.textContent = reason;
      copy.append(sub);
    }
    const add = document.createElement("button");
    add.type = "button";
    add.className = "rec-add";
    add.title = t("addFriend");
    add.setAttribute("aria-label", t("addFriend"));
    add.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="10" cy="8" r="3"/><path d="M4.5 18.5c.6-2.6 2.8-4 5.5-4s4.9 1.4 5.5 4"/><path d="M17 8v6M14 11h6"/></svg>';
    add.addEventListener("click", () => void addRecommended(person, add));
    row.addEventListener("click", (event) => {
      if ((event.target as HTMLElement).closest(".rec-add")) {
        return;
      }
      void invoke("open_profile", {
        id: person.id,
        name: person.name,
        picture: person.picture,
        mine: false,
        friend: false,
      }).catch((error: unknown) => {
        addRecsNote.hidden = false;
        addRecsNote.textContent = directoryError(error);
      });
    });
    row.append(copy, add);
    item.append(row);
    addRecs.append(item);
  });
}

async function addRecommended(person: Recommendation, button: HTMLButtonElement): Promise<void> {
  button.disabled = true;
  try {
    await invoke("add_profile", { id: person.id });
    recommendations = recommendations.filter((item) => item.id !== person.id);
    directoryPeople = [];
    paintRecommendations();
    void loadPeople();
  } catch (error) {
    button.disabled = false;
    addRecsNote.hidden = false;
    addRecsNote.textContent = directoryError(error);
  }
}

async function loadRecommendations(): Promise<void> {
  const token = ++recsToken;
  if (!recsReady) {
    paintRecommendations();
  }
  try {
    const loaded = await invoke<Recommendation[]>("friend_recommendations");
    if (token !== recsToken) {
      return;
    }
    recommendations = Array.isArray(loaded) ? loaded : [];
    recsReady = true;
    recsError = "";
    paintRecommendations();
  } catch (error) {
    if (token !== recsToken) {
      return;
    }
    recsReady = true;
    recsError = directoryError(error);
    paintRecommendations();
  }
}

function showAddScreen(screen: "home" | "search" | "group"): void {
  addHome.hidden = screen !== "home";
  addSearchView.hidden = screen !== "search";
  addGroupView.hidden = screen !== "group";
  if (screen === "search") {
    addSearchNote.hidden = true;
    addSearchQuery.focus();
  }
  if (screen === "group") {
    addGroupNote.hidden = true;
    paintGroupPicks();
    void loadPeople();
  }
}

function paintGroupPicks(): void {
  const needle = addGroupQuery.value.trim().toLowerCase();
  const shown = directoryPeople.filter((person) => !needle || person.name.toLowerCase().includes(needle));
  addGroupList.replaceChildren();
  addGroupCreate.disabled = groupPicked.size === 0;
  addGroupNote.hidden = shown.length > 0;
  if (shown.length === 0) {
    addGroupNote.textContent = directoryPeople.length === 0 ? (peopleReady ? t("noFriendsYet") : t("loadingFriends")) : t("noFriendsNamed");
  }
  shown.forEach((person) => {
    const item = document.createElement("li");
    const button = document.createElement("button");
    button.type = "button";
    button.className = groupPicked.has(person.id) ? "group-pick is-on" : "group-pick";
    button.setAttribute("aria-pressed", groupPicked.has(person.id) ? "true" : "false");
    button.append(personFace(person.name, person.picture));
    const name = document.createElement("span");
    name.textContent = person.name;
    const box = document.createElement("span");
    box.className = "pick-box";
    button.append(name, box);
    button.addEventListener("click", () => {
      if (groupPicked.has(person.id)) {
        groupPicked.delete(person.id);
      } else {
        groupPicked.add(person.id);
      }
      paintGroupPicks();
    });
    item.append(button);
    addGroupList.append(item);
  });
}

function paintSearchHit(person: DirectoryPerson): void {
  addSearchResult.replaceChildren();
  const row = document.createElement("div");
  row.className = "search-hit";
  row.append(personFace(person.name, person.picture));
  const copy = document.createElement("div");
  copy.className = "rec-copy";
  const name = document.createElement("strong");
  name.textContent = person.name;
  copy.append(name);
  if (person.status) {
    const status = document.createElement("span");
    status.textContent = person.status;
    copy.append(status);
  }
  const action = document.createElement("button");
  action.type = "button";
  action.className = "rec-add";
  action.title = person.friend ? t("chat") : t("addFriend");
  action.setAttribute("aria-label", action.title);
  action.textContent = person.friend ? t("chat") : "+";
  action.addEventListener("click", () => {
    if (person.friend) {
      showPersonChat(person);
      setRail("chats");
      return;
    }
    action.disabled = true;
    void invoke("add_profile", { id: person.id })
      .then(() => {
        person.friend = true;
        addSearchNote.hidden = false;
        addSearchNote.textContent = t("added");
        directoryPeople = [];
        void loadPeople();
        paintSearchHit(person);
      })
      .catch((error: unknown) => {
        action.disabled = false;
        addSearchNote.hidden = false;
        addSearchNote.textContent = directoryError(error);
      });
  });
  row.append(copy, action);
  addSearchResult.append(row);
}

const AGENT_LOG_KEY = "lineapp.agentLog";
type AgentTurn = { role: "user" | "assistant" | "error"; text: string; calls?: string[] };
type AgentConfig = {
  endpoint: string;
  model: string;
  apiKey: string;
  protocol: string;
  temperature: string;
  topP: string;
  maxTokens: string;
  thinking: string;
  extra: string;
  stream: boolean;
  autoContext: string;
  replyTemperature: string;
  replyTopP: string;
  replyMaxTokens: string;
  replyThinking: string;
  replyExtra: string;
};
const EMPTY_AGENT: AgentConfig = {
  endpoint: "",
  model: "",
  apiKey: "",
  protocol: "openai",
  temperature: "",
  topP: "",
  maxTokens: "",
  thinking: "default",
  extra: "",
  stream: true,
  autoContext: "",
  replyTemperature: "",
  replyTopP: "",
  replyMaxTokens: "",
  replyThinking: "off",
  replyExtra: "",
};
const AGENT_CALL_KEYS: Record<string, MessageKey> = {
  search_conversations: "agentSearched",
  get_messages: "agentRetrieved",
  latest_messages: "agentLatest",
  list_chats: "agentListed",
};
let agentTurns: AgentTurn[] = [];
let agentBusy = false;
let agentLiveText = "";
let agentLiveCalls: string[] = [];
let agentConfig: AgentConfig = { ...EMPTY_AGENT };
let agentSettingsFromAgent = false;

function loadAgentLog(): void {
  agentTurns = [];
  try {
    const parsed = JSON.parse(localStorage.getItem(AGENT_LOG_KEY) || "[]") as unknown;
    if (!Array.isArray(parsed)) {
      return;
    }
    for (const item of parsed) {
      if (!item || typeof item !== "object") {
        continue;
      }
      const turn = item as Partial<AgentTurn>;
      if ((turn.role !== "user" && turn.role !== "assistant" && turn.role !== "error") || typeof turn.text !== "string") {
        continue;
      }
      const calls = Array.isArray(turn.calls) ? turn.calls.filter((call): call is string => typeof call === "string") : undefined;
      agentTurns.push({ role: turn.role, text: turn.text, calls });
    }
    agentTurns = agentTurns.slice(-40);
  } catch {
    agentTurns = [];
  }
}

function saveAgentLog(): void {
  localStorage.setItem(AGENT_LOG_KEY, JSON.stringify(agentTurns.slice(-40)));
}

function resetAgentLive(): void {
  agentLiveText = "";
  agentLiveCalls = [];
}

function clearAgentLog(): void {
  agentTurns = [];
  agentBusy = false;
  resetAgentLive();
  localStorage.removeItem(AGENT_LOG_KEY);
  if (railMode === "agent") {
    paintAgent();
  }
}

let autoReplyState: Record<string, boolean> = {};

loadAgentLog();
loadAutoReply();

function agentEndpointSummary(endpoint: string): string {
  const trimmed = endpoint.trim();
  if (!trimmed) {
    return t("agentNotSet");
  }
  try {
    return new URL(trimmed).host || trimmed;
  } catch {
    return trimmed.length > 36 ? `${trimmed.slice(0, 36)}…` : trimmed;
  }
}

function paintAgentCurrent(): void {
  agentCurrent.textContent = agentEndpointSummary(agentConfig.endpoint);
}

function syncAgentSend(): void {
  agentSend.disabled = agentBusy || agentInput.value.trim().length === 0;
}

function appendAgentCalls(names: string[]): void {
  if (names.length === 0) {
    return;
  }
  const calls = document.createElement("div");
  calls.className = "agent-calls";
  const seen = new Set<string>();
  for (const name of names) {
    if (seen.has(name)) {
      continue;
    }
    seen.add(name);
    const chip = document.createElement("span");
    const key = AGENT_CALL_KEYS[name];
    chip.textContent = key ? t(key) : name.replace(/_/g, " ");
    calls.append(chip);
  }
  agentLog.append(calls);
}

function paintAgent(): void {
  agentLog.replaceChildren();
  if (agentTurns.length === 0 && !agentBusy) {
    const empty = document.createElement("p");
    empty.className = "agent-empty";
    empty.textContent = agentConfig.endpoint.trim() ? t("agentAsk") : t("agentNeedsEndpoint");
    agentLog.append(empty);
  }
  for (const turn of agentTurns) {
    appendAgentCalls(turn.calls ?? []);
    const bubble = document.createElement("div");
    bubble.className = `agent-bubble ${turn.role}`;
    bubble.textContent = turn.text;
    agentLog.append(bubble);
  }
  if (agentBusy && (agentLiveText || agentLiveCalls.length > 0)) {
    appendAgentCalls(agentLiveCalls);
    if (agentLiveText) {
      const bubble = document.createElement("div");
      bubble.className = "agent-bubble assistant";
      bubble.textContent = agentLiveText;
      agentLog.append(bubble);
    }
  }
  if (agentBusy && !agentLiveText) {
    const thinking = document.createElement("p");
    thinking.className = "agent-thinking";
    thinking.textContent = t("agentThinkingWait");
    agentLog.append(thinking);
  }
  agentLog.scrollTop = agentLog.scrollHeight;
  syncAgentSend();
}

function dismissAgentSettings(): void {
  sheetAgent.hidden = true;
  sheetChatLlm.hidden = true;
  sheetProfile.hidden = true;
  agentSettingsNote.hidden = true;
  if (agentSettingsFromAgent) {
    agentSettingsFromAgent = false;
    sheet.hidden = true;
    return;
  }
  sheetSettings.hidden = false;
  sheetTitle.textContent = t("settings");
}

async function openAgent(): Promise<void> {
  try {
    agentConfig = { ...EMPTY_AGENT, ...(await invoke<AgentConfig>("get_agent_settings")) };
  } catch {
    agentConfig = { ...EMPTY_AGENT };
  }
  setRail("agent");
  if (!agentConfig.endpoint.trim()) {
    agentSettingsFromAgent = true;
    await showAgentSettings();
    agentEndpoint.focus();
  }
}

async function askAgent(): Promise<void> {
  const text = agentInput.value.trim();
  if (!text || agentBusy) {
    return;
  }
  agentBusy = true;
  resetAgentLive();
  agentInput.value = "";
  agentTurns.push({ role: "user", text });
  paintAgent();
  try {
    const reply = await invoke<{ text: string; calls: string[] }>("agent_ask", {
      messages: agentTurns
        .filter((turn) => turn.role === "user" || turn.role === "assistant")
        .map((turn) => ({ role: turn.role, content: turn.text })),
    });
    agentTurns.push({ role: "assistant", text: reply.text, calls: reply.calls });
  } catch (error) {
    agentTurns.push({ role: "error", text: error instanceof Error ? error.message : String(error) });
  }
  agentBusy = false;
  resetAgentLive();
  agentTurns = agentTurns.slice(-40);
  saveAgentLog();
  paintAgent();
}

type McpServer = { url: string; enabled: boolean };
let mcpServers: McpServer[] = [];

function mcpSections(): HTMLElement[] {
  return [...document.querySelectorAll<HTMLElement>("[data-mcp]")];
}

function paintMcp(): void {
  for (const section of mcpSections()) {
    const list = section.querySelector<HTMLElement>("[data-mcp-list]");
    if (!list) {
      continue;
    }
    list.replaceChildren();
    mcpServers.forEach((server, index) => {
      const item = document.createElement("li");
      item.className = "mcp-row";
      const toggle = document.createElement("input");
      toggle.type = "checkbox";
      toggle.checked = server.enabled;
      toggle.dataset.i18nAria = "mcpEnabled";
      toggle.setAttribute("aria-label", t("mcpEnabled"));
      toggle.addEventListener("change", () => {
        const next = mcpServers.map((entry, entryIndex) => entryIndex === index ? { ...entry, enabled: toggle.checked } : entry);
        void commitMcp(next, section);
      });
      const url = document.createElement("input");
      url.type = "url";
      url.value = server.url;
      url.spellcheck = false;
      url.autocomplete = "off";
      url.addEventListener("change", () => {
        const next = mcpServers.map((entry, entryIndex) => entryIndex === index ? { ...entry, url: url.value.trim() } : entry);
        void commitMcp(next, section);
      });
      const remove = document.createElement("button");
      remove.type = "button";
      remove.dataset.i18n = "mcpRemove";
      remove.textContent = t("mcpRemove");
      remove.addEventListener("click", () => {
        void commitMcp(mcpServers.filter((_, entryIndex) => entryIndex !== index), section);
      });
      item.append(toggle, url, remove);
      list.append(item);
    });
  }
}

type ListedTool = { name: string; description: string };
type ListedServer = { url: string; ok: boolean; error?: string; tools?: ListedTool[] };
type ToolCatalog = { chats?: ListedTool[]; line?: ListedTool[]; servers?: ListedServer[] };

function isListedTool(value: ListedTool | undefined): value is ListedTool {
  return !!value && typeof value.name === "string" && typeof value.description === "string";
}

function toolRow(tool: ListedTool): HTMLElement {
  const row = document.createElement("div");
  row.className = "tool-row";
  const name = document.createElement("strong");
  name.textContent = tool.name;
  const description = document.createElement("span");
  description.textContent = tool.description;
  row.append(name, description);
  return row;
}

function toolGroup(titleKey: MessageKey, tools: ListedTool[]): HTMLElement {
  const block = document.createElement("section");
  const heading = document.createElement("h4");
  heading.dataset.i18n = titleKey;
  heading.textContent = t(titleKey);
  block.append(heading);
  for (const tool of tools) {
    block.append(toolRow(tool));
  }
  return block;
}

async function loadAgentTools(): Promise<void> {
  const list = document.querySelector<HTMLElement>("[data-agent-tools-list]");
  const note = document.querySelector<HTMLElement>("[data-agent-tools-note]");
  if (!list || !note) {
    return;
  }
  note.hidden = false;
  note.textContent = t("agentToolsLoading");
  list.replaceChildren();
  try {
    const catalog = await invoke<ToolCatalog>("list_agent_tools");
    note.hidden = true;
    const chats = (catalog.chats ?? []).filter(isListedTool);
    const line = (catalog.line ?? []).filter(isListedTool);
    const servers = catalog.servers ?? [];
    list.append(toolGroup("agentToolsChats", chats));
    list.append(toolGroup("agentToolsLine", line));
    const remote = document.createElement("section");
    const heading = document.createElement("h4");
    heading.dataset.i18n = "agentToolsRemote";
    heading.textContent = t("agentToolsRemote");
    remote.append(heading);
    if (servers.length === 0) {
      const empty = document.createElement("p");
      empty.className = "tool-server";
      empty.dataset.i18n = "agentToolsNone";
      empty.textContent = t("agentToolsNone");
      remote.append(empty);
    }
    for (const server of servers) {
      if (!server || typeof server.url !== "string") {
        continue;
      }
      const label = document.createElement("p");
      label.className = "tool-server";
      const problem = server.ok ? "" : server.error || "";
      label.textContent = problem ? `${server.url} — ${problem}` : server.url;
      remote.append(label);
      const tools = (server.tools ?? []).filter(isListedTool);
      for (const tool of tools) {
        remote.append(toolRow(tool));
      }
    }
    list.append(remote);
  } catch (error) {
    note.hidden = false;
    note.textContent = error instanceof Error ? error.message : String(error);
  }
}

async function loadMcp(): Promise<void> {
  try {
    mcpServers = await invoke<McpServer[]>("get_mcp_servers");
    for (const section of mcpSections()) {
      const note = section.querySelector<HTMLElement>("[data-mcp-note]");
      if (note) {
        note.hidden = true;
      }
    }
    paintMcp();
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    for (const section of mcpSections()) {
      const note = section.querySelector<HTMLElement>("[data-mcp-note]");
      if (note) {
        note.hidden = false;
        note.textContent = message;
      }
    }
  }
}

async function commitMcp(next: McpServer[], section: HTMLElement): Promise<void> {
  const note = section.querySelector<HTMLElement>("[data-mcp-note]");
  try {
    await invoke("set_mcp_servers", { servers: next });
    mcpServers = next;
    if (note) {
      note.hidden = true;
    }
    paintMcp();
    if (!sheetAgent.hidden) {
      void loadAgentTools();
    }
  } catch (error) {
    if (note) {
      note.hidden = false;
      note.textContent = error instanceof Error ? error.message : String(error);
    }
    paintMcp();
  }
}

function addMcp(section: HTMLElement): void {
  const input = section.querySelector<HTMLInputElement>("[data-mcp-url]");
  const url = input?.value.trim() || "";
  void commitMcp([...mcpServers, { url, enabled: true }], section).then(() => {
    const note = section.querySelector<HTMLElement>("[data-mcp-note]");
    if (input && note?.hidden) {
      input.value = "";
    }
  });
}

for (const section of document.querySelectorAll<HTMLElement>("[data-mcp]")) {
  section.querySelector<HTMLButtonElement>("[data-mcp-add]")?.addEventListener("click", () => {
    addMcp(section);
  });
  section.querySelector<HTMLInputElement>("[data-mcp-url]")?.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      addMcp(section);
    }
  });
}

async function showAgentSettings(): Promise<void> {
  sheetTitle.textContent = t("agent");
  sheetBody.hidden = true;
  sheetSettings.hidden = true;
  sheetList.hidden = true;
  sheetAbout.hidden = true;
  sheetLanguage.hidden = true;
  sheetAgent.hidden = false;
  sheetChatLlm.hidden = true;
  sheetProfile.hidden = true;
  agentSettingsNote.hidden = true;
  sheet.hidden = false;
  try {
    agentConfig = { ...EMPTY_AGENT, ...(await invoke<AgentConfig>("get_agent_settings")) };
  } catch (error) {
    agentSettingsNote.hidden = false;
    agentSettingsNote.textContent = error instanceof Error ? error.message : String(error);
  }
  agentEndpoint.value = agentConfig.endpoint;
  agentModel.value = agentConfig.model;
  agentApiKey.value = agentConfig.apiKey;
  agentProtocol.value = agentConfig.protocol === "anthropic" || agentConfig.protocol === "gemini" ? agentConfig.protocol : "openai";
  agentTemperature.value = agentConfig.temperature;
  agentTopP.value = agentConfig.topP;
  agentMaxTokens.value = agentConfig.maxTokens;
  agentThinking.value = ["default", "off", "low", "medium", "high"].includes(agentConfig.thinking) ? agentConfig.thinking : "default";
  agentStream.checked = agentConfig.stream !== false;
  agentExtra.value = agentConfig.extra;
  replyTemperature.value = agentConfig.replyTemperature || "";
  replyTopP.value = agentConfig.replyTopP || "";
  replyMaxTokens.value = agentConfig.replyMaxTokens || "";
  replyThinking.value = ["default", "off", "low", "medium", "high"].includes(agentConfig.replyThinking) ? agentConfig.replyThinking : "off";
  agentAutoContext.value = agentConfig.autoContext || "";
  replyExtra.value = agentConfig.replyExtra || "";
  paintAgentCurrent();
  void loadMcp();
  void loadAgentTools();
}

function setRail(mode: RailMode): void {
  railMode = mode;
  railPeople.classList.toggle("on", mode === "friends");
  railChats.classList.toggle("on", mode === "chats");
  railFriends.classList.toggle("on", mode === "add");
  railAgent.classList.toggle("on", mode === "agent");
  const chatsOn = mode === "chats";
  const agentOn = mode === "agent";
  listPane.hidden = agentOn;
  paneSplit.hidden = agentOn;
  agentPane.hidden = !agentOn;
  document.querySelector<HTMLElement>("#tabs")!.hidden = !chatsOn;
  document.querySelector<HTMLElement>("#search-wrap")!.hidden = !chatsOn;
  chats.hidden = !chatsOn;
  if (chatsOn) {
    failures.hidden = failures.textContent.trim().length === 0;
  } else {
    failures.hidden = true;
  }
  peoplePane.hidden = mode !== "friends";
  addPane.hidden = mode !== "add";
  if (agentOn) {
    thread.hidden = true;
    stage.hidden = true;
    paintAgent();
    agentInput.focus();
    return;
  }
  if (mode === "chats") {
    if (openId) {
      thread.hidden = false;
      stage.hidden = true;
    } else {
      thread.hidden = true;
      stage.hidden = !signedIn;
    }
    renderList();
    return;
  }
  showThread(undefined);
  if (mode === "friends") {
    void loadPeople();
    return;
  }
  showAddScreen("home");
  void loadRecommendations();
}

const LIST_WIDTH_KEY = "lineapp.listWidth";
const listPane = document.querySelector<HTMLElement>("#list-pane")!;
const paneSplit = document.querySelector<HTMLElement>("#pane-split")!;

function listWidthBounds(): { min: number; max: number } {
  const rail = document.querySelector<HTMLElement>("#rail")?.offsetWidth ?? 68;
  const room = Math.max(0, app.clientWidth - rail - paneSplit.offsetWidth);
  const min = Math.min(320, Math.max(240, room - 280));
  const max = Math.max(min, room - 300);
  return { min, max };
}

function applyListWidth(width: number, save: boolean): void {
  const { min, max } = listWidthBounds();
  const next = Math.round(Math.min(max, Math.max(min, width)));
  app.style.setProperty("--list-width", `${next}px`);
  if (save) {
    localStorage.setItem(LIST_WIDTH_KEY, String(next));
  }
}

function fitListWidth(): void {
  const saved = Number(localStorage.getItem(LIST_WIDTH_KEY));
  applyListWidth(Number.isFinite(saved) && saved > 0 ? saved : 400, false);
}

paneSplit.addEventListener("pointerdown", (event) => {
  if (event.button !== 0) {
    return;
  }
  event.preventDefault();
  paneSplit.classList.add("is-drag");
  document.body.classList.add("is-pane-drag");
  const startX = event.clientX;
  const start = listPane.getBoundingClientRect().width;
  const direction = document.documentElement.dir === "rtl" ? -1 : 1;
  const move = (ev: PointerEvent) => {
    applyListWidth(start + direction * (ev.clientX - startX), false);
  };
  const up = () => {
    paneSplit.classList.remove("is-drag");
    document.body.classList.remove("is-pane-drag");
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    applyListWidth(listPane.getBoundingClientRect().width, true);
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
});

paneSplit.addEventListener("keydown", (event) => {
  if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") {
    return;
  }
  event.preventDefault();
  const direction = document.documentElement.dir === "rtl" ? -1 : 1;
  const delta = event.key === "ArrowRight" ? 24 : -24;
  applyListWidth(listPane.getBoundingClientRect().width + direction * delta, true);
});

window.addEventListener("resize", fitListWidth);
fitListWidth();

railPeople.addEventListener("click", () => setRail("friends"));
railChats.addEventListener("click", () => {
  filter = "all";
  syncChatTabs();
  setRail("chats");
});
railFriends.addEventListener("click", () => setRail("add"));
railAgent.addEventListener("click", () => {
  void openAgent();
});
agentSettingsButton.addEventListener("click", () => {
  agentSettingsFromAgent = true;
  void showAgentSettings();
});
agentClear.addEventListener("click", () => {
  clearAgentLog();
});
agentForm.addEventListener("submit", (event) => {
  event.preventDefault();
  void askAgent();
});
agentInput.addEventListener("input", syncAgentSend);
agentInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    void askAgent();
  }
});
peopleQuery.addEventListener("input", paintPeople);
addRecsToggle.addEventListener("click", () => {
  recsOpen = !recsOpen;
  paintRecommendations();
});
document.querySelector<HTMLButtonElement>("#add-search")!.addEventListener("click", () => showAddScreen("search"));
document.querySelector<HTMLButtonElement>("#add-group")!.addEventListener("click", () => showAddScreen("group"));
document.querySelector<HTMLButtonElement>("#add-search-back")!.addEventListener("click", () => showAddScreen("home"));
document.querySelector<HTMLButtonElement>("#add-group-back")!.addEventListener("click", () => showAddScreen("home"));
addGroupQuery.addEventListener("input", paintGroupPicks);
addSearchForm.addEventListener("submit", (event) => {
  event.preventDefault();
  const query = addSearchQuery.value.trim();
  if (!query) {
    return;
  }
  addSearchNote.hidden = false;
  addSearchNote.textContent = t("searchLabel");
  addSearchResult.replaceChildren();
  void invoke<DirectoryPerson>("find_person", { query })
    .then((person) => {
      addSearchNote.hidden = true;
      if (!person?.id) {
        addSearchNote.hidden = false;
        addSearchNote.textContent = t("nobodyFound");
        return;
      }
      paintSearchHit(person);
    })
    .catch((error: unknown) => {
      addSearchNote.hidden = false;
      addSearchNote.textContent = directoryError(error);
    });
});
addGroupCreate.addEventListener("click", () => {
  if (groupPicked.size === 0) {
    return;
  }
  const typed = addGroupInput.value.trim();
  const targets = [...groupPicked];
  const names = directoryPeople.filter((person) => groupPicked.has(person.id)).map((person) => person.name);
  addGroupCreate.disabled = true;
  addGroupNote.hidden = false;
  addGroupNote.textContent = t("create");
  void invoke<{ id: string; name: string; members: number }>("create_group", { name: typed, targets })
    .then((made) => {
      groupPicked.clear();
      addGroupInput.value = "";
      const chat: ChatRow = {
        id: made.id,
        name: made.name || typed || names.join(", ") || t("group"),
        lastMessage: "",
        sealed: false,
        time: Date.now(),
        members: made.members,
        picture: "",
        muted: false,
        kind: "group",
        unread: null,
      };
      rememberChat(chat);
      setRail("chats");
    })
    .catch((error: unknown) => {
      addGroupCreate.disabled = groupPicked.size === 0;
      addGroupNote.hidden = false;
      addGroupNote.textContent = directoryError(error);
    });
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
  closeSendMenu();
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
    const image = rowImageQuote(row);
    replyDraft = {
      id: row.dataset.id || "",
      text: quoteText(row),
      name: senderLabel(row),
      picture: rowPicture(row),
      thumb: image.url,
      mediaId: image.mediaId,
    };
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

replyThumb.addEventListener("error", () => {
  replyThumb.hidden = true;
});

syncAppearanceControls();

fontFamily.addEventListener("change", () => {
  saveAppearance({ font: selectedFont(), scaleIndex: Number(textSize.value) });
});

textSize.addEventListener("input", () => {
  saveAppearance({ font: selectedFont(), scaleIndex: Number(textSize.value) });
  paintTextSize();
});

let volumePreviewAt = 0;
let volumePreviewTimer = 0;
notificationVolume.addEventListener("input", () => {
  const volume = Math.min(100, Math.max(0, Math.round(Number(notificationVolume.value))));
  setNotificationVolume(volume);
  paintNotificationVolume();
  window.clearTimeout(volumePreviewTimer);
  if (notificationGain(volume) <= 0) {
    return;
  }
  const play = () => {
    volumePreviewAt = performance.now();
    void playChime();
  };
  const wait = 180 - (performance.now() - volumePreviewAt);
  if (wait <= 0) {
    play();
    return;
  }
  volumePreviewTimer = window.setTimeout(play, wait);
});

notificationTones.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("button[data-tone]");
  const tone = button?.dataset.tone ?? "";
  if (!button || !(TONE_IDS as readonly string[]).includes(tone)) {
    return;
  }
  setNotificationTone(tone as ToneId);
  paintNotificationTone();
  window.clearTimeout(volumePreviewTimer);
  volumePreviewAt = performance.now();
  void playChime();
});

await listen<LoginEvent>("login-event", (event) => {
  void onEvent(event.payload);
});

await listen<{ type?: string; text?: string; name?: string }>("agent-stream", (event) => {
  if (!agentBusy) {
    return;
  }
  if (event.payload.type === "text" && event.payload.text) {
    agentLiveText += event.payload.text;
  } else if (event.payload.type === "call" && event.payload.name) {
    agentLiveCalls.push(event.payload.name);
  }
  if (railMode === "agent") {
    paintAgent();
  }
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
    replyThumb: item.reply?.thumb || "",
    replyMediaId: item.reply?.mediaId || "",
  });
  const bubble = row.querySelector(".bubble");
  if (item.albumUrls && item.albumUrls.length > 1) {
    bubble?.classList.add("rich");
    bubble?.querySelector(".msg-text")?.remove();
    const grid = document.createElement("div");
    grid.className = "media-grid many";
    const urls = item.albumUrls;
    urls.forEach((url, index) => {
      grid.append(imageThumb(url, () => openLocalGallery(urls, index)));
    });
    bubble?.prepend(grid);
    if (item.file?.groupId) {
      row.dataset.gid = item.file.groupId;
    }
  } else if (item.previewUrl && item.kind === "image") {
    bubble?.classList.add("rich");
    bubble?.querySelector(".msg-text")?.remove();
    const grid = document.createElement("div");
    grid.className = "media-grid one";
    const url = item.previewUrl;
    grid.append(imageThumb(url, () => openLocalGallery([url], 0)));
    bubble?.prepend(grid);
  } else if (item.kind === "audio") {
    bubble?.querySelector(".msg-text")?.remove();
    bubble?.append(voiceCard(item.localId, item.duration || 0, item.previewUrl || "", item.voice ? "" : item.text, item.peaks));
  } else if (item.kind === "file" && item.file) {
    const file = item.file;
    bubble?.querySelector(".msg-text")?.remove();
    bubble?.append(fileCard(file.name, (button) => {
      void saveDownload(file.name, file.mime, file.data, button);
    }));
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
  if (item.silent) {
    row.dataset.quiet = "1";
    markQuiet(row);
  }
  const retry = row.querySelector<HTMLButtonElement>(".msg-retry");
  if (retry && item.error) {
    retry.title = item.error;
  }
  return row;
}

function markQuiet(row: HTMLElement): void {
  const meta = row.classList.contains("msg-meta") ? row : row.querySelector(".msg-meta");
  if (!meta || meta.querySelector(".msg-quiet")) {
    return;
  }
  const mark = document.createElement("span");
  mark.className = "msg-quiet";
  mark.textContent = t("sentQuiet");
  meta.prepend(mark);
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
  if (row.dataset.quiet === "1") {
    markQuiet(next);
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
  const sent = messageBubble({
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
    duration: payload.duration || item?.duration || 0,
    gid: payload.gid || "",
    seq: payload.seq || 0,
    replyId: item?.reply?.id || payload.replyId || "",
    replyText: item?.reply?.text || "",
    replyName: item?.reply?.name || "",
    replyPicture: item?.reply?.picture || "",
    replyThumb: item?.reply?.thumb || "",
    replyMediaId: item?.reply?.mediaId || "",
  });
  if (item?.silent) {
    sent.dataset.quiet = "1";
    markQuiet(sent);
  }
  threadBody.append(sent);
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
    duration: payload.duration || item?.duration || 0,
    animated: Boolean(payload.animated),
    gid: payload.gid || "",
    seq: payload.seq || 0,
    replyId: item?.reply?.id || payload.replyId || "",
    replyText: item?.reply?.text || "",
    replyName: item?.reply?.name || "",
    replyPicture: item?.reply?.picture || "",
    replyThumb: item?.reply?.thumb || "",
    replyMediaId: item?.reply?.mediaId || "",
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
      duration: message.duration || prior.duration,
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
      silent: item.silent === true,
      durationMs: item.duration || 0,
      voice: item.voice === true,
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
    silent: item.silent === true,
  }).then(() => undefined, fail);
}

const AUTO_REPLY_KEY = "lineapp.autoReply";
const DEFAULT_HISTORY = 20;
const MAX_HISTORY = 100;
type AutoTurn = { id: string; mine: boolean; name: string; text: string };
const autoThreads = new Map<string, AutoTurn[]>();
const autoQueued = new Set<string>();
const autoReplyTimers = new Map<string, number>();
const autoReplyBusy = new Set<string>();
const autoReplyAgain = new Set<string>();
let agentReady = false;
let agentProbed = false;
let agentProbeStarted = false;
let agentProbeError = "";

function autoReplyOn(chatId: string): boolean {
  return Boolean(chatId) && autoReplyState[chatId] === true;
}

function loadAutoReply(): void {
  autoReplyState = {};
  try {
    const saved = JSON.parse(localStorage.getItem(AUTO_REPLY_KEY) || "{}") as unknown;
    const ids = Array.isArray(saved) ? saved : saved && typeof saved === "object" ? Object.keys(saved) : [];
    const record = Array.isArray(saved) ? null : saved && typeof saved === "object" ? saved as Record<string, unknown> : null;
    for (const id of ids) {
      if (typeof id !== "string" || !id) {
        continue;
      }
      if (record && record[id] !== true) {
        continue;
      }
      autoReplyState[id] = true;
    }
  } catch {
    autoReplyState = {};
  }
}

function saveAutoReply(): void {
  const saved: Record<string, boolean> = {};
  for (const [id, on] of Object.entries(autoReplyState)) {
    if (id && on === true) {
      saved[id] = true;
    }
  }
  localStorage.setItem(AUTO_REPLY_KEY, JSON.stringify(saved));
}

function replaceAutoThread(chatId: string, messages: HistoryMessage[]): void {
  const turns: AutoTurn[] = [];
  for (const message of messages) {
    const text = message.text?.trim() || "";
    if (!text || message.kind === "chat update" || message.kind === "unsent") {
      continue;
    }
    turns.push({ id: message.id || "", mine: message.mine, name: message.name || "", text });
  }
  autoThreads.set(chatId, turns.slice(-MAX_HISTORY));
}

function pushAutoTurn(chatId: string, message: { id?: string; mine: boolean; name?: string; text?: string }): void {
  const text = message.text?.trim() || "";
  if (!chatId || !text) {
    return;
  }
  const turns = autoThreads.get(chatId) ?? [];
  const id = message.id || "";
  const next = { id, mine: message.mine, name: message.name || "", text };
  const index = id ? turns.findIndex((turn) => turn.id === id) : -1;
  if (index >= 0) {
    turns[index] = next;
  } else {
    turns.push(next);
  }
  autoThreads.set(chatId, turns.slice(-MAX_HISTORY));
}

const autoReplyStarted = new Map<string, number>();
const autoReplyToken = new Map<string, number>();
let autoReplyClock = 0;
let autoReplyCancelNote = "";
const autoReplyErrors = new Map<string, string>();

function formatElapsed(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

function paintAutoReplyWait(): void {
  const started = openId ? autoReplyStarted.get(openId) : undefined;
  const cancelled = Boolean(openId) && autoReplyCancelNote === openId && !started;
  autoReplyCancelled.hidden = !cancelled;
  const failure = openId ? autoReplyErrors.get(openId) || "" : "";
  autoReplyError.hidden = !failure || Boolean(started);
  autoReplyError.textContent = failure;
  if (!started) {
    autoReplyWait.hidden = true;
    return;
  }
  autoReplyWait.hidden = false;
  autoReplyElapsed.textContent = formatElapsed(Date.now() - started);
}

function interruptAutoReply(): void {
  const chatId = openId;
  const saved = chatId ? chatLlmCache.get(chatId) : undefined;
  if (!chatId || !saved?.interrupt || !autoReplyStarted.has(chatId)) {
    return;
  }
  autoReplyToken.set(chatId, (autoReplyToken.get(chatId) ?? 0) + 1);
  autoReplyAgain.delete(chatId);
  autoReplyCancelNote = chatId;
  settleAutoReply(chatId);
}

function watchAutoReply(chatId: string): void {
  autoReplyStarted.set(chatId, Date.now());
  if (!autoReplyClock) {
    autoReplyClock = window.setInterval(paintAutoReplyWait, 1000);
  }
  paintAutoReplyWait();
}

function settleAutoReply(chatId: string): void {
  autoReplyStarted.delete(chatId);
  if (autoReplyStarted.size === 0 && autoReplyClock) {
    window.clearInterval(autoReplyClock);
    autoReplyClock = 0;
  }
  paintAutoReplyWait();
}

type ChatLlm = {
  endpoint: string;
  model: string;
  apiKey: string;
  protocol: string;
  temperature: string;
  topP: string;
  maxTokens: string;
  thinking: string;
  extra: string;
  context: string;
  language: string;
  markAi: boolean;
  interrupt: boolean;
  history: number;
};

function chatHistoryLimit(chatId: string): number {
  const saved = chatId ? chatLlmCache.get(chatId) : undefined;
  const raw = saved?.history;
  if (!raw || raw < 1) {
    return DEFAULT_HISTORY;
  }
  return Math.min(MAX_HISTORY, Math.floor(raw));
}

function historyField(value: string): number {
  const raw = Number(value.trim());
  if (!value.trim() || !Number.isFinite(raw) || raw < 1) {
    return DEFAULT_HISTORY;
  }
  return Math.floor(raw);
}

const chatLlmCache = new Map<string, ChatLlm | null>();
let chatLlmEditing = "";

function chatHasOwnModel(chatId: string): boolean {
  const saved = chatId ? chatLlmCache.get(chatId) : undefined;
  return Boolean(saved && saved.endpoint.trim() && saved.model.trim());
}

function chatModelName(chatId: string): string {
  const own = chatId ? chatLlmCache.get(chatId) : undefined;
  if (own && own.model.trim()) {
    return own.model.trim();
  }
  return agentConfig.model.trim();
}

function llmFromAgent(): ChatLlm {
  return {
    endpoint: agentConfig.endpoint,
    model: agentConfig.model,
    apiKey: agentConfig.apiKey,
    protocol: agentConfig.protocol || "openai",
    temperature: agentConfig.replyTemperature || "",
    topP: agentConfig.replyTopP || "",
    maxTokens: agentConfig.replyMaxTokens || "",
    thinking: agentConfig.replyThinking || "off",
    extra: agentConfig.replyExtra || "",
    context: agentConfig.autoContext || "",
    language: "",
    markAi: false,
    interrupt: false,
    history: DEFAULT_HISTORY,
  };
}

function fillChatLlm(settings: ChatLlm, saved: boolean): void {
  chatLlmProtocol.value = settings.protocol === "anthropic" || settings.protocol === "gemini" ? settings.protocol : "openai";
  chatLlmEndpoint.value = settings.endpoint;
  chatLlmModel.value = settings.model;
  chatLlmKey.value = settings.apiKey;
  chatLlmTemperature.value = settings.temperature;
  chatLlmTopP.value = settings.topP;
  chatLlmMaxTokens.value = settings.maxTokens;
  chatLlmThinking.value = ["default", "off", "low", "medium", "high"].includes(settings.thinking) ? settings.thinking : "off";
  chatLlmLanguage.value = settings.language || "";
  if (chatLlmLanguage.value !== (settings.language || "")) {
    chatLlmLanguage.value = "";
  }
  chatLlmHistory.value = String(historyField(String(settings.history ?? DEFAULT_HISTORY)));
  chatLlmContext.value = settings.context;
  chatLlmMarkAi.checked = settings.markAi === true;
  chatLlmInterrupt.checked = settings.interrupt === true;
  chatLlmExtra.value = settings.extra;
  chatLlmUnsaved.hidden = saved;
  chatLlmUnsaved.textContent = t("chatLlmUnsaved");
  chatLlmClear.disabled = !saved;
}

async function rememberChatLlm(chatId: string): Promise<ChatLlm | null> {
  if (!chatId) {
    return null;
  }
  try {
    const saved = await invoke<ChatLlm | null>("get_chat_llm", { id: chatId });
    chatLlmCache.set(chatId, saved);
    return saved;
  } catch {
    return chatLlmCache.get(chatId) ?? null;
  }
}

function paintAutoReply(chatId = openId): void {
  const usable = agentReady || chatHasOwnModel(chatId);
  const on = usable && autoReplyOn(chatId);
  autoReplyButton.setAttribute("aria-pressed", on ? "true" : "false");
  autoReplyButton.dataset.chat = chatId;
  autoReplyButton.disabled = !chatId || (!agentReady && !chatHasOwnModel(chatId));
  chatLlmButton.disabled = !chatId;
  const modelName = chatId ? chatModelName(chatId) : "";
  chatLlmModelLabel.hidden = !modelName;
  chatLlmModelLabel.textContent = modelName;
  chatLlmButton.title = modelName;
  aiMark.hidden = !(chatId && chatLlmCache.get(chatId)?.markAi);
  if (!chatId) {
    autoReplyButton.title = t("autoReplyOff");
  } else if (!agentProbeStarted) {
    autoReplyButton.title = t("autoReplyChecking");
  } else if (!usable) {
    autoReplyButton.title = agentProbeError || t("autoReplyUnavailable");
  } else {
    autoReplyButton.title = on ? t("autoReplyOn") : t("autoReplyOff");
  }
  paintAutoReplyWait();
}

function ensureAgentProbe(): void {
  if (agentProbed) {
    return;
  }
  agentProbed = true;
  void refreshAgentReady();
}

async function refreshAgentReady(): Promise<void> {
  agentReady = false;
  agentProbeError = "";
  paintAutoReply();
  try {
    await invoke("agent_probe");
    agentReady = true;
  } catch (error) {
    agentReady = false;
    agentProbeError = error instanceof Error ? error.message : String(error);
  }
  agentProbeStarted = true;
  paintAutoReply();
}

function queueAutoReply(chatId: string, messageId: string): void {
  if (!chatId || !messageId || !autoReplyOn(chatId) || (!agentReady && !chatHasOwnModel(chatId))) {
    return;
  }
  const key = `${chatId}\u001f${messageId}`;
  if (autoQueued.has(key)) {
    return;
  }
  autoQueued.add(key);
  window.clearTimeout(autoReplyTimers.get(chatId));
  autoReplyTimers.set(chatId, window.setTimeout(() => {
    void runAutoReply(chatId);
  }, 800));
}

async function runAutoReply(chatId: string): Promise<void> {
  if (!autoReplyOn(chatId) || (!agentReady && !chatHasOwnModel(chatId))) {
    return;
  }
  if (autoReplyBusy.has(chatId)) {
    autoReplyAgain.add(chatId);
    return;
  }
  const turns = autoThreads.get(chatId) ?? [];
  const last = turns[turns.length - 1];
  if (!last || last.mine) {
    return;
  }
  autoReplyBusy.add(chatId);
  if (autoReplyCancelNote === chatId) {
    autoReplyCancelNote = "";
  }
  const token = autoReplyToken.get(chatId) ?? 0;
  autoReplyErrors.delete(chatId);
  watchAutoReply(chatId);
  if (chatId === openId) {
    sendError.hidden = true;
  }
  try {
    const text = await invoke<string>("agent_compose", {
      chatId,
      messages: turns.slice(-chatHistoryLimit(chatId)).map((turn) => ({ mine: turn.mine, name: turn.name, text: turn.text })),
    });
    if ((autoReplyToken.get(chatId) ?? 0) !== token) {
      return;
    }
    let reply = text.trim();
    if (reply && chatLlmCache.get(chatId)?.markAi) {
      const mark = t("chatLlmAiMark");
      if (!reply.startsWith(mark)) {
        reply = mark + reply;
      }
    }
    if (reply && (agentReady || chatHasOwnModel(chatId)) && autoReplyOn(chatId)) {
      autoReplyErrors.delete(chatId);
      if (chatId === openId) {
        sendError.hidden = true;
      }
      startSend(chatId, reply, null);
      pushAutoTurn(chatId, { id: "", mine: true, name: "", text: reply });
    }
  } catch (error) {
    if ((autoReplyToken.get(chatId) ?? 0) !== token) {
      return;
    }
    const message = error instanceof Error ? error.message : String(error);
    autoReplyErrors.set(chatId, message.trim() || "The endpoint did not respond.");
    if (chatId === openId) {
      sendError.hidden = false;
      sendError.textContent = autoReplyErrors.get(chatId) || "";
    }
  } finally {
    autoReplyBusy.delete(chatId);
    const cancelled = (autoReplyToken.get(chatId) ?? 0) !== token;
    if (!cancelled) {
      settleAutoReply(chatId);
    }
    if (!cancelled && autoReplyAgain.delete(chatId)) {
      void runAutoReply(chatId);
    }
  }
}

function startSend(chatId: string, text: string, reply: ReplyDraft | null, silent = false): void {
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
    silent,
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

void listen<SendResult>("line-sent", (event) => {
  try {
    onSent(event.payload);
  } catch (error) {
    setStatusText(error instanceof Error ? error.message : String(error));
  }
});

void listen<{ id?: string; settings?: ChatLlm | null }>("chat-llm", (event) => {
  const id = event.payload?.id || "";
  if (!id) {
    return;
  }
  const settings = event.payload.settings ?? null;
  chatLlmCache.set(id, settings);
  if (openId === id) {
    paintAutoReply(id);
  }
  if (chatLlmEditing === id && !sheetChatLlm.hidden) {
    fillChatLlm(settings ?? llmFromAgent(), Boolean(settings));
    void paintChatLlmCopy(id);
  }
});

const MEDIA_LIMIT = 40_000_000;

type StickerSale = "free" | "coins" | "store" | "premium" | "none";

type StickerPack = {
  id: string;
  name: string;
  animated: boolean;
  stickerIds: string[];
  owned: boolean;
  price: string;
  coins: string;
  sale: StickerSale;
};

let stickerShelf: { owned: StickerPack[]; shop: StickerPack[]; shopError: string } | null = null;
let stickerTab: "mine" | "shop" = "mine";
let stickerPackId = "";
let stickerLoad = 0;
let stickerBuying = false;
let stickerNoteText = "";

function closeStickerPanel(): void {
  stickerPanel.hidden = true;
  stickerOpen.setAttribute("aria-expanded", "false");
  scheduleStickerSuggest();
}

function stickerList(): StickerPack[] {
  if (!stickerShelf) {
    return [];
  }
  return stickerTab === "mine" ? stickerShelf.owned : stickerShelf.shop;
}

function selectedStickerPack(): StickerPack | null {
  const packs = stickerList();
  return packs.find((pack) => pack.id === stickerPackId) || packs[0] || null;
}

function asStickerSale(value: unknown): StickerSale {
  if (value === "free" || value === "coins" || value === "store" || value === "premium" || value === "none") {
    return value;
  }
  return "none";
}

function asStickerPack(value: unknown): StickerPack | null {
  if (!value || typeof value !== "object") {
    return null;
  }
  const row = value as Record<string, unknown>;
  const id = String(row.id || "");
  if (!/^\d{1,16}$/.test(id)) {
    return null;
  }
  const stickerIds = Array.isArray(row.stickerIds) ? row.stickerIds.map((item) => String(item)).filter((item) => /^\d{1,16}$/.test(item)) : [];
  return {
    id,
    name: String(row.name || id),
    animated: Boolean(row.animated),
    stickerIds,
    owned: Boolean(row.owned),
    price: String(row.price || ""),
    coins: String(row.coins || ""),
    sale: asStickerSale(row.sale),
  };
}

function stickerImageUrl(stickerId: string, animated: boolean): string {
  const asset = animated ? "sticker_animation.png" : "sticker.png";
  return `https://stickershop.line-scdn.net/stickershop/v1/sticker/${stickerId}/android/${asset}`;
}

function showStickerNote(text: string): void {
  stickerNote.hidden = text.length === 0;
  stickerNote.textContent = text;
}

const STICKER_STORE = "https://store.line.me/stickershop";

function stickerStoreUrl(): string {
  const pack = stickerTab === "shop" ? selectedStickerPack() : null;
  return pack ? `${STICKER_STORE}/product/${pack.id}` : STICKER_STORE;
}

function paintStickerElsewhere(): void {
  const show = stickerTab === "shop";
  stickerElsewhere.hidden = !show;
  if (!show) {
    return;
  }
  const url = stickerStoreUrl();
  stickerStoreLink.title = url;
  stickerStoreLink.setAttribute("aria-label", url);
}

function paintStickerOffer(): void {
  const pack = selectedStickerPack();
  if (!pack || pack.owned) {
    stickerOffer.hidden = true;
    return;
  }
  stickerOffer.hidden = false;
  stickerBuy.hidden = pack.sale !== "free" && pack.sale !== "coins";
  stickerBuy.disabled = stickerBuying;
  if (pack.sale === "free") {
    stickerOfferText.textContent = t("stickerFree");
    stickerBuy.textContent = stickerBuying ? t("stickerBuying") : t("stickerGet");
    return;
  }
  if (pack.sale === "coins") {
    const raw = pack.price || pack.coins;
    const price = /^\d+$/.test(raw) ? `${raw} LINE Coins` : raw;
    stickerOfferText.textContent = price;
    stickerBuy.textContent = stickerBuying ? t("stickerBuying") : t("stickerBuy", { price });
    return;
  }
  const key = pack.sale === "store" ? "stickerStoreOnly" : pack.sale === "premium" ? "stickerPremiumOnly" : "stickerUnavailable";
  stickerOfferText.textContent = t(key);
}

function paintStickerPanel(): void {
  stickerTabMine.setAttribute("aria-selected", stickerTab === "mine" ? "true" : "false");
  stickerTabShop.setAttribute("aria-selected", stickerTab === "shop" ? "true" : "false");
  const packs = stickerList();
  const selected = selectedStickerPack();
  stickerPackId = selected?.id || "";
  stickerPacks.replaceChildren();
  for (const pack of packs) {
    const button = document.createElement("button");
    button.type = "button";
    button.title = pack.owned ? `${pack.name} · ${t("stickerOwned")}` : pack.name;
    button.setAttribute("aria-selected", pack.id === stickerPackId ? "true" : "false");
    const img = document.createElement("img");
    img.alt = pack.name;
    img.src = `https://stickershop.line-scdn.net/stickershop/v1/product/${pack.id}/android/tab_on.png`;
    img.addEventListener("error", () => {
      img.src = `https://stickershop.line-scdn.net/stickershop/v1/product/${pack.id}/android/main.png`;
    }, { once: true });
    button.append(img);
    button.addEventListener("click", () => {
      stickerPackId = pack.id;
      paintStickerPanel();
      void fillStickerPack(pack);
    });
    stickerPacks.append(button);
  }
  stickerGrid.replaceChildren();
  if (selected) {
    for (const stickerId of selected.stickerIds) {
      const button = document.createElement("button");
      button.type = "button";
      if (!selected.owned) {
        button.dataset.locked = "true";
      }
      const img = document.createElement("img");
      img.alt = "";
      img.src = stickerImageUrl(stickerId, selected.animated);
      button.append(img);
      button.addEventListener("click", () => {
        if (!selected.owned || !openId || composerInput.disabled) {
          return;
        }
        closeStickerPanel();
        startSticker(openId, { packageId: selected.id, stickerId, animated: selected.animated });
      });
      stickerGrid.append(button);
    }
  }
  paintStickerOffer();
  paintStickerElsewhere();
  if (!stickerShelf) {
    showStickerNote(stickerNoteText || t("stickerLoading"));
    return;
  }
  if (stickerNoteText) {
    showStickerNote(stickerNoteText);
    return;
  }
  if (packs.length === 0) {
    showStickerNote(stickerTab === "shop" && stickerShelf.shopError ? stickerShelf.shopError : t("stickerEmpty"));
    return;
  }
  showStickerNote("");
}

function replaceStickerPack(pack: StickerPack): void {
  if (!stickerShelf) {
    return;
  }
  for (const list of [stickerShelf.owned, stickerShelf.shop]) {
    const index = list.findIndex((item) => item.id === pack.id);
    if (index >= 0) {
      list[index] = { ...list[index], ...pack, owned: list[index].owned || pack.owned };
    }
  }
  if (pack.owned && !stickerShelf.owned.some((item) => item.id === pack.id)) {
    stickerShelf.owned.unshift(pack);
  }
}

async function fillStickerPack(pack: StickerPack): Promise<void> {
  if (pack.stickerIds.length > 0 && (pack.owned || pack.sale === "free" || pack.sale === "store" || pack.sale === "premium" || pack.sale === "none" || pack.price)) {
    return;
  }
  try {
    const loaded = asStickerPack(await invoke("sticker_pack", { productId: pack.id }));
    if (loaded) {
      replaceStickerPack(loaded);
      paintStickerPanel();
    }
  } catch (error) {
    showStickerNote(error instanceof Error ? error.message : String(error));
  }
}

async function loadStickerShelf(): Promise<void> {
  const ticket = stickerLoad + 1;
  stickerLoad = ticket;
  stickerNoteText = "";
  if (!stickerShelf) {
    paintStickerPanel();
  }
  try {
    const payload = await invoke<{ owned?: unknown; shop?: unknown; shopError?: unknown }>("list_stickers");
    if (ticket !== stickerLoad) {
      return;
    }
    const owned = Array.isArray(payload.owned) ? payload.owned.map(asStickerPack).filter((pack): pack is StickerPack => Boolean(pack)) : [];
    const shop = Array.isArray(payload.shop) ? payload.shop.map(asStickerPack).filter((pack): pack is StickerPack => Boolean(pack)) : [];
    stickerShelf = { owned, shop, shopError: String(payload.shopError || "") };
    if (!stickerList().some((pack) => pack.id === stickerPackId)) {
      stickerPackId = stickerList()[0]?.id || "";
    }
    paintStickerPanel();
    const selected = selectedStickerPack();
    if (selected) {
      void fillStickerPack(selected);
    }
  } catch (error) {
    if (ticket !== stickerLoad) {
      return;
    }
    stickerNoteText = error instanceof Error ? error.message : String(error);
    paintStickerPanel();
  }
}

async function buySelectedSticker(): Promise<void> {
  const pack = selectedStickerPack();
  if (!pack || pack.owned || stickerBuying || (pack.sale !== "free" && pack.sale !== "coins")) {
    return;
  }
  stickerBuying = true;
  stickerNoteText = "";
  paintStickerOffer();
  try {
    const result = await invoke<{ owned?: boolean; confirmUrl?: string; pack?: unknown }>("buy_sticker", { productId: pack.id });
    if (result.confirmUrl?.startsWith("https://")) {
      stickerNoteText = t("stickerConfirm");
      void invoke("open_link", { url: result.confirmUrl });
      return;
    }
    const loaded = asStickerPack(result.pack);
    replaceStickerPack(loaded ? { ...loaded, owned: true } : { ...pack, owned: true });
  } catch (error) {
    stickerNoteText = error instanceof Error ? error.message : String(error);
  } finally {
    stickerBuying = false;
    paintStickerPanel();
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

type DictatePhase = "idle" | "prepare" | "listen" | "transcribe";

const MICROPHONE_KEY = "lineapp.microphone";

let dictatePhase: DictatePhase = "idle";
let dictateGen = 0;
let dictateTimer = 0;
let dictateLevelTimer = 0;
let dictateRun: Promise<boolean> | null = null;

function savedMicrophone(): string {
  return localStorage.getItem(MICROPHONE_KEY) || "";
}

function microphoneSource(): string {
  if (listedDevices) {
    const chosen = chosenMicrophone(listedDevices.inputs);
    if (chosen) {
      return chosen;
    }
  }
  return savedMicrophone();
}

function chosenMicrophone(devices: ListedDevice[]): string {
  const saved = savedMicrophone();
  if (devices.some((device) => device.id === saved)) {
    return saved;
  }
  return devices.find((device) => device.detail === "Default" || device.detail.startsWith("Default ·"))?.id || "";
}

function setDictateMeter(level: number): void {
  const clamped = Math.max(0, Math.min(100, Math.round(level)));
  dictateFill.style.width = `${clamped}%`;
  dictateMeter.classList.toggle("hearing", clamped >= 22);
}

function stopDictateMeter(): void {
  window.clearInterval(dictateLevelTimer);
  dictateLevelTimer = 0;
  dictateMeter.hidden = true;
  setDictateMeter(0);
}

function startDictateMeter(): void {
  stopDictateMeter();
  dictateMeter.hidden = false;
  dictateLevelTimer = window.setInterval(() => {
    void invoke<number>("dictate_level").then((level) => {
      if (dictatePhase === "listen") {
        setDictateMeter(level);
      }
    }).catch(() => undefined);
  }, 100);
}

function dictateMessage(error: unknown): string {
  const code = error instanceof Error ? error.message : String(error);
  if (code.includes("no-mic")) {
    return t("dictateNoMic");
  }
  if (code.includes("no-model") || code.includes("no-engine")) {
    return t("dictateUnavailable");
  }
  if (code.includes("empty")) {
    return t("dictateEmpty");
  }
  return t("dictateFailed");
}

function paintDictate(): void {
  const listening = dictatePhase === "listen";
  dictateButton.disabled = composerInput.disabled || dictatePhase === "prepare" || dictatePhase === "transcribe";
  dictateButton.setAttribute("aria-pressed", listening || dictatePhase === "transcribe" ? "true" : "false");
  const key = dictatePhase === "prepare" ? "dictatePrepare" : dictatePhase === "transcribe" ? "dictateWorking" : listening ? "dictateStop" : "dictate";
  dictateButton.setAttribute("aria-label", t(key));
  dictateButton.title = t(key);
}

function insertDictation(text: string): void {
  const body = text.trim();
  if (!body) {
    sendError.hidden = false;
    sendError.textContent = t("dictateEmpty");
    return;
  }
  const start = composerInput.selectionStart ?? composerInput.value.length;
  const end = composerInput.selectionEnd ?? start;
  const before = composerInput.value.slice(0, start);
  const after = composerInput.value.slice(end);
  const gap = before.length > 0 && !/\s$/.test(before) ? " " : "";
  composerInput.value = `${before}${gap}${body}${after}`;
  const caret = before.length + gap.length + body.length;
  composerInput.setSelectionRange(caret, caret);
  sendError.hidden = true;
  refreshSendButton();
  scheduleStickerSuggest();
  composerInput.focus();
}

function abandonDictation(): void {
  if (dictatePhase === "idle") {
    return;
  }
  dictateGen += 1;
  const recording = dictatePhase === "listen" || dictatePhase === "prepare";
  dictatePhase = "idle";
  window.clearTimeout(dictateTimer);
  stopDictateMeter();
  if (recording) {
    void invoke("dictate_cancel").catch(() => undefined);
  }
  paintDictate();
}

async function stopDictation(): Promise<boolean> {
  if (dictateRun) {
    return dictateRun;
  }
  if (dictatePhase !== "listen") {
    return dictatePhase === "idle";
  }
  const gen = dictateGen;
  const chat = openId;
  dictatePhase = "transcribe";
  window.clearTimeout(dictateTimer);
  stopDictateMeter();
  paintDictate();
  dictateRun = (async () => {
    try {
      const text = await invoke<string>("dictate_stop", { language: intlLocale() });
      if (gen !== dictateGen || chat !== openId) {
        return false;
      }
      insertDictation(text);
      return true;
    } catch (error) {
      if (gen !== dictateGen || chat !== openId) {
        return false;
      }
      sendError.hidden = false;
      sendError.textContent = dictateMessage(error);
      return false;
    } finally {
      if (gen === dictateGen) {
        dictatePhase = "idle";
        paintDictate();
      }
      dictateRun = null;
    }
  })();
  return dictateRun;
}

async function finishDictation(): Promise<boolean> {
  if (dictatePhase === "listen") {
    return stopDictation();
  }
  if (dictateRun) {
    return dictateRun;
  }
  return dictatePhase === "idle";
}

async function toggleDictation(): Promise<void> {
  if (dictatePhase === "listen") {
    await stopDictation();
    return;
  }
  if (dictatePhase !== "idle" || composerInput.disabled) {
    return;
  }
  const gen = dictateGen;
  dictatePhase = "prepare";
  paintDictate();
  sendError.hidden = true;
  try {
    if (micTestOn) {
      await cancelMicTest();
    }
    await invoke("dictate_start", { source: microphoneSource() });
    if (gen !== dictateGen) {
      await invoke("dictate_cancel").catch(() => undefined);
      return;
    }
    dictatePhase = "listen";
    paintDictate();
    startDictateMeter();
    dictateTimer = window.setTimeout(() => {
      void stopDictation();
    }, 90_000);
  } catch (error) {
    if (gen !== dictateGen) {
      return;
    }
    dictatePhase = "idle";
    paintDictate();
    sendError.hidden = false;
    sendError.textContent = dictateMessage(error);
  }
}

function refreshSendButton(): void {
  const hasText = composerInput.value.trim().length > 0;
  const hasFile = attachmentDrafts.length > 0;
  sendButton.disabled = sendBusy || composerInput.disabled || voiceLive || (!hasText && !hasFile);
  paintDictate();
  paintVoiceButton();
}

type VoiceClip = { data: string; mime: string; duration: number; peaks: number[] };

let voiceGen = 0;
let voiceDown = false;
let voiceLive = false;
let voiceCanceling = false;
let voiceFinishing = false;
let voiceOriginX = 0;
let voiceBegan = 0;
let voiceClockTimer = 0;
let voiceLevelTimer = 0;

for (let index = 0; index < 16; index += 1) {
  voiceHoldWaves.append(document.createElement("i"));
}

function paintVoiceButton(): void {
  voiceButton.disabled = !openId || composerInput.disabled || dictatePhase === "prepare" || dictatePhase === "transcribe" || sendBusy;
  voiceButton.setAttribute("aria-pressed", voiceLive ? "true" : "false");
}

function meterVoice(level: number): void {
  const bars = voiceHoldWaves.querySelectorAll<HTMLElement>("i");
  bars.forEach((bar, index) => {
    const wave = 0.35 + 0.65 * Math.abs(Math.sin(index * 0.9 + level / 17));
    bar.style.height = `${4 + Math.round((Math.max(0, Math.min(100, level)) / 100) * 16 * wave)}px`;
  });
}

function showVoiceHold(canceling: boolean): void {
  composerForm.classList.add("voicing");
  voiceHold.hidden = false;
  voiceHold.classList.toggle("canceling", canceling);
  voiceHoldHint.textContent = t(canceling ? "voiceRelease" : "voiceSlide");
}

function hideVoiceHold(): void {
  composerForm.classList.remove("voicing");
  voiceHold.hidden = true;
  voiceHold.classList.remove("canceling");
  window.clearInterval(voiceClockTimer);
  window.clearInterval(voiceLevelTimer);
  voiceClockTimer = 0;
  voiceLevelTimer = 0;
  paintVoiceButton();
}

function voiceError(error: unknown): string {
  const code = error instanceof Error ? error.message : String(error);
  if (code === "no-mic") {
    return t("voiceNoMic");
  }
  if (code === "no-ffmpeg") {
    return t("voiceUnavailable");
  }
  if (code === "short") {
    return t("voiceShort");
  }
  return t("voiceFailed");
}

async function abandonVoice(): Promise<void> {
  voiceGen += 1;
  voiceDown = false;
  voiceLive = false;
  voiceCanceling = false;
  voiceFinishing = false;
  hideVoiceHold();
  await invoke("voice_cancel").catch(() => undefined);
}

async function beginVoice(gen: number): Promise<void> {
  sendError.hidden = true;
  if (dictatePhase !== "idle") {
    abandonDictation();
    await invoke("dictate_cancel").catch(() => undefined);
  }
  if (micTestOn) {
    await cancelMicTest();
  }
  if (gen !== voiceGen || !voiceDown) {
    return;
  }
  try {
    await invoke("voice_start", { source: microphoneSource() });
  } catch (error) {
    if (gen !== voiceGen) {
      return;
    }
    voiceDown = false;
    voiceLive = false;
    hideVoiceHold();
    sendError.hidden = false;
    sendError.textContent = voiceError(error);
    return;
  }
  if (gen !== voiceGen || !voiceDown) {
    voiceLive = false;
    hideVoiceHold();
    await invoke("voice_cancel").catch(() => undefined);
    return;
  }
  voiceLive = true;
  voiceBegan = performance.now();
  showVoiceHold(voiceCanceling);
  voiceHoldTime.textContent = "0:00";
  paintVoiceButton();
  voiceClockTimer = window.setInterval(() => {
    const elapsed = performance.now() - voiceBegan;
    voiceHoldTime.textContent = voiceClock(elapsed);
    if (elapsed >= 300_000) {
      void finishVoice(false);
    }
  }, 200);
  voiceLevelTimer = window.setInterval(() => {
    void invoke<number>("voice_level").then((level) => {
      if (voiceLive) {
        meterVoice(level);
      }
    }).catch(() => undefined);
  }, 90);
}

function stageVoiceClip(chatId: string, clip: VoiceClip): void {
  const mime = clip.mime || "audio/mp4";
  const blob = new Blob([decodeBase64(clip.data)], { type: mime });
  const previewUrl = URL.createObjectURL(blob);
  const kept = attachmentDrafts.filter((draft) => !draft.voice);
  for (const draft of attachmentDrafts) {
    if (draft.voice && draft.previewUrl) {
      URL.revokeObjectURL(draft.previewUrl);
    }
  }
  attachmentDrafts = [
    ...kept,
    {
      chatId,
      file: new File([blob], "voice.m4a", { type: mime }),
      name: "voice.m4a",
      mime,
      kind: "audio",
      previewUrl,
      voice: true,
      duration: clip.duration,
      peaks: clip.peaks,
    },
  ];
  attachmentEpoch += 1;
  paintAttachment();
  composerInput.focus();
}

async function finishVoice(forceCancel: boolean): Promise<void> {
  if (voiceFinishing || (!voiceLive && !voiceDown)) {
    return;
  }
  voiceFinishing = true;
  const gen = voiceGen;
  const tooShort = !forceCancel && !voiceCanceling && performance.now() - voiceBegan < 500;
  const cancel = forceCancel || voiceCanceling || !voiceLive || tooShort;
  voiceDown = false;
  voiceLive = false;
  voiceCanceling = false;
  hideVoiceHold();
  try {
    if (cancel) {
      await invoke("voice_cancel").catch(() => undefined);
      if (tooShort && gen === voiceGen) {
        sendError.hidden = false;
        sendError.textContent = t("voiceShort");
      }
      return;
    }
    const clip = await invoke<VoiceClip>("voice_stop");
    if (gen !== voiceGen || !openId) {
      return;
    }
    stageVoiceClip(openId, clip);
  } catch (error) {
    if (gen === voiceGen) {
      sendError.hidden = false;
      sendError.textContent = voiceError(error);
    }
  } finally {
    voiceFinishing = false;
  }
}

voiceButton.addEventListener("pointerdown", (event) => {
  if (event.button !== 0 || voiceButton.disabled || voiceLive || voiceDown) {
    return;
  }
  event.preventDefault();
  voiceButton.setPointerCapture(event.pointerId);
  voiceDown = true;
  voiceCanceling = false;
  voiceOriginX = event.clientX;
  const gen = ++voiceGen;
  void beginVoice(gen);
});

voiceButton.addEventListener("pointermove", (event) => {
  if (!voiceDown) {
    return;
  }
  const rtl = document.documentElement.dir === "rtl";
  const delta = rtl ? event.clientX - voiceOriginX : voiceOriginX - event.clientX;
  const canceling = delta > 72;
  if (canceling === voiceCanceling) {
    return;
  }
  voiceCanceling = canceling;
  if (voiceLive) {
    showVoiceHold(canceling);
  }
});

voiceButton.addEventListener("pointerup", () => {
  if (voiceFinishing) {
    return;
  }
  if (!voiceLive) {
    voiceDown = false;
    voiceGen += 1;
    return;
  }
  void finishVoice(false);
});

voiceButton.addEventListener("pointercancel", () => {
  void finishVoice(true);
});

voiceButton.addEventListener("contextmenu", (event) => {
  event.preventDefault();
});

function probeDuration(url: string): Promise<number> {
  if (!url) {
    return Promise.resolve(0);
  }
  const audio = document.createElement("audio");
  audio.preload = "metadata";
  return new Promise((resolve) => {
    const finish = (value: number): void => {
      audio.onloadedmetadata = null;
      audio.onerror = null;
      audio.removeAttribute("src");
      audio.load();
      resolve(value);
    };
    audio.onloadedmetadata = () => finish(Number.isFinite(audio.duration) ? Math.round(audio.duration * 1000) : 0);
    audio.onerror = () => finish(0);
    audio.src = url;
  });
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
    if (draft.voice && draft.previewUrl) {
      item.append(voiceCard("", draft.duration || 0, draft.previewUrl, "", draft.peaks || []));
    } else if ((draft.kind === "image" || draft.kind === "video") && draft.previewUrl) {
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
  stopVoicePlayback();
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
  const voices = attachmentDrafts.filter((draft) => draft.voice);
  const rest = attachmentDrafts.filter((draft) => !draft.voice);
  const prior = imageBatch && rest.every((draft) => draft.kind === "image") ? rest : [];
  if (prior.length === 0) {
    for (const draft of rest) {
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
      previewUrl: kind === "image" || kind === "video" || kind === "audio" ? URL.createObjectURL(file) : "",
    };
  });
  attachmentDrafts = [...(imageBatch ? [...prior, ...added] : added), ...voices];
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

async function submitComposer(silent = false): Promise<void> {
  if (sendBusy || !openId || composerInput.disabled) {
    return;
  }
  stopVoicePlayback();
  sendNote.hidden = true;
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
  hideStickerSuggest();
  if (drafts.length > 0) {
    attachmentDrafts = [];
    attachmentEpoch += 1;
    paintAttachment();
  }
  sendBusy = false;
  sendError.hidden = true;
  refreshSendButton();
  const media = drafts.filter((draft) => !draft.voice);
  const album = media.length > 1 && media.every((draft) => draft.kind === "image")
    ? `${Date.now()}${Math.floor(Math.random() * 100000).toString().padStart(5, "0")}`
    : "";
  const albumUrls = album ? media.map((draft) => draft.previewUrl) : [];
  let mediaIndex = 0;
  for (let index = 0; index < drafts.length; index += 1) {
    const draft = drafts[index];
    if (draft.voice) {
      await queueOutgoing({
        localId: `local-${Date.now()}-${outgoingSeq += 1}`,
        chatId,
        text: t("voiceMessage"),
        reply: text ? null : reply,
        time: Date.now(),
        state: "sending",
        error: "",
        knownIds: chatId === openId ? paintedIds : "",
        kind: "audio",
        previewUrl: draft.previewUrl,
        duration: draft.duration || 0,
        peaks: draft.peaks,
        voice: true,
        file: {
          name: draft.name,
          mime: draft.mime,
          data: encoded[index],
        },
        silent,
      });
      continue;
    }
    const preview = draft.kind === "image" || draft.kind === "video";
    const duration = draft.kind === "audio" ? await probeDuration(draft.previewUrl) : 0;
    const seq = album ? mediaIndex + 1 : 0;
    mediaIndex += 1;
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
      duration,
      albumUrls: seq === 1 ? albumUrls : undefined,
      file: {
        name: draft.name,
        mime: draft.mime,
        data: encoded[index],
        groupId: album,
        groupSeq: seq,
        groupCount: album ? media.length : 0,
      },
      silent,
    }, seq <= 1);
  }
  if (text) {
    startSend(chatId, text, reply, silent);
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
  interruptAutoReply();
  scheduleStickerSuggest();
});

let sendHold: number | null = null;
let suppressSendClick = false;

function closeSendMenu(): void {
  sendMenu.hidden = true;
}

function openSendMenu(): void {
  if (sendButton.disabled) {
    return;
  }
  closeMsgMenu();
  sendMenu.hidden = false;
  const button = sendButton.getBoundingClientRect();
  const width = sendMenu.offsetWidth;
  const height = sendMenu.offsetHeight;
  let left = button.right - width;
  let top = button.top - height - 8;
  if (left < 8) {
    left = 8;
  }
  if (top < 8) {
    top = button.bottom + 8;
  }
  sendMenu.style.left = `${left}px`;
  sendMenu.style.top = `${top}px`;
}

function cancelSendHold(): void {
  if (sendHold !== null) {
    window.clearTimeout(sendHold);
    sendHold = null;
  }
}

sendButton.addEventListener("pointerdown", (event) => {
  if (sendButton.disabled || event.button !== 0) {
    return;
  }
  cancelSendHold();
  sendHold = window.setTimeout(() => {
    sendHold = null;
    suppressSendClick = true;
    openSendMenu();
  }, 450);
});
sendButton.addEventListener("pointerup", () => {
  cancelSendHold();
  queueMicrotask(() => {
    suppressSendClick = false;
  });
});
sendButton.addEventListener("pointerleave", cancelSendHold);
sendButton.addEventListener("pointercancel", cancelSendHold);
sendButton.addEventListener("contextmenu", (event) => {
  event.preventDefault();
  cancelSendHold();
  if (sendButton.disabled) {
    return;
  }
  openSendMenu();
});

sendMenu.addEventListener("click", (event) => {
  event.stopPropagation();
  const choice = (event.target as HTMLElement).closest("button")?.dataset.send;
  closeSendMenu();
  if (choice === "quiet") {
    sendError.hidden = false;
    sendError.textContent = t("sendQuietUnsupported");
    return;
  }
  if (choice === "normal") {
    void sendComposer(false);
  }
});

autoReplyButton.addEventListener("click", () => {
  const chatId = autoReplyButton.dataset.chat || openId;
  if (!chatId || chatId !== openId || autoReplyButton.disabled) {
    return;
  }
  if (autoReplyOn(chatId)) {
    delete autoReplyState[chatId];
  } else {
    autoReplyState[chatId] = true;
  }
  saveAutoReply();
  paintAutoReply(chatId);
});

document.querySelector<HTMLFormElement>("#composer")!.addEventListener("submit", (event) => {
  event.preventDefault();
  if (suppressSendClick) {
    suppressSendClick = false;
    return;
  }
  closeSendMenu();
  void sendComposer(false);
});

async function sendComposer(silent = false): Promise<void> {
  if (!(await finishDictation())) {
    return;
  }
  await submitComposer(silent);
}

dictateButton.addEventListener("click", () => {
  void toggleDictation();
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
  if (!stickerPanel.hidden) {
    closeStickerPanel();
    return;
  }
  hideStickerSuggest();
  stickerPanel.hidden = false;
  stickerOpen.setAttribute("aria-expanded", "true");
  void loadStickerShelf();
});

let suggestTimer = 0;
let suggestTicket = 0;

function hideStickerSuggest(): void {
  suggestTicket += 1;
  window.clearTimeout(suggestTimer);
  stickerSuggest.hidden = true;
  stickerSuggest.replaceChildren();
}

function scheduleStickerSuggest(): void {
  window.clearTimeout(suggestTimer);
  const text = composerInput.value.trim();
  if (!openId || composerInput.disabled || text.length === 0 || !stickerPanel.hidden) {
    hideStickerSuggest();
    return;
  }
  suggestTimer = window.setTimeout(() => {
    void loadStickerSuggest(text);
  }, 160);
}

async function loadStickerSuggest(text: string): Promise<void> {
  const ticket = suggestTicket + 1;
  suggestTicket = ticket;
  let stickers: Array<{ packageId: string; stickerId: string; animated: boolean }> = [];
  try {
    const payload = await invoke<{ stickers?: unknown }>("suggest_stickers", {
      query: text.slice(0, 80),
      language: resolvedLanguage(),
    });
    if (Array.isArray(payload.stickers)) {
      for (const item of payload.stickers) {
        if (!item || typeof item !== "object") {
          continue;
        }
        const row = item as Record<string, unknown>;
        const packageId = String(row.packageId || "");
        const stickerId = String(row.stickerId || "");
        if (!/^\d{1,16}$/.test(packageId) || !/^\d{1,16}$/.test(stickerId)) {
          continue;
        }
        stickers.push({ packageId, stickerId, animated: Boolean(row.animated) });
      }
    }
  } catch {
    stickers = [];
  }
  if (ticket !== suggestTicket || composerInput.value.trim() !== text || !stickerPanel.hidden) {
    return;
  }
  stickerSuggest.replaceChildren();
  stickerSuggest.setAttribute("aria-label", t("stickers"));
  if (stickers.length === 0) {
    stickerSuggest.hidden = true;
    return;
  }
  for (const sticker of stickers) {
    const button = document.createElement("button");
    button.type = "button";
    const img = document.createElement("img");
    img.alt = "";
    img.src = stickerImageUrl(sticker.stickerId, sticker.animated);
    img.addEventListener("error", () => {
      img.src = stickerImageUrl(sticker.stickerId, false);
    }, { once: true });
    button.append(img);
    button.addEventListener("click", () => {
      if (!openId || composerInput.disabled) {
        return;
      }
      hideStickerSuggest();
      composerInput.value = "";
      refreshSendButton();
      interruptAutoReply();
      startSticker(openId, sticker);
    });
    stickerSuggest.append(button);
  }
  stickerSuggest.hidden = false;
}

stickerTabMine.addEventListener("click", () => {
  stickerTab = "mine";
  stickerNoteText = "";
  stickerPackId = stickerList()[0]?.id || "";
  paintStickerPanel();
  const selected = selectedStickerPack();
  if (selected) {
    void fillStickerPack(selected);
  }
});

stickerTabShop.addEventListener("click", () => {
  stickerTab = "shop";
  stickerNoteText = "";
  stickerPackId = stickerList()[0]?.id || "";
  paintStickerPanel();
  const selected = selectedStickerPack();
  if (selected) {
    void fillStickerPack(selected);
  }
});

stickerBuy.addEventListener("click", () => {
  void buySelectedSticker();
});

stickerStoreLink.addEventListener("click", () => {
  void invoke("open_link", { url: stickerStoreUrl() }).catch((error: unknown) => {
    showStickerNote(error instanceof Error ? error.message : String(error));
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
  sheetList.hidden = true;
  sheetAbout.hidden = true;
  sheetLanguage.hidden = true;
  sheetAgent.hidden = true;
  sheetChatLlm.hidden = true;
  sheetProfile.hidden = true;
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
  sheetAgent.hidden = true;
  sheetChatLlm.hidden = true;
  sheetProfile.hidden = true;
  sheet.hidden = false;
}

type ListedDevice = { id: string; name: string; detail: string };
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
  sheetAgent.hidden = true;
  sheetChatLlm.hidden = true;
  sheetProfile.hidden = true;
  sheet.hidden = false;
}

function refreshLocale(): void {
  applyI18n();
  if (!stickerPanel.hidden) {
    paintStickerPanel();
  }
  paintDictate();
  if (voiceLive) {
    voiceHoldHint.textContent = t(voiceCanceling ? "voiceRelease" : "voiceSlide");
  }
  paintMicTest();
  paintLanguageCurrent();
  paintAutoReply();
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
    } else if (!sheetAgent.hidden) {
      sheetTitle.textContent = t("agent");
    } else if (!sheetChatLlm.hidden) {
      const row = rows.find((chat) => chat.id === chatLlmEditing);
      sheetTitle.textContent = row?.name || t("chatLlm");
    } else if (!sheetProfile.hidden) {
      sheetTitle.textContent = t("profile");
    } else if (!sheetSettings.hidden) {
      sheetTitle.textContent = t("settings");
      paintTextSize();
      paintNotificationVolume();
      paintNotificationTone();
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
  if (!lightbox.hidden) {
    paintLightboxChrome();
  }
  paintPeople();
  paintRecommendations();
  paintGroupPicks();
  if (railMode === "agent") {
    paintAgent();
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
  const selectable = list === devicesInput;
  const chosen = selectable ? chosenMicrophone(devices) : "";
  for (const device of devices) {
    const item = document.createElement("li");
    const name = document.createElement("span");
    name.textContent = device.name;
    const detail = document.createElement("span");
    detail.className = "device-detail";
    detail.textContent = device.detail;
    if (!selectable) {
      item.append(name);
      if (device.detail) {
        item.append(detail);
      }
      list.append(item);
      continue;
    }
    item.className = "device-selectable";
    const button = document.createElement("button");
    button.type = "button";
    button.className = "device-choice";
    button.setAttribute("aria-pressed", device.id === chosen ? "true" : "false");
    button.append(name);
    if (device.detail) {
      button.append(detail);
    }
    button.addEventListener("click", () => {
      localStorage.setItem(MICROPHONE_KEY, device.id);
      if (listedDevices) {
        fillDevices(devicesInput, listedDevices.inputs, t("noMics"));
      }
    });
    item.append(button);
    list.append(item);
  }
}

function selectedFont(): FontId {
  const value = fontFamily.value;
  return (FONT_IDS as readonly string[]).includes(value) ? (value as FontId) : "noto";
}

function paintTextSize(): void {
  const index = Number(textSize.value);
  const label = t(TEXT_SIZE_KEYS[index] ?? "textDefault");
  textSizeLabel.textContent = label;
  textSize.setAttribute("aria-valuetext", label);
}

function syncAppearanceControls(): void {
  const appearance = readAppearance();
  fontFamily.value = appearance.font;
  textSize.value = String(appearance.scaleIndex);
  for (const option of fontFamily.options) {
    if ((FONT_IDS as readonly string[]).includes(option.value)) {
      option.style.fontFamily = fontStack(option.value as FontId);
    }
  }
  paintTextSize();
}

async function showSettings(): Promise<void> {
  sheetTitle.textContent = t("settings");
  sheetBody.hidden = true;
  sheetSettings.hidden = false;
  syncAppearanceControls();
  paintNotificationVolume();
  paintNotificationTone();
  sheetList.hidden = true;
  sheetAbout.hidden = true;
  sheetLanguage.hidden = true;
  sheetAgent.hidden = true;
  sheetChatLlm.hidden = true;
  sheetProfile.hidden = true;
  sheet.hidden = false;
  paintLanguageCurrent();
  paintProfileCurrent();
  paintAgentCurrent();
  void invoke<AgentConfig>("get_agent_settings")
    .then((settings) => {
      agentConfig = { ...EMPTY_AGENT, ...settings };
      paintAgentCurrent();
      if (railMode === "agent") {
        paintAgent();
      }
    })
    .catch(() => undefined);
  devicesNote.hidden = true;
  fillDevices(devicesOutput, [], t("looking"));
  fillDevices(devicesInput, [], t("looking"));
  fillDevices(devicesCamera, [], t("looking"));
  minimizeToTray.disabled = true;
  keepOnTop.disabled = true;
  focusOnMessageBox.disabled = true;
  try {
    const settings = await invoke<{ minimizeToTray: boolean; alwaysOnTop: boolean; focusOnMessage: boolean }>("get_settings");
    minimizeToTray.checked = settings.minimizeToTray;
    keepOnTop.checked = settings.alwaysOnTop;
    focusOnMessage = settings.focusOnMessage;
    focusOnMessageBox.checked = settings.focusOnMessage;
  } catch (error) {
    setStatusText(error instanceof Error ? error.message : String(error));
  }
  minimizeToTray.disabled = false;
  keepOnTop.disabled = false;
  focusOnMessageBox.disabled = false;
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

type AccountCard = {
  id?: string;
  name?: string;
  status?: string;
  picture?: string;
  cover?: string;
};

function profileSay(text: string): void {
  profileNote.hidden = text.length === 0;
  profileNote.textContent = text;
}

function paintProfileCurrent(): void {
  profileCurrent.textContent = myName.trim();
}

function paintRailMe(): void {
  const label = myName.trim();
  railInitial.textContent = (label[0] || "L").toUpperCase();
  if (!myPicture) {
    railPhoto.hidden = true;
    railPhoto.removeAttribute("src");
    railInitial.hidden = false;
    return;
  }
  railInitial.hidden = true;
  railPhoto.hidden = false;
  railPhoto.onerror = () => {
    railPhoto.hidden = true;
    railInitial.hidden = false;
  };
  if (railPhoto.getAttribute("src") !== myPicture) {
    railPhoto.src = myPicture;
  }
}

function paintProfileMedia(): void {
  const label = myName.trim();
  profileFaceLetter.textContent = (label[0] || "L").toUpperCase();
  profilePhotoRemove.hidden = myPicture.length === 0;
  if (myPicture) {
    profileFaceImg.hidden = false;
    profileFaceLetter.hidden = true;
    profileFaceImg.onerror = () => {
      profileFaceImg.hidden = true;
      profileFaceLetter.hidden = false;
    };
    if (profileFaceImg.getAttribute("src") !== myPicture) {
      profileFaceImg.src = myPicture;
    }
  } else {
    profileFaceImg.hidden = true;
    profileFaceImg.removeAttribute("src");
    profileFaceLetter.hidden = false;
  }
  if (myCover) {
    profileCoverImg.hidden = false;
    profileCoverImg.onerror = () => {
      profileCoverImg.hidden = true;
    };
    if (profileCoverImg.getAttribute("src") !== myCover) {
      profileCoverImg.src = myCover;
    }
  } else {
    profileCoverImg.hidden = true;
    profileCoverImg.removeAttribute("src");
  }
}

function fillProfileForm(): void {
  profileNameInput.value = myName;
  profileStatusInput.value = myStatus;
  paintProfileMedia();
}

function rememberAccount(card: AccountCard, fields: boolean): void {
  if (typeof card.name === "string") {
    myName = card.name;
  }
  if (typeof card.status === "string") {
    myStatus = card.status;
  }
  if (typeof card.picture === "string") {
    myPicture = card.picture;
  }
  if (typeof card.cover === "string") {
    myCover = card.cover;
  }
  paintRailMe();
  paintProfileCurrent();
  if (fields) {
    fillProfileForm();
  } else {
    paintProfileMedia();
  }
}

function profileLock(on: boolean): void {
  profileBusy = on;
  profileSave.disabled = on;
  profileCancel.disabled = on;
  profilePhotoChange.disabled = on;
  profilePhotoRemove.disabled = on || myPicture.length === 0;
  profileCoverChange.disabled = on;
  profileNameInput.disabled = on;
  profileStatusInput.disabled = on;
}

async function showProfile(fromSettings: boolean): Promise<void> {
  profileFromSettings = fromSettings;
  sheetTitle.textContent = t("profile");
  sheetBody.hidden = true;
  sheetSettings.hidden = true;
  sheetList.hidden = true;
  sheetAbout.hidden = true;
  sheetLanguage.hidden = true;
  sheetAgent.hidden = true;
  sheetChatLlm.hidden = true;
  sheetProfile.hidden = false;
  sheet.hidden = false;
  profileSay("");
  fillProfileForm();
  profileLock(true);
  try {
    rememberAccount(await invoke<AccountCard>("get_my_profile"), true);
  } catch (error) {
    profileSay(error instanceof Error ? error.message : String(error));
  } finally {
    profileLock(false);
  }
}

function leaveProfile(): void {
  sheetProfile.hidden = true;
  profileSay("");
  if (profileFromSettings) {
    sheetSettings.hidden = false;
    sheetTitle.textContent = t("settings");
    paintProfileCurrent();
    return;
  }
  sheet.hidden = true;
}

async function saveProfile(): Promise<void> {
  if (profileBusy) {
    return;
  }
  const name = profileNameInput.value.trim();
  const status = profileStatusInput.value.trim();
  if (!name) {
    profileSay(t("profileNameRequired"));
    profileNameInput.focus();
    return;
  }
  if ([...name].length > 20) {
    profileSay(t("nameTooLong"));
    return;
  }
  if ([...status].length > 500) {
    profileSay(t("statusTooLong"));
    return;
  }
  profileLock(true);
  profileSay("");
  try {
    rememberAccount(await invoke<AccountCard>("set_my_profile", { name, status }), true);
    leaveProfile();
  } catch (error) {
    profileSay(error instanceof Error ? error.message : String(error));
    profileLock(false);
  }
}

function fileAsBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const raw = String(reader.result || "");
      const comma = raw.indexOf(",");
      resolve(comma >= 0 ? raw.slice(comma + 1) : raw);
    };
    reader.onerror = () => reject(reader.error ?? new Error("that photo could not be read"));
    reader.readAsDataURL(file);
  });
}

async function useProfileFile(file: File): Promise<void> {
  profileLock(true);
  profileSay("");
  try {
    const data = await fileAsBase64(file);
    const command = profilePick === "cover" ? "set_my_cover" : "set_my_picture";
    rememberAccount(await invoke<AccountCard>(command, { data }), false);
  } catch (error) {
    profileSay(error instanceof Error ? error.message : String(error));
  } finally {
    profileLock(false);
  }
}

railMe.addEventListener("click", () => {
  closeMenu();
  void showProfile(false);
});

profileOpen.addEventListener("click", () => {
  void showProfile(true);
});

profileSave.addEventListener("click", () => {
  void saveProfile();
});

profileCancel.addEventListener("click", () => {
  if (!profileBusy) {
    leaveProfile();
  }
});

profileNameInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter") {
    event.preventDefault();
    void saveProfile();
  }
});

profilePhotoChange.addEventListener("click", () => {
  profilePick = "photo";
  profileFile.click();
});

profileCoverChange.addEventListener("click", () => {
  profilePick = "cover";
  profileFile.click();
});

profilePhotoRemove.addEventListener("click", () => {
  if (profileBusy || !myPicture) {
    return;
  }
  profileLock(true);
  profileSay("");
  void invoke<AccountCard>("clear_my_picture")
    .then((card) => rememberAccount(card, false))
    .catch((error: unknown) => {
      profileSay(error instanceof Error ? error.message : String(error));
    })
    .finally(() => {
      profileLock(false);
    });
});

profileFile.addEventListener("change", () => {
  const file = profileFile.files?.[0];
  profileFile.value = "";
  if (file) {
    void useProfileFile(file);
  }
});

languageOpen.addEventListener("click", () => {
  showLanguage();
});

languageSave.addEventListener("click", () => {
  sheetLanguage.hidden = true;
  sheetAgent.hidden = true;
  sheetChatLlm.hidden = true;
  sheetProfile.hidden = true;
  sheetSettings.hidden = false;
  setLanguage(pendingLanguage);
  sheetTitle.textContent = t("settings");
});

languageCancel.addEventListener("click", () => {
  sheet.hidden = true;
  void cancelMicTest();
});

agentOpen.addEventListener("click", () => {
  agentSettingsFromAgent = false;
  void showAgentSettings();
});

agentCancel.addEventListener("click", () => {
  dismissAgentSettings();
});

function dismissChatLlm(): void {
  sheetChatLlm.hidden = true;
  sheetProfile.hidden = true;
  chatLlmNote.hidden = true;
  chatLlmEditing = "";
  sheet.hidden = true;
}

async function showChatLlm(chatId: string): Promise<void> {
  if (!chatId) {
    return;
  }
  chatLlmEditing = chatId;
  const row = rows.find((chat) => chat.id === chatId);
  sheetTitle.textContent = row?.name || t("chatLlm");
  sheetBody.hidden = true;
  sheetSettings.hidden = true;
  sheetList.hidden = true;
  sheetAbout.hidden = true;
  sheetLanguage.hidden = true;
  sheetAgent.hidden = true;
  sheetChatLlm.hidden = false;
  sheetChatLlm.scrollTop = 0;
  chatLlmNote.hidden = true;
  sheet.hidden = false;
  let saved = chatLlmCache.get(chatId) ?? null;
  try {
    saved = await rememberChatLlm(chatId);
  } catch {
    saved = chatLlmCache.get(chatId) ?? null;
  }
  if (chatLlmEditing !== chatId) {
    return;
  }
  if (!agentConfig.endpoint && !agentConfig.model) {
    try {
      agentConfig = { ...EMPTY_AGENT, ...(await invoke<AgentConfig>("get_agent_settings")) };
    } catch {
      agentConfig = { ...EMPTY_AGENT, ...agentConfig };
    }
  }
  fillChatLlm(saved ?? llmFromAgent(), Boolean(saved));
  void paintChatLlmCopy(chatId);
  chatLlmEndpoint.focus();
}

async function paintChatLlmCopy(chatId: string): Promise<void> {
  chatLlmCopy.replaceChildren();
  const placeholder = document.createElement("option");
  placeholder.value = "";
  placeholder.textContent = t("chatLlmCopyChoose");
  chatLlmCopy.append(placeholder);
  let ids: string[] = [];
  try {
    ids = await invoke<string[]>("list_chat_llm");
  } catch {
    ids = [];
  }
  if (chatLlmEditing !== chatId) {
    return;
  }
  const choices = ids
    .filter((id) => id !== chatId)
    .map((id) => ({ id, name: rows.find((chat) => chat.id === id)?.name || id }))
    .sort((left, right) => left.name.localeCompare(right.name));
  if (choices.length === 0) {
    placeholder.textContent = t("chatLlmCopyEmpty");
    chatLlmCopy.disabled = true;
    chatLlmCopyApply.disabled = true;
    return;
  }
  for (const choice of choices) {
    const option = document.createElement("option");
    option.value = choice.id;
    option.textContent = choice.name;
    chatLlmCopy.append(option);
  }
  chatLlmCopy.disabled = false;
  chatLlmCopyApply.disabled = false;
}

chatLlmCopyApply.addEventListener("click", () => {
  const chatId = chatLlmEditing;
  const source = chatLlmCopy.value;
  if (!chatId || !source || source === chatId) {
    return;
  }
  chatLlmNote.hidden = true;
  chatLlmCopyApply.disabled = true;
  void invoke<ChatLlm | null>("get_chat_llm", { id: source })
    .then((saved) => {
      if (chatLlmEditing !== chatId) {
        return;
      }
      if (!saved) {
        chatLlmNote.hidden = false;
        chatLlmNote.textContent = t("chatLlmCopyEmpty");
        return;
      }
      chatLlmCache.set(source, saved);
      fillChatLlm(saved, false);
      chatLlmClear.disabled = !chatLlmCache.get(chatId);
      const name = rows.find((chat) => chat.id === source)?.name || source;
      chatLlmUnsaved.hidden = false;
      chatLlmUnsaved.textContent = t("chatLlmCopied", { name });
    })
    .catch((error: unknown) => {
      chatLlmNote.hidden = false;
      chatLlmNote.textContent = error instanceof Error ? error.message : String(error);
    })
    .finally(() => {
      chatLlmCopyApply.disabled = chatLlmCopy.disabled;
    });
});

chatLlmButton.addEventListener("click", () => {
  if (!openId || chatLlmButton.disabled) {
    return;
  }
  void showChatLlm(openId);
});

chatLlmCancel.addEventListener("click", () => {
  dismissChatLlm();
});

chatLlmClear.addEventListener("click", () => {
  const chatId = chatLlmEditing;
  if (!chatId) {
    return;
  }
  chatLlmNote.hidden = true;
  chatLlmClear.disabled = true;
  void invoke("clear_chat_llm", { id: chatId })
    .then(() => {
      chatLlmCache.set(chatId, null);
      fillChatLlm(llmFromAgent(), false);
      paintAutoReply(chatId);
    })
    .catch((error: unknown) => {
      chatLlmNote.hidden = false;
      chatLlmNote.textContent = error instanceof Error ? error.message : String(error);
      chatLlmClear.disabled = false;
    });
});

chatLlmSave.addEventListener("click", () => {
  const chatId = chatLlmEditing;
  if (!chatId) {
    return;
  }
  chatLlmNote.hidden = true;
  chatLlmSave.disabled = true;
  const settings: ChatLlm = {
    endpoint: chatLlmEndpoint.value.trim(),
    model: chatLlmModel.value.trim(),
    apiKey: chatLlmKey.value,
    protocol: chatLlmProtocol.value,
    temperature: chatLlmTemperature.value.trim(),
    topP: chatLlmTopP.value.trim(),
    maxTokens: chatLlmMaxTokens.value.trim(),
    thinking: chatLlmThinking.value,
    language: chatLlmLanguage.value,
    extra: chatLlmExtra.value.trim(),
    context: chatLlmContext.value.trim(),
    history: historyField(chatLlmHistory.value),
    markAi: chatLlmMarkAi.checked,
    interrupt: chatLlmInterrupt.checked,
  };
  void invoke("set_chat_llm", { id: chatId, settings })
    .then(() => {
      const saved = { ...settings, apiKey: settings.apiKey.trim() };
      chatLlmCache.set(chatId, saved);
      dismissChatLlm();
      paintAutoReply(chatId);
    })
    .catch((error: unknown) => {
      chatLlmNote.hidden = false;
      chatLlmNote.textContent = error instanceof Error ? error.message : String(error);
    })
    .finally(() => {
      chatLlmSave.disabled = false;
    });
});

agentSave.addEventListener("click", () => {
  agentSettingsNote.hidden = true;
  agentSave.disabled = true;
  const reply = {
    temperature: replyTemperature.value.trim(),
    topP: replyTopP.value.trim(),
    maxTokens: replyMaxTokens.value.trim(),
    thinking: replyThinking.value,
    extra: replyExtra.value.trim(),
    autoContext: agentAutoContext.value.trim(),
  };
  const settings: AgentConfig = {
    endpoint: agentEndpoint.value.trim(),
    model: agentModel.value.trim(),
    apiKey: agentApiKey.value,
    protocol: agentProtocol.value,
    temperature: agentTemperature.value.trim(),
    topP: agentTopP.value.trim(),
    maxTokens: agentMaxTokens.value.trim(),
    thinking: agentThinking.value,
    extra: agentExtra.value.trim(),
    stream: agentStream.checked,
    autoContext: reply.autoContext,
    replyTemperature: reply.temperature,
    replyTopP: reply.topP,
    replyMaxTokens: reply.maxTokens,
    replyThinking: reply.thinking || "off",
    replyExtra: reply.extra,
  };
  void invoke("set_agent_settings", { settings })
    .then(() => invoke("set_reply_settings", { settings: reply }))
    .then(() => {
      agentConfig = { ...settings, apiKey: settings.apiKey.trim() };
      dismissAgentSettings();
      paintAgentCurrent();
      paintAutoReply();
      if (railMode === "agent") {
        paintAgent();
      }
      agentReady = false;
      agentProbed = false;
      ensureAgentProbe();
    })
    .catch((error: unknown) => {
      agentSettingsNote.hidden = false;
      agentSettingsNote.textContent = error instanceof Error ? error.message : String(error);
    })
    .finally(() => {
      agentSave.disabled = false;
    });
});

applyI18n();
paintLanguageCurrent();
paintAgentCurrent();
syncAgentSend();
onLocaleChange(refreshLocale);

railMore.addEventListener("click", (event) => {
  event.stopPropagation();
  railMenu.hidden = !railMenu.hidden;
  railMore.setAttribute("aria-expanded", railMenu.hidden ? "false" : "true");
});

function syncThemeToggle(): void {
  const themeButton = railMenu.querySelector<HTMLButtonElement>("[data-action='theme']");
  if (!themeButton) {
    return;
  }
  themeButton.setAttribute("aria-checked", document.documentElement.dataset.theme === "dark" ? "true" : "false");
}

syncThemeToggle();
document.addEventListener("lineapp-appearance", syncThemeToggle);

railMenu.addEventListener("click", (event) => {
  event.stopPropagation();
  const action = (event.target as HTMLElement).closest("button")?.dataset.action;
  if (!action) {
    return;
  }
  if (action === "theme") {
    toggleTheme();
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
  const target = event.target as Node;
  if (msgMenu.contains(target)) {
    return;
  }
  closeMsgMenu();
  if (sendMenu.contains(target) || sendButton.contains(target)) {
    return;
  }
  closeSendMenu();
});

document.querySelector<HTMLButtonElement>("#sheet-close")!.addEventListener("click", () => {
  sheet.hidden = true;
  void cancelMicTest();
});

let micTestOn = false;
let micTestBusy = false;
let micTestTimer = 0;
let micTestLevelTimer = 0;
let micTestUrl = "";

function paintMicTest(): void {
  micTestButton.disabled = micTestBusy;
  micTestButton.textContent = micTestBusy ? t("dictateWorking") : micTestOn ? t("stopMicTest") : t("testMicrophone");
  if (!micTestOn) {
    micTestMeter.hidden = true;
  }
}

function stopMicTestMeter(): void {
  window.clearInterval(micTestLevelTimer);
  micTestLevelTimer = 0;
  micTestMeter.hidden = true;
  micTestMeter.classList.remove("hearing");
  micTestFill.style.width = "0%";
}

function playMicTest(audio: string): void {
  if (micTestUrl) {
    URL.revokeObjectURL(micTestUrl);
  }
  const binary = atob(audio);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  micTestUrl = URL.createObjectURL(new Blob([bytes], { type: "audio/wav" }));
  micTestPlayer.hidden = false;
  micTestPlayer.src = micTestUrl;
  void micTestPlayer.play().catch(() => undefined);
}

async function cancelMicTest(): Promise<void> {
  if (!micTestOn && !micTestBusy) {
    return;
  }
  micTestOn = false;
  window.clearTimeout(micTestTimer);
  stopMicTestMeter();
  paintMicTest();
  await invoke("dictate_cancel").catch(() => undefined);
}

async function finishMicTest(): Promise<void> {
  if (!micTestOn || micTestBusy) {
    return;
  }
  micTestOn = false;
  micTestBusy = true;
  window.clearTimeout(micTestTimer);
  stopMicTestMeter();
  paintMicTest();
  try {
    const sample = await invoke<{ text: string; audio: string }>("mic_test_stop", { language: intlLocale() });
    micTestResult.hidden = false;
    micTestResult.textContent = sample.text ? t("micTestHeard", { text: sample.text }) : t("dictateEmpty");
    if (sample.audio) {
      playMicTest(sample.audio);
    }
  } catch (error) {
    micTestResult.hidden = false;
    micTestResult.textContent = dictateMessage(error);
  } finally {
    micTestBusy = false;
    paintMicTest();
  }
}

async function startMicTest(): Promise<void> {
  if (micTestOn || micTestBusy) {
    return;
  }
  if (dictatePhase !== "idle") {
    abandonDictation();
  }
  micTestResult.hidden = true;
  micTestPlayer.pause();
  micTestPlayer.hidden = true;
  try {
    await invoke("dictate_start", { source: microphoneSource() });
    micTestOn = true;
    paintMicTest();
    micTestMeter.hidden = false;
    micTestLevelTimer = window.setInterval(() => {
      void invoke<number>("dictate_level").then((level) => {
        if (!micTestOn) {
          return;
        }
        const clamped = Math.max(0, Math.min(100, Math.round(level)));
        micTestFill.style.width = `${clamped}%`;
        micTestMeter.classList.toggle("hearing", clamped >= 22);
      }).catch(() => undefined);
    }, 100);
    micTestTimer = window.setTimeout(() => {
      void finishMicTest();
    }, 12_000);
  } catch (error) {
    micTestOn = false;
    paintMicTest();
    micTestResult.hidden = false;
    micTestResult.textContent = dictateMessage(error);
  }
}

micTestButton.addEventListener("click", () => {
  if (micTestOn) {
    void finishMicTest();
    return;
  }
  void startMicTest();
});

minimizeToTray.addEventListener("change", () => {
  const enabled = minimizeToTray.checked;
  void invoke("set_minimize_to_tray", { enabled }).catch((error: unknown) => {
    minimizeToTray.checked = !enabled;
    setStatusText(error instanceof Error ? error.message : String(error));
  });
});

keepOnTop.addEventListener("change", () => {
  const enabled = keepOnTop.checked;
  void invoke("set_always_on_top", { enabled }).catch((error: unknown) => {
    keepOnTop.checked = !enabled;
    setStatusText(error instanceof Error ? error.message : String(error));
  });
});

focusOnMessageBox.addEventListener("change", () => {
  const enabled = focusOnMessageBox.checked;
  focusOnMessage = enabled;
  void invoke("set_focus_on_message", { enabled }).catch((error: unknown) => {
    focusOnMessage = !enabled;
    focusOnMessageBox.checked = !enabled;
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
  if (event.key === "Escape" && (voiceLive || voiceDown)) {
    void finishVoice(true);
    return;
  }
  if (event.key === "Escape" && !msgMenu.hidden) {
    closeMsgMenu();
    return;
  }
  if (event.key === "Escape" && reactionPicker && lightbox.hidden) {
    closeReactionPicker();
    return;
  }
  if (event.key === "Escape" && !sheet.hidden && lightbox.hidden) {
    agentSettingsFromAgent = false;
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
      duration: message.duration || prior.duration,
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

function assignThreadScroll(top: number): void {
  const max = Math.max(0, threadBody.scrollHeight - threadBody.clientHeight);
  const next = Math.max(0, Math.min(top, max));
  expectedTop = next;
  if (Math.abs(threadBody.scrollTop - next) < 1) {
    return;
  }
  placingThread = true;
  threadBody.scrollTop = next;
  requestAnimationFrame(() => {
    placingThread = false;
  });
}

function stopLayoutWatch(): void {
  layoutWatch?.stop();
  layoutWatch = null;
}

function watchThreadLayout(token: number, kind: "tail" | "anchor", pin: () => void): void {
  stopLayoutWatch();
  threadBody.classList.add("tail-lock");
  const observer = new ResizeObserver(() => {
    if (token !== placeToken) {
      if (layoutWatch?.token === token) {
        stopLayoutWatch();
      }
      return;
    }
    pin();
  });
  const observeRows = (): void => {
    observer.disconnect();
    for (const child of threadBody.children) {
      observer.observe(child);
    }
  };
  const onMedia = (event: Event): void => {
    if (token !== placeToken) {
      if (layoutWatch?.token === token) {
        stopLayoutWatch();
      }
      return;
    }
    if (event.target instanceof Node && threadBody.contains(event.target)) {
      pin();
    }
  };
  threadBody.addEventListener("load", onMedia, true);
  threadBody.addEventListener("loadedmetadata", onMedia, true);
  const mutations = new MutationObserver(() => {
    if (token !== placeToken) {
      if (layoutWatch?.token === token) {
        stopLayoutWatch();
      }
      return;
    }
    observeRows();
    pin();
  });
  observeRows();
  mutations.observe(threadBody, { childList: true, subtree: true });
  layoutWatch = {
    token,
    kind,
    stop: () => {
      observer.disconnect();
      mutations.disconnect();
      threadBody.removeEventListener("load", onMedia, true);
      threadBody.removeEventListener("loadedmetadata", onMedia, true);
      threadBody.classList.remove("tail-lock");
    },
  };
}

function pinTail(token: number): void {
  if (token !== placeToken) {
    if (layoutWatch?.token === token) {
      stopLayoutWatch();
    }
    return;
  }
  assignThreadScroll(threadBody.scrollHeight);
}

function scrollThreadToEnd(): void {
  const token = ++placeToken;
  userMoved = false;
  watchThreadLayout(token, "tail", () => pinTail(token));
  pinTail(token);
  requestAnimationFrame(() => pinTail(token));
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
    const nextTop = tail <= threadBody.clientHeight + 1 ? threadBody.scrollHeight : next;
    assignThreadScroll(nextTop);
  };
  watchThreadLayout(token, "anchor", pin);
  pin();
  requestAnimationFrame(pin);
}

threadBody.addEventListener("scroll", () => {
  closeReactionPicker();
  if (placingThread || (expectedTop >= 0 && Math.abs(threadBody.scrollTop - expectedTop) < 2)) {
    return;
  }
  expectedTop = -1;
  if (threadNearBottom()) {
    userMoved = false;
    unreadFocus = "";
    if (layoutWatch?.kind !== "tail") {
      const token = ++placeToken;
      watchThreadLayout(token, "tail", () => pinTail(token));
    }
    return;
  }
  placeToken += 1;
  stopLayoutWatch();
  userMoved = true;
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
    if (update.incoming && update.thread?.id && update.thread.text?.trim()) {
      pushAutoTurn(update.id, update.thread);
      queueAutoReply(update.id, update.thread.id);
    }
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
    requestAttention();
    if (focusOnMessage) {
      void invoke("focus_for_message").catch(() => {});
    }
  }
  if (openAdvanced && openId) {
    void invoke("load_history", { id: openId });
  }
}

async function logout(): Promise<void> {
  signedIn = false;
  trayBadge = -1;
  myName = "";
  myStatus = "";
  myPicture = "";
  myCover = "";
  myMid = "";
  paintRailMe();
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
  clearAgentLog();
  agentReady = false;
  agentProbed = false;
  agentProbeStarted = false;
  agentProbeError = "";
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

void invoke<{ focusOnMessage: boolean }>("get_settings")
  .then((settings) => {
    focusOnMessage = settings.focusOnMessage;
  })
  .catch(() => {});

void start();
