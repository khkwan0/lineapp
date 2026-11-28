import { invoke } from "@tauri-apps/api/core";
import "./appearance";
import { applyI18n, onLocaleChange, t, type MessageKey } from "./i18n";

type Member = {
  id: string;
  name: string;
  picture: string;
  friend?: boolean;
  mine?: boolean;
};

type MembersCard = {
  id: string;
  name: string;
  picture: string;
  autoJoin: boolean;
  canRemove?: boolean;
  inviteUrl?: string;
  inviteEnabled?: boolean;
  members: Member[];
};

type Friend = { id: string; name: string; picture: string };

const photo = document.querySelector<HTMLElement>("#members-photo")!;
const camera = document.querySelector<HTMLButtonElement>("#members-camera")!;
const photoFile = document.querySelector<HTMLInputElement>("#members-file")!;
const clearPhoto = document.querySelector<HTMLButtonElement>("#members-clear-photo")!;
const nameEl = document.querySelector<HTMLElement>("#members-name")!;
const nameInput = document.querySelector<HTMLInputElement>("#members-name-input")!;
const rename = document.querySelector<HTMLButtonElement>("#members-rename")!;
const query = document.querySelector<HTMLInputElement>("#members-query")!;
const fold = document.querySelector<HTMLButtonElement>("#members-fold")!;
const countEl = document.querySelector<HTMLElement>("#members-count")!;
const list = document.querySelector<HTMLUListElement>("#members-list")!;
const note = document.querySelector<HTMLElement>("#members-note")!;
const foot = document.querySelector<HTMLElement>("#members-foot")!;
const autoJoin = document.querySelector<HTMLButtonElement>("#auto-join")!;
const inviteOpen = document.querySelector<HTMLButtonElement>("#invite-open")!;
const inviteView = document.querySelector<HTMLElement>("#invite-view")!;
const inviteBack = document.querySelector<HTMLButtonElement>("#invite-back")!;
const inviteQuery = document.querySelector<HTMLInputElement>("#invite-query")!;
const inviteList = document.querySelector<HTMLUListElement>("#invite-list")!;
const inviteNote = document.querySelector<HTMLElement>("#invite-note")!;
const inviteSend = document.querySelector<HTMLButtonElement>("#invite-send")!;
const removeConfirm = document.querySelector<HTMLElement>("#remove-confirm")!;
const removeText = document.querySelector<HTMLElement>("#remove-text")!;
const removeError = document.querySelector<HTMLElement>("#remove-error")!;
const removeCancel = document.querySelector<HTMLButtonElement>("#remove-cancel")!;
const removeYes = document.querySelector<HTMLButtonElement>("#remove-yes")!;
const linkBox = document.querySelector<HTMLElement>("#link-box")!;
const linkAllow = document.querySelector<HTMLButtonElement>("#link-allow")!;
const linkUrl = document.querySelector<HTMLElement>("#link-url")!;
const linkHint = document.querySelector<HTMLElement>("#link-hint")!;
const linkCopy = document.querySelector<HTMLButtonElement>("#link-copy")!;
const linkReset = document.querySelector<HTMLButtonElement>("#link-reset")!;
const linkCreate = document.querySelector<HTMLButtonElement>("#link-create")!;
const leaveChat = document.querySelector<HTMLButtonElement>("#leave-chat")!;
const leaveConfirm = document.querySelector<HTMLElement>("#leave-confirm")!;
const leaveText = document.querySelector<HTMLElement>("#leave-text")!;
const leaveError = document.querySelector<HTMLElement>("#leave-error")!;
const leaveCancel = document.querySelector<HTMLButtonElement>("#leave-cancel")!;
const leaveYes = document.querySelector<HTMLButtonElement>("#leave-yes")!;
const linkConfirm = document.querySelector<HTMLElement>("#link-confirm")!;
const linkError = document.querySelector<HTMLElement>("#link-error")!;
const linkCancel = document.querySelector<HTMLButtonElement>("#link-cancel")!;
const linkYes = document.querySelector<HTMLButtonElement>("#link-yes")!;

let card: MembersCard = { id: "", name: "", picture: "", autoJoin: true, canRemove: false, inviteUrl: "", inviteEnabled: true, members: [] };
let pendingRemoval: Member | null = null;
let loaded = false;
let collapsed = false;
let editing = false;
let busy = false;
let loadToken = 0;
let friends: Friend[] = [];
let inviteLoading = false;
let inviteToken = 0;
const picked = new Set<string>();

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function letter(label: string): HTMLElement {
  const circle = document.createElement("span");
  circle.className = "avatar-fallback";
  circle.textContent = (label.trim()[0] || "?").toUpperCase();
  const hue = [...label].reduce((sum, char) => sum + char.charCodeAt(0), 0) % 360;
  circle.style.background = `hsl(${hue} 42% 42%)`;
  return circle;
}

function paintFace(host: HTMLElement, label: string, picture: string, keep: HTMLElement | null): void {
  const face = picture ? document.createElement("img") : letter(label);
  if (face instanceof HTMLImageElement) {
    face.alt = "";
    face.src = picture;
    face.addEventListener("error", () => face.replaceWith(letter(label)));
  }
  host.replaceChildren(face);
  if (keep) {
    host.append(keep);
  }
}

function say(text: string, quiet = false, kind = ""): void {
  note.hidden = !text;
  note.textContent = text;
  note.dataset.kind = kind;
  note.classList.toggle("is-quiet", quiet);
}

function sayKey(key: MessageKey, quiet = true): void {
  say(t(key), quiet, key);
}

function inviteSay(text: string, quiet = false, kind = ""): void {
  inviteNote.hidden = !text;
  inviteNote.textContent = text;
  inviteNote.dataset.kind = kind;
  inviteNote.classList.toggle("is-quiet", quiet);
}

function inviteSayKey(key: MessageKey, quiet = true): void {
  inviteSay(t(key), quiet, key);
}

function visibleMembers(): Member[] {
  const needle = query.value.trim().toLowerCase();
  if (!needle) {
    return card.members;
  }
  return card.members.filter((member) => member.name.toLowerCase().includes(needle));
}

function paintList(): void {
  countEl.textContent = String(card.members.length);
  fold.setAttribute("aria-expanded", collapsed ? "false" : "true");
  fold.classList.toggle("is-closed", collapsed);
  list.hidden = collapsed;
  list.replaceChildren();
  if (collapsed) {
    return;
  }
  if (!loaded) {
    return;
  }
  const shown = visibleMembers();
  if (shown.length === 0 && query.value.trim()) {
    sayKey("noMembersNamed");
    return;
  }
  if (note.dataset.kind === "noMembersNamed") {
    say("");
  }
  shown.forEach((member) => {
    const item = document.createElement("li");
    item.className = "member-line";
    const button = document.createElement("button");
    button.type = "button";
    button.className = "member-row";
    const avatar = document.createElement("span");
    avatar.className = "member-avatar";
    paintFace(avatar, member.name, member.picture, null);
    const name = document.createElement("span");
    name.className = "member-name";
    name.textContent = member.name;
    button.append(avatar, name);
    button.addEventListener("click", () => {
      if (!member.id.startsWith("u")) {
        return;
      }
      void invoke("open_profile", {
        id: member.id,
        name: member.name,
        picture: member.picture,
        mine: Boolean(member.mine),
        friend: Boolean(member.friend),
      }).catch((error: unknown) => say(message(error)));
    });
    item.append(button);
    if (card.canRemove && !member.mine && member.id.startsWith("u")) {
      const remove = document.createElement("button");
      remove.type = "button";
      remove.className = "member-remove";
      remove.setAttribute("aria-label", t("removeNamed", { name: member.name }));
      const icon = document.createElementNS("http://www.w3.org/2000/svg", "svg");
      icon.setAttribute("viewBox", "0 0 16 16");
      icon.setAttribute("aria-hidden", "true");
      const stroke = document.createElementNS("http://www.w3.org/2000/svg", "path");
      stroke.setAttribute("d", "M4 4l8 8M12 4l-8 8");
      stroke.setAttribute("fill", "none");
      stroke.setAttribute("stroke", "currentColor");
      stroke.setAttribute("stroke-width", "1.6");
      stroke.setAttribute("stroke-linecap", "round");
      icon.append(stroke);
      remove.append(icon);
      remove.addEventListener("click", (event) => {
        event.stopPropagation();
        askRemove(member);
      });
      item.append(remove);
    }
    list.append(item);
  });
}

function askRemove(member: Member): void {
  pendingRemoval = member;
  removeText.textContent = t("removeFromGroup", { name: member.name });
  removeError.hidden = true;
  removeError.textContent = "";
  removeYes.disabled = false;
  removeConfirm.hidden = false;
  removeCancel.focus();
}

function closeRemove(): void {
  pendingRemoval = null;
  removeConfirm.hidden = true;
}

function isGroup(): boolean {
  return card.id.startsWith("c");
}

function adopt(next: MembersCard, id: string): void {
  card = {
    id: next.id || id,
    name: next.name || card.name,
    picture: next.picture ?? card.picture,
    autoJoin: Boolean(next.autoJoin),
    canRemove: Boolean(next.canRemove),
    inviteUrl: next.inviteUrl || "",
    inviteEnabled: next.inviteEnabled !== false,
    members: Array.isArray(next.members) ? next.members : card.members,
  };
  loaded = true;
}

function paint(): void {
  const group = isGroup();
  paintFace(photo, card.name || t(group ? "group" : "room"), card.picture, camera);
  camera.hidden = !group;
  clearPhoto.hidden = !group || !card.picture;
  rename.hidden = editing || !group;
  inviteOpen.hidden = !group;
  document.querySelector<HTMLElement>("#auto-row")!.hidden = !group;
  document.querySelector<HTMLElement>("#auto-copy")!.hidden = !group;
  linkBox.hidden = !group;
  if (!editing) {
    nameEl.hidden = false;
    nameInput.hidden = true;
    nameEl.textContent = card.name || t(group ? "group" : "room");
  }
  foot.hidden = !loaded;
  autoJoin.setAttribute("aria-checked", card.autoJoin ? "true" : "false");
  const linkOn = card.inviteEnabled !== false;
  linkAllow.setAttribute("aria-checked", linkOn ? "true" : "false");
  const url = linkOn ? card.inviteUrl || "" : "";
  linkUrl.hidden = !url;
  linkUrl.textContent = url;
  linkHint.textContent = !linkOn ? t("inviteLinkOff") : url ? t("inviteLinkHint") : t("noInviteLink");
  linkCopy.hidden = !url;
  linkReset.hidden = !card.inviteUrl;
  linkCreate.hidden = Boolean(card.inviteUrl);
  leaveChat.textContent = t(group ? "leaveGroup" : "leaveRoom");
  if (!loaded) {
    list.hidden = false;
    sayKey("loadingMembers");
  } else if (note.dataset.kind === "loadingMembers") {
    say("");
  }
  paintList();
}

function stopEditing(): void {
  editing = false;
  nameInput.hidden = true;
  nameEl.hidden = false;
  rename.hidden = false;
}

function applyMembers(next: MembersCard): void {
  card = {
    id: next.id,
    name: next.name || "",
    picture: next.picture || "",
    autoJoin: Boolean(next.autoJoin),
    canRemove: Boolean(next.canRemove),
    inviteUrl: next.inviteUrl || "",
    inviteEnabled: next.inviteEnabled !== false,
    members: Array.isArray(next.members) ? next.members : [],
  };
  loaded = false;
  collapsed = false;
  picked.clear();
  friends = [];
  inviteLoading = false;
  inviteToken += 1;
  query.value = "";
  inviteQuery.value = "";
  inviteView.hidden = true;
  closeRemove();
  stopEditing();
  say("");
  inviteSay("");
  paint();
  const token = ++loadToken;
  const id = card.id;
  void invoke<MembersCard>("group_roster", { id })
    .then((roster) => {
      if (token !== loadToken) {
        return;
      }
      adopt(roster, id);
      say("");
      paint();
    })
    .catch((error: unknown) => {
      if (token !== loadToken) {
        return;
      }
      say(message(error));
    });
}

(window as unknown as { __applyMembers?: (next: MembersCard) => void }).__applyMembers = applyMembers;

const queued = (window as unknown as { __members?: MembersCard }).__members;
if (queued?.id) {
  applyMembers(queued);
}

rename.addEventListener("click", () => {
  if (busy || !card.id) {
    return;
  }
  editing = true;
  nameEl.hidden = true;
  rename.hidden = true;
  nameInput.hidden = false;
  nameInput.value = card.name;
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
  if (!next || next === card.name) {
    stopEditing();
    paint();
    return;
  }
  if (next.length > 50) {
    sayKey("nameTooLong", false);
    stopEditing();
    paint();
    return;
  }
  busy = true;
  const previous = card.name;
  card.name = next;
  stopEditing();
  paint();
  void invoke("rename_group", { id: card.id, name: next })
    .then(() => say(""))
    .catch((error: unknown) => {
      card.name = previous;
      paint();
      say(message(error));
    })
    .finally(() => {
      busy = false;
    });
});

query.addEventListener("input", () => {
  if (query.value.trim()) {
    collapsed = false;
  }
  paintList();
});

fold.addEventListener("click", () => {
  collapsed = !collapsed;
  paintList();
});

autoJoin.addEventListener("click", () => {
  if (busy || !loaded || !card.id) {
    return;
  }
  const next = !card.autoJoin;
  card.autoJoin = next;
  autoJoin.setAttribute("aria-checked", next ? "true" : "false");
  busy = true;
  void invoke("set_group_auto_join", { id: card.id, enabled: next })
    .then(() => say(""))
    .catch((error: unknown) => {
      card.autoJoin = !next;
      autoJoin.setAttribute("aria-checked", card.autoJoin ? "true" : "false");
      say(message(error));
    })
    .finally(() => {
      busy = false;
    });
});

function paintInvite(): void {
  const needle = inviteQuery.value.trim().toLowerCase();
  const shown = friends.filter((friend) => !needle || friend.name.toLowerCase().includes(needle));
  inviteList.replaceChildren();
  inviteSend.disabled = picked.size === 0 || busy || inviteLoading;
  inviteSend.textContent = picked.size > 0 ? t("inviteCount", { count: picked.size }) : t("invite");
  if (inviteLoading) {
    return;
  }
  if (friends.length === 0) {
    if (inviteNote.hidden || inviteNote.classList.contains("is-quiet")) {
      inviteSayKey("noFriendsToInvite");
    }
    return;
  }
  if (shown.length === 0) {
    inviteSayKey("noFriendsNamed");
    return;
  }
  if (inviteNote.classList.contains("is-quiet")) {
    inviteSay("");
  }
  shown.forEach((friend) => {
    const item = document.createElement("li");
    const button = document.createElement("button");
    button.type = "button";
    button.className = picked.has(friend.id) ? "invite-row is-on" : "invite-row";
    button.setAttribute("aria-pressed", picked.has(friend.id) ? "true" : "false");
    const avatar = document.createElement("span");
    avatar.className = "invite-avatar";
    paintFace(avatar, friend.name, friend.picture, null);
    const name = document.createElement("span");
    name.className = "invite-name";
    name.textContent = friend.name;
    const mark = document.createElement("span");
    mark.className = "invite-check";
    button.append(avatar, name, mark);
    button.addEventListener("click", () => {
      if (picked.has(friend.id)) {
        picked.delete(friend.id);
      } else {
        picked.add(friend.id);
      }
      paintInvite();
    });
    item.append(button);
    inviteList.append(item);
  });
}

inviteOpen.addEventListener("click", () => {
  if (!card.id) {
    return;
  }
  inviteView.hidden = false;
  picked.clear();
  friends = [];
  inviteLoading = true;
  const token = ++inviteToken;
  const id = card.id;
  inviteQuery.value = "";
  inviteSayKey("loadingFriends");
  paintInvite();
  void invoke<Friend[]>("group_invite_candidates", { id })
    .then((rows) => {
      if (token !== inviteToken) {
        return;
      }
      friends = Array.isArray(rows) ? rows : [];
      inviteSay("");
    })
    .catch((error: unknown) => {
      if (token !== inviteToken) {
        return;
      }
      friends = [];
      inviteSay(message(error));
    })
    .finally(() => {
      if (token !== inviteToken) {
        return;
      }
      inviteLoading = false;
      paintInvite();
    });
});

inviteBack.addEventListener("click", () => {
  inviteView.hidden = true;
  inviteSay("");
});

inviteQuery.addEventListener("input", () => paintInvite());

inviteSend.addEventListener("click", () => {
  if (busy || picked.size === 0 || !card.id) {
    return;
  }
  const targets = [...picked];
  busy = true;
  inviteSend.disabled = true;
  void invoke("invite_into_group", { id: card.id, targets })
    .then(() => {
      picked.clear();
      inviteView.hidden = true;
      inviteSay("");
      sayKey(card.autoJoin ? "invited" : "invitedNeedAccept");
      const token = ++loadToken;
      const id = card.id;
      return invoke<MembersCard>("group_roster", { id }).then((roster) => {
        if (token !== loadToken) {
          return;
        }
        adopt(roster, id);
        paint();
      });
    })
    .catch((error: unknown) => inviteSay(message(error)))
    .finally(() => {
      busy = false;
      paintInvite();
    });
});

removeCancel.addEventListener("click", () => {
  if (!busy) {
    closeRemove();
  }
});

removeConfirm.addEventListener("click", (event) => {
  if (event.target === removeConfirm && !busy) {
    closeRemove();
  }
});

document.addEventListener("keydown", (event) => {
  if (event.key !== "Escape" || busy) {
    return;
  }
  if (!removeConfirm.hidden) {
    closeRemove();
  } else if (!leaveConfirm.hidden) {
    leaveConfirm.hidden = true;
  } else if (!linkConfirm.hidden) {
    linkConfirm.hidden = true;
  }
});

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

camera.addEventListener("click", () => {
  if (!busy && isGroup()) {
    photoFile.click();
  }
});

photoFile.addEventListener("change", () => {
  const file = photoFile.files?.[0];
  photoFile.value = "";
  if (!file || busy || !isGroup()) {
    return;
  }
  busy = true;
  void fileAsBase64(file)
    .then((data) => invoke<MembersCard>("set_group_picture", { id: card.id, data }))
    .then((roster) => {
      adopt(roster, card.id);
      say("");
      paint();
    })
    .catch((error: unknown) => say(message(error)))
    .finally(() => {
      busy = false;
    });
});

clearPhoto.addEventListener("click", () => {
  if (busy || !isGroup() || !card.picture) {
    return;
  }
  busy = true;
  void invoke<MembersCard>("clear_group_picture", { id: card.id })
    .then((roster) => {
      adopt({ ...roster, picture: roster.picture || "" }, card.id);
      say("");
      paint();
    })
    .catch((error: unknown) => say(message(error)))
    .finally(() => {
      busy = false;
    });
});

linkAllow.addEventListener("click", () => {
  if (busy || !loaded || !isGroup()) {
    return;
  }
  const next = card.inviteEnabled === false;
  busy = true;
  const command = next && !card.inviteUrl ? "reissue_group_ticket" : "set_group_invite_link";
  const args = command === "reissue_group_ticket" ? { id: card.id } : { id: card.id, enabled: next };
  void invoke<MembersCard>(command, args)
    .then((roster) => {
      adopt(roster, card.id);
      if (command === "set_group_invite_link") {
        card.inviteEnabled = next;
      }
      say("");
      paint();
    })
    .catch((error: unknown) => say(message(error)))
    .finally(() => {
      busy = false;
    });
});

linkCopy.addEventListener("click", () => {
  const url = card.inviteUrl || "";
  if (!url || card.inviteEnabled === false) {
    return;
  }
  const done = () => sayKey("linkCopied");
  if (navigator.clipboard?.writeText) {
    void navigator.clipboard.writeText(url).then(done).catch(() => say(url, true));
    return;
  }
  say(url, true);
});

linkCreate.addEventListener("click", () => {
  if (busy || !isGroup()) {
    return;
  }
  busy = true;
  void invoke<MembersCard>("reissue_group_ticket", { id: card.id })
    .then((roster) => {
      adopt(roster, card.id);
      say("");
      paint();
    })
    .catch((error: unknown) => say(message(error)))
    .finally(() => {
      busy = false;
    });
});

linkReset.addEventListener("click", () => {
  if (busy || !card.inviteUrl) {
    return;
  }
  linkError.hidden = true;
  linkError.textContent = "";
  linkYes.disabled = false;
  linkConfirm.hidden = false;
  linkCancel.focus();
});

linkCancel.addEventListener("click", () => {
  if (!busy) {
    linkConfirm.hidden = true;
  }
});

linkConfirm.addEventListener("click", (event) => {
  if (event.target === linkConfirm && !busy) {
    linkConfirm.hidden = true;
  }
});

linkYes.addEventListener("click", () => {
  if (busy || !isGroup()) {
    return;
  }
  busy = true;
  linkYes.disabled = true;
  linkError.hidden = true;
  void invoke<MembersCard>("reissue_group_ticket", { id: card.id })
    .then((roster) => {
      adopt(roster, card.id);
      linkConfirm.hidden = true;
      say("");
      paint();
    })
    .catch((error: unknown) => {
      linkError.hidden = false;
      linkError.textContent = message(error);
    })
    .finally(() => {
      busy = false;
      linkYes.disabled = false;
    });
});

leaveChat.addEventListener("click", () => {
  if (busy || !card.id) {
    return;
  }
  const group = isGroup();
  leaveText.textContent = t(group ? "leaveGroupAsk" : "leaveRoomAsk");
  leaveYes.textContent = t(group ? "leaveGroup" : "leaveRoom");
  leaveError.hidden = true;
  leaveError.textContent = "";
  leaveYes.disabled = false;
  leaveConfirm.hidden = false;
  leaveCancel.focus();
});

leaveCancel.addEventListener("click", () => {
  if (!busy) {
    leaveConfirm.hidden = true;
  }
});

leaveConfirm.addEventListener("click", (event) => {
  if (event.target === leaveConfirm && !busy) {
    leaveConfirm.hidden = true;
  }
});

leaveYes.addEventListener("click", () => {
  if (busy || !card.id) {
    return;
  }
  busy = true;
  leaveYes.disabled = true;
  leaveError.hidden = true;
  void invoke("leave_chat", { id: card.id }).catch((error: unknown) => {
    leaveError.hidden = false;
    leaveError.textContent = message(error);
    busy = false;
    leaveYes.disabled = false;
  });
});

removeYes.addEventListener("click", () => {
  const member = pendingRemoval;
  if (busy || !member || !card.id || !card.canRemove || member.mine) {
    return;
  }
  busy = true;
  removeYes.disabled = true;
  removeError.hidden = true;
  void invoke("remove_group_member", { id: card.id, member: member.id })
    .then(() => {
      card.members = card.members.filter((item) => item.id !== member.id);
      closeRemove();
      paint();
      const token = ++loadToken;
      const id = card.id;
      void invoke<MembersCard>("group_roster", { id })
        .then((roster) => {
          if (token !== loadToken) {
            return;
          }
          adopt(roster, id);
          paint();
        })
        .catch(() => undefined);
    })
    .catch((error: unknown) => {
      removeError.hidden = false;
      removeError.textContent = message(error);
    })
    .finally(() => {
      busy = false;
      removeYes.disabled = false;
    });
});

function refreshLocale(): void {
  applyI18n();
  document.title = t("members");
  paint();
  if (!inviteView.hidden) {
    paintInvite();
  }
  if (pendingRemoval) {
    askRemove(pendingRemoval);
  }
  if (!leaveConfirm.hidden) {
    leaveText.textContent = t(isGroup() ? "leaveGroupAsk" : "leaveRoomAsk");
    leaveYes.textContent = t(isGroup() ? "leaveGroup" : "leaveRoom");
  }
  const kind = note.dataset.kind || "";
  if (kind && !note.hidden && kind !== "noMembersNamed" && kind !== "loadingMembers") {
    say(t(kind as MessageKey), note.classList.contains("is-quiet"), kind);
  }
}

applyI18n();
document.title = t("members");
onLocaleChange(refreshLocale);
