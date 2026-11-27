import { invoke } from "@tauri-apps/api/core";
import "./appearance";
import { applyI18n, onLocaleChange, t, type MessageKey } from "./i18n";

type ProfileCard = {
  id: string;
  name: string;
  given?: string;
  status: string;
  picture: string;
  cover: string;
  mine: boolean;
  friend: boolean;
  blocked: boolean;
  favorite?: boolean;
};

const cover = document.querySelector<HTMLElement>("#cover")!;
const coverImg = document.querySelector<HTMLImageElement>("#cover-img")!;
const avatar = document.querySelector<HTMLElement>("#avatar")!;
const nameEl = document.querySelector<HTMLElement>("#profile-name")!;
const nameInput = document.querySelector<HTMLInputElement>("#name-input")!;
const rename = document.querySelector<HTMLButtonElement>("#rename")!;
const favorite = document.querySelector<HTMLButtonElement>("#favorite")!;
const chat = document.querySelector<HTMLButtonElement>("#chat")!;
const statusEl = document.querySelector<HTMLElement>("#profile-status")!;
const report = document.querySelector<HTMLButtonElement>("#report")!;
const fraud = document.querySelector<HTMLElement>("#fraud")!;
const learn = document.querySelector<HTMLButtonElement>("#learn")!;
const note = document.querySelector<HTMLElement>("#note")!;
const confirmBox = document.querySelector<HTMLElement>("#confirm")!;
const confirmText = document.querySelector<HTMLElement>("#confirm-text")!;
const confirmYes = document.querySelector<HTMLButtonElement>("#confirm-yes")!;
const confirmNo = document.querySelector<HTMLButtonElement>("#confirm-no")!;
const actions = document.querySelector<HTMLElement>("#profile-actions")!;
const block = document.querySelector<HTMLButtonElement>("#block")!;
const add = document.querySelector<HTMLButtonElement>("#add")!;

let card: ProfileCard = {
  id: "",
  name: "",
  status: "",
  picture: "",
  cover: "",
  mine: false,
  friend: false,
  blocked: false,
  favorite: false,
  given: "",
};
let busy = false;
let editing = false;

function letter(label: string): HTMLElement {
  const circle = document.createElement("span");
  circle.textContent = (label.trim()[0] || "?").toUpperCase();
  const hue = [...label].reduce((sum, char) => sum + char.charCodeAt(0), 0) % 360;
  circle.style.background = `hsl(${hue} 42% 42%)`;
  return circle;
}

function paintAvatar(label: string, picture: string): void {
  avatar.replaceChildren();
  if (!picture) {
    avatar.append(letter(label));
    return;
  }
  const img = document.createElement("img");
  img.alt = "";
  img.src = picture;
  img.addEventListener("error", () => {
    img.replaceWith(letter(label));
  });
  avatar.append(img);
}

function paintCover(banner: string): void {
  if (!banner) {
    coverImg.hidden = true;
    coverImg.removeAttribute("src");
    return;
  }
  coverImg.hidden = false;
  if (coverImg.src !== banner) {
    coverImg.src = banner;
  }
  coverImg.onerror = () => {
    coverImg.hidden = true;
  };
}

function say(text: string, warn = false, key = ""): void {
  note.hidden = text.length === 0;
  note.textContent = text;
  note.dataset.key = key;
  note.classList.toggle("warn", warn);
}

function sayKey(key: MessageKey, warn = false): void {
  say(t(key), warn, key);
}

function layout(): void {
  const own = card.mine;
  const friend = card.friend && !own;
  document.body.classList.toggle("is-friend", friend);
  report.hidden = own || friend;
  fraud.hidden = own || friend;
  actions.hidden = own;
  block.hidden = friend;
  add.hidden = friend;
  chat.hidden = !friend;
  rename.hidden = !friend || editing;
  nameEl.hidden = editing;
  nameInput.hidden = !editing;
  favorite.hidden = !friend;
  favorite.classList.toggle("is-on", Boolean(card.favorite));
  favorite.setAttribute("aria-pressed", card.favorite ? "true" : "false");
  favorite.setAttribute("aria-label", card.favorite ? t("removeFavorite") : t("favorite"));
  block.setAttribute("aria-label", card.blocked ? t("unblock") : t("block"));
}

function apply(next: ProfileCard): void {
  card = {
    id: next.id || card.id,
    name: next.name || card.name,
    given: next.given || card.given || next.name || card.name,
    status: next.status ?? card.status,
    picture: next.picture || card.picture,
    cover: next.cover || "",
    mine: Boolean(next.mine),
    friend: Boolean(next.friend),
    blocked: Boolean(next.blocked),
    favorite: Boolean(next.favorite),
  };
  const label = card.name || "LINE";
  document.title = label;
  nameEl.textContent = card.name;
  statusEl.hidden = card.status.length === 0 || (card.friend && !card.mine);
  statusEl.textContent = card.status;
  paintCover(card.cover);
  paintAvatar(card.name, card.picture);
  confirmBox.hidden = true;
  layout();
}

function applyProfile(next: ProfileCard): void {
  apply(next);
}

(window as unknown as { __applyProfile?: (next: ProfileCard) => void }).__applyProfile = applyProfile;

const queued = (window as unknown as { __profile?: ProfileCard }).__profile;
if (queued) {
  apply(queued);
}

report.addEventListener("click", () => {
  sayKey("reportingUnavailable", true);
});

learn.addEventListener("click", () => {
  void invoke("open_fraud_help").catch((error: unknown) => {
    say(error instanceof Error ? error.message : String(error), true);
  });
});

function askBlock(): void {
  confirmText.textContent = card.blocked ? t("unblockUser") : t("blockUser");
  confirmYes.textContent = card.blocked ? t("unblock") : t("block");
  confirmBox.hidden = false;
}

confirmNo.addEventListener("click", () => {
  confirmBox.hidden = true;
});

confirmYes.addEventListener("click", () => {
  if (busy || !card.id) {
    return;
  }
  const blocked = !card.blocked;
  busy = true;
  confirmYes.disabled = true;
  void invoke("block_profile", { id: card.id, blocked })
    .then(() => {
      card.blocked = blocked;
      if (blocked) {
        card.friend = false;
      }
      confirmBox.hidden = true;
      layout();
      sayKey(blocked ? "blocked" : "unblocked");
    })
    .catch((error: unknown) => {
      say(error instanceof Error ? error.message : String(error), true);
    })
    .finally(() => {
      busy = false;
      confirmYes.disabled = false;
    });
});

block.addEventListener("click", () => {
  if (!busy) {
    askBlock();
  }
});

add.addEventListener("click", () => {
  if (busy || !card.id) {
    return;
  }
  if (card.friend) {
    sayKey("alreadyFriend");
    return;
  }
  if (card.blocked) {
    sayKey("unblockBeforeAdd", true);
    return;
  }
  busy = true;
  add.disabled = true;
  void invoke("add_profile", { id: card.id })
    .then(() => {
      card.friend = true;
      say("");
      layout();
    })
    .catch((error: unknown) => {
      say(error instanceof Error ? error.message : String(error), true);
    })
    .finally(() => {
      busy = false;
      add.disabled = false;
    });
});

function stopEditing(): void {
  editing = false;
  layout();
}

rename.addEventListener("click", () => {
  if (busy || !card.friend) {
    return;
  }
  editing = true;
  nameInput.value = card.name;
  layout();
  nameInput.focus();
  nameInput.select();
});

nameInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter") {
    event.preventDefault();
    nameInput.blur();
  } else if (event.key === "Escape") {
    nameInput.value = card.name;
    stopEditing();
  }
});

nameInput.addEventListener("blur", () => {
  if (!editing) {
    return;
  }
  const next = nameInput.value.trim();
  if (next === card.name) {
    stopEditing();
    return;
  }
  if (next.length > 20) {
    sayKey("nameTooLong", true);
    stopEditing();
    return;
  }
  busy = true;
  void invoke("rename_profile", { id: card.id, name: next })
    .then(() => {
      card.name = next || card.given || card.name;
      nameEl.textContent = card.name;
      document.title = card.name || "LINE";
      say("");
    })
    .catch((error: unknown) => {
      say(error instanceof Error ? error.message : String(error), true);
    })
    .finally(() => {
      busy = false;
      stopEditing();
    });
});

favorite.addEventListener("click", () => {
  if (busy || !card.id || !card.friend) {
    return;
  }
  const next = !card.favorite;
  card.favorite = next;
  layout();
  busy = true;
  void invoke("favorite_profile", { id: card.id, favorite: next })
    .catch((error: unknown) => {
      card.favorite = !next;
      layout();
      say(error instanceof Error ? error.message : String(error), true);
    })
    .finally(() => {
      busy = false;
    });
});

chat.addEventListener("click", () => {
  if (!card.id) {
    return;
  }
  void invoke("open_friend_chat", { id: card.id, name: card.name, picture: card.picture }).catch((error: unknown) => {
    say(error instanceof Error ? error.message : String(error), true);
  });
});

applyI18n();
onLocaleChange(() => {
  applyI18n();
  layout();
  if (!confirmBox.hidden) {
    confirmText.textContent = card.blocked ? t("unblockUser") : t("blockUser");
    confirmYes.textContent = card.blocked ? t("unblock") : t("block");
  }
  const key = note.dataset.key || "";
  if (key && !note.hidden) {
    say(t(key as MessageKey), note.classList.contains("warn"), key);
  }
});
