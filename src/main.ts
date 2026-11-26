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
  text: string;
  sealed: boolean;
  time: number;
  sticker?: string;
  stickerFallback?: string;
};

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

let shownUrl = "";
let rows: ChatRow[] = [];
let filter = "all";
let selected = 0;
let openId = "";
let historyToken = 0;
let pollTimer = 0;
let pollBusy = false;

type PollUpdate = {
  id: string;
  lastMessage: string;
  time: number;
  sealed: boolean;
  sticker?: string;
  stickerFallback?: string;
  unread: number | null;
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

function showThread(chat: ChatRow | undefined): void {
  if (!chat) {
    thread.hidden = true;
    openId = "";
    composerInput.disabled = true;
    sendError.hidden = true;
    return;
  }
  thread.hidden = false;
  const count = chat.members === null ? "" : ` (${chat.members})`;
  const mute = chat.muted ? " 🔇" : "";
  threadHead.textContent = `${chat.name}${count}${mute}`;
  composerInput.disabled = false;
  if (chat.id === openId) {
    return;
  }
  openId = chat.id;
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
  payload.messages.forEach((message) => threadBody.append(messageBubble(message)));
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

function messageBubble(message: HistoryMessage): HTMLElement {
  const bubble = document.createElement("div");
  bubble.className = message.mine ? "bubble mine" : "bubble";
  if (message.name && !message.mine) {
    const who = document.createElement("div");
    who.className = "who";
    who.textContent = message.name;
    bubble.append(who);
  }
  if (message.sticker) {
    bubble.append(stickerImage(message.sticker, message.stickerFallback || "", "sticker", () => {
      const text = document.createElement("div");
      text.textContent = message.text || "Sticker";
      bubble.append(text);
    }));
  } else {
    const text = document.createElement("div");
    text.textContent = message.text;
    bubble.append(text);
  }
  if (message.time) {
    const when = document.createElement("div");
    when.className = "when";
    when.textContent = clock(message.time);
    bubble.append(when);
  }
  return bubble;
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
    if (chat.muted) {
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
    preview.textContent = chat.lastMessage || "No messages";
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

function fallback(name: string): HTMLElement {
  const circle = document.createElement("span");
  circle.className = "avatar-fallback";
  circle.textContent = (name.trim()[0] || "?").toUpperCase();
  const hue = [...name].reduce((sum, char) => sum + char.charCodeAt(0), 0) % 360;
  circle.style.background = `hsl(${hue} 42% 42%)`;
  return circle;
}

function renderChats(payload: ChatPayload): void {
  login.hidden = true;
  app.hidden = false;
  rows = payload.chats;
  selected = 0;
  openId = "";
  document.body.dataset.friends = String(rows.filter((chat) => chat.kind === "friend").length);
  const profile = payload.profile;
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
  const total = payload.unreadTotal;
  if (total !== null && total > 0) {
    railCount.hidden = false;
    railCount.textContent = String(total);
  } else {
    railCount.hidden = true;
    railCount.textContent = "";
  }
  renderList();
  startPolling();
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
  sheet.hidden = false;
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
    showSheet("Settings", "Settings are not in this window yet.");
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

function startPolling(): void {
  window.clearInterval(pollTimer);
  pollTimer = window.setInterval(() => {
    void pollInbox();
  }, 5000);
}

async function pollInbox(): Promise<void> {
  if (pollBusy || app.hidden || rows.length === 0) {
    return;
  }
  pollBusy = true;
  const times: Record<string, number> = {};
  for (const row of rows) {
    times[row.id] = row.time || 0;
  }
  try {
    await invoke("refresh_inbox", { known: JSON.stringify({ times }) });
  } catch {
    // The next pass tries again.
  } finally {
    pollBusy = false;
  }
}

function applyPoll(payload: PollPayload): void {
  const selectedId = visibleRows()[selected]?.id || openId;
  const openTime = rows.find((row) => row.id === openId)?.time || 0;
  if (payload.unreadTotal !== null && payload.unreadTotal > 0) {
    railCount.hidden = false;
    railCount.textContent = String(payload.unreadTotal);
  } else if (payload.unreadTotal === 0) {
    railCount.hidden = true;
    railCount.textContent = "";
  }
  for (const item of payload.unreads || []) {
    const chat = rows.find((row) => row.id === item.id);
    if (chat) {
      chat.unread = item.unread;
    }
  }
  let openAdvanced = false;
  for (const update of payload.updates || []) {
    const chat = rows.find((row) => row.id === update.id);
    if (!chat) {
      rows.push({
        id: update.id,
        name: update.name || "Direct chat",
        lastMessage: update.lastMessage,
        sealed: update.sealed,
        time: update.time,
        members: update.members ?? null,
        picture: update.picture || "",
        muted: false,
        kind: update.kind || "friend",
        unread: update.unread,
        sticker: update.sticker || "",
        stickerFallback: update.stickerFallback || "",
      });
      continue;
    }
    if (typeof update.unread === "number") {
      chat.unread = update.unread;
    }
    if (update.time > chat.time) {
      const keepTyped = update.sealed
        && Boolean(chat.lastMessage)
        && !chat.lastMessage.startsWith("Letter-sealed")
        && update.time - chat.time < 20000;
      if (!keepTyped) {
        chat.lastMessage = update.lastMessage;
        chat.sealed = update.sealed;
        chat.sticker = update.sticker || "";
        chat.stickerFallback = update.stickerFallback || "";
        if (chat.id === openId && update.time > openTime) {
          openAdvanced = true;
        }
      }
      chat.time = update.time;
    }
  }
  rows.sort((left, right) => right.time - left.time);
  const index = visibleRows().findIndex((row) => row.id === selectedId);
  selected = index >= 0 ? index : 0;
  renderList();
  if (openAdvanced && openId) {
    void invoke("load_history", { id: openId });
  }
}

async function logout(): Promise<void> {
  window.clearInterval(pollTimer);
  closeMenu();
  sheet.hidden = true;
  try {
    await invoke("logout");
  } catch (error) {
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
