//! LINE actions the agent can call.
//!
//! These follow the personal account this client already speaks. The public
//! Messaging API is for an Official Account channel token, which this app does
//! not have, so rich menus, broadcasts, quotas, insights, audiences, and
//! webhooks are not offered.

use serde_json::{json, Value as Json};

use crate::line::{self, Client};
use crate::store;

pub(crate) struct Offered {
    pub name: String,
    pub description: String,
    pub schema: Json,
}

pub(crate) fn offered() -> Vec<Offered> {
    vec![
        tool(
            "search_users",
            "Search for a person. A LINE ID or phone number is looked up on LINE. A name, including one that is not an ID, is matched against friends saved on this computer.",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "A display name, LINE ID, or phone number." },
                    "limit": { "type": "integer", "description": "Maximum saved friends to return, from 1 to 20. Defaults to 12." }
                },
                "required": ["query"]
            }),
        ),
        tool(
            "list_friends",
            "List friends on the signed-in account. Optional name filter. Like the friend list, not the bot follower API.",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Optional part of a friend's name." },
                    "limit": { "type": "integer", "description": "Maximum friends, from 1 to 40. Defaults to 20." }
                }
            }),
        ),
        tool(
            "friend_recommendations",
            "List people LINE recommends adding.",
            json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Maximum people, from 1 to 20. Defaults to 12." }
                }
            }),
        ),
        tool(
            "get_profile",
            "Load one person's profile by their id. Corresponds to Get profile on a personal account.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The person's id, starting with u." }
                },
                "required": ["id"]
            }),
        ),
        tool(
            "get_my_profile",
            "Load the signed-in account's name and status message.",
            json!({ "type": "object", "properties": {} }),
        ),
        tool(
            "list_group_members",
            "List members of a group, with the group name and whether automatic join is on. Corresponds to the group member profile and member count calls.",
            json!({
                "type": "object",
                "properties": {
                    "group": { "type": "string", "description": "Group id, starting with c." }
                },
                "required": ["group"]
            }),
        ),
        tool(
            "list_invite_candidates",
            "List friends who can be invited into a group.",
            json!({
                "type": "object",
                "properties": {
                    "group": { "type": "string", "description": "Group id, starting with c." },
                    "name": { "type": "string", "description": "Optional part of a friend's name." },
                    "limit": { "type": "integer", "description": "Maximum friends, from 1 to 40. Defaults to 20." }
                },
                "required": ["group"]
            }),
        ),
        tool(
            "fetch_chat",
            "Load the newest messages LINE has for one chat. Use this when the saved copy may be behind.",
            json!({
                "type": "object",
                "properties": {
                    "chat": { "type": "string", "description": "Chat id of a person, group, or room." },
                    "count": { "type": "integer", "description": "How many of the newest messages to return, from 1 to 40. Defaults to 20." }
                },
                "required": ["chat"]
            }),
        ),
        tool(
            "send_text",
            "Send a text message to a person, group, or room. Corresponds to a push message. Call this only when the user asked to send that message.",
            json!({
                "type": "object",
                "properties": {
                    "chat": { "type": "string", "description": "Chat id." },
                    "text": { "type": "string", "description": "The message to send." },
                    "replyTo": { "type": "string", "description": "Optional message id to quote." }
                },
                "required": ["chat", "text"]
            }),
        ),
        tool(
            "send_sticker",
            "Send a sticker the signed-in account owns. packageId and stickerId must be ids from that account. Call this only when the user asked to send a sticker.",
            json!({
                "type": "object",
                "properties": {
                    "chat": { "type": "string", "description": "Chat id." },
                    "packageId": { "type": "string", "description": "Sticker package id." },
                    "stickerId": { "type": "string", "description": "Sticker id." },
                    "animated": { "type": "boolean", "description": "True for the animated package." }
                },
                "required": ["chat", "packageId", "stickerId"]
            }),
        ),
        tool(
            "send_contact",
            "Send a contact card for a person. mid is that person's id, starting with u. Call this only when the user asked to send that contact.",
            json!({
                "type": "object",
                "properties": {
                    "chat": { "type": "string", "description": "Chat id to send the card into." },
                    "mid": { "type": "string", "description": "User id of the person on the card, starting with u." }
                },
                "required": ["chat", "mid"]
            }),
        ),
        tool(
            "unsend_message",
            "Unsend one of your own messages. Call this only when the user asked to unsend it.",
            json!({
                "type": "object",
                "properties": {
                    "messageId": { "type": "string", "description": "Message id." }
                },
                "required": ["messageId"]
            }),
        ),
        tool(
            "react_to_message",
            "React to a message. Emoji is one of 👍, 😎, 😍, 😠, 🥺, 😭, 😮. Call this only when the user asked for that reaction.",
            json!({
                "type": "object",
                "properties": {
                    "messageId": { "type": "string", "description": "Message id." },
                    "emoji": { "type": "string", "description": "One of 👍, 😎, 😍, 😠, 🥺, 😭, 😮." }
                },
                "required": ["messageId", "emoji"]
            }),
        ),
        tool(
            "mark_chat_read",
            "Mark a chat read up through a message. Corresponds to Mark messages as read. Call this only when the user asked.",
            json!({
                "type": "object",
                "properties": {
                    "chat": { "type": "string", "description": "Chat id." },
                    "messageId": { "type": "string", "description": "Last message id to mark read." }
                },
                "required": ["chat", "messageId"]
            }),
        ),
        tool(
            "add_friend",
            "Add a person as a friend by their id. Call this only when the user asked to add them.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The person's id, starting with u." }
                },
                "required": ["id"]
            }),
        ),
        tool(
            "set_blocked",
            "Block or unblock a person. Call this only when the user asked.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The person's id, starting with u." },
                    "blocked": { "type": "boolean", "description": "True to block, false to unblock." }
                },
                "required": ["id", "blocked"]
            }),
        ),
        tool(
            "set_favorite",
            "Add or remove a friend from favorites. Call this only when the user asked.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The person's id, starting with u." },
                    "favorite": { "type": "boolean", "description": "True to favorite, false to remove." }
                },
                "required": ["id", "favorite"]
            }),
        ),
        tool(
            "rename_friend",
            "Change the display name shown for a friend on this account. Call this only when the user asked.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The person's id, starting with u." },
                    "name": { "type": "string", "description": "The new display name, up to 20 characters." }
                },
                "required": ["id", "name"]
            }),
        ),
        tool(
            "create_room",
            "Create a room with friend ids. A room has no name and no invite link. Call this only when the user asked.",
            json!({
                "type": "object",
                "properties": {
                    "members": { "type": "array", "items": { "type": "string" }, "description": "Friend ids, each starting with u." }
                },
                "required": ["members"]
            }),
        ),
        tool(
            "create_group",
            "Create a group with a name and friend ids. Call this only when the user asked.",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Group name." },
                    "members": { "type": "array", "items": { "type": "string" }, "description": "Friend ids, each starting with u." }
                },
                "required": ["name", "members"]
            }),
        ),
        tool(
            "rename_group",
            "Rename a group. Call this only when the user asked.",
            json!({
                "type": "object",
                "properties": {
                    "group": { "type": "string", "description": "Group id, starting with c." },
                    "name": { "type": "string", "description": "New group name." }
                },
                "required": ["group", "name"]
            }),
        ),
        tool(
            "invite_to_group",
            "Invite friends into a group. Call this only when the user asked.",
            json!({
                "type": "object",
                "properties": {
                    "group": { "type": "string", "description": "Group id, starting with c." },
                    "members": { "type": "array", "items": { "type": "string" }, "description": "Friend ids, each starting with u." }
                },
                "required": ["group", "members"]
            }),
        ),
        tool(
            "remove_group_member",
            "Remove a member from a group. Call this only when the user asked.",
            json!({
                "type": "object",
                "properties": {
                    "group": { "type": "string", "description": "Group id, starting with c." },
                    "id": { "type": "string", "description": "Member id, starting with u." }
                },
                "required": ["group", "id"]
            }),
        ),
        tool(
            "leave_chat",
            "Leave a group or a room. Call this only when the user asked.",
            json!({
                "type": "object",
                "properties": {
                    "chat": { "type": "string", "description": "Group id starting with c, or room id starting with r." }
                },
                "required": ["chat"]
            }),
        ),
        tool(
            "reissue_invite_link",
            "Create or reset a group's invite link. The previous link stops working. Call this only when the user asked.",
            json!({
                "type": "object",
                "properties": {
                    "group": { "type": "string", "description": "Group id, starting with c." }
                },
                "required": ["group"]
            }),
        ),
        tool(
            "set_invite_link",
            "Turn a group's invite link on or off. Call this only when the user asked.",
            json!({
                "type": "object",
                "properties": {
                    "group": { "type": "string", "description": "Group id, starting with c." },
                    "enabled": { "type": "boolean", "description": "True to allow joining with the link." }
                },
                "required": ["group", "enabled"]
            }),
        ),
        tool(
            "set_group_auto_join",
            "Turn a group's automatic join on or off. Call this only when the user asked.",
            json!({
                "type": "object",
                "properties": {
                    "group": { "type": "string", "description": "Group id, starting with c." },
                    "enabled": { "type": "boolean", "description": "True to allow automatic join." }
                },
                "required": ["group", "enabled"]
            }),
        ),
        tool(
            "report_message",
            "Report someone else's message. Reason is other, advertising, gender, or harassment. Call this only when the user asked.",
            json!({
                "type": "object",
                "properties": {
                    "chat": { "type": "string", "description": "Chat id." },
                    "sender": { "type": "string", "description": "Sender id, starting with u." },
                    "messageId": { "type": "string", "description": "Message id." },
                    "reason": { "type": "string", "description": "other, advertising, gender, or harassment." },
                    "text": { "type": "string", "description": "Optional note, up to 1000 characters." }
                },
                "required": ["chat", "sender", "messageId", "reason"]
            }),
        ),
    ]
}

pub(crate) fn handles(name: &str) -> bool {
    offered().iter().any(|tool| tool.name == name)
}

pub(crate) fn invoke(name: &str, arguments: &Json) -> Json {
    let args = object_args(arguments);
    if !args.is_object() {
        return json!({ "error": "Could not read the tool arguments." });
    }
    match name {
        "search_users" => search_users(&args),
        "list_friends" => list_friends(&args),
        "friend_recommendations" => friend_recommendations(&args),
        "get_profile" => get_profile(&args),
        "get_my_profile" => get_my_profile(),
        "list_group_members" => list_group_members(&args),
        "list_invite_candidates" => list_invite_candidates(&args),
        "fetch_chat" => fetch_chat(&args),
        "send_text" => send_text(&args),
        "send_sticker" => send_sticker(&args),
        "send_contact" => send_contact(&args),
        "unsend_message" => unsend_message(&args),
        "react_to_message" => react_to_message(&args),
        "mark_chat_read" => mark_chat_read(&args),
        "add_friend" => add_friend(&args),
        "set_blocked" => set_blocked(&args),
        "set_favorite" => set_favorite(&args),
        "rename_friend" => rename_friend(&args),
        "create_group" => create_group(&args),
        "create_room" => create_room(&args),
        "leave_chat" => leave_chat(&args),
        "reissue_invite_link" => reissue_invite_link(&args),
        "set_invite_link" => set_invite_link(&args),
        "rename_group" => rename_group(&args),
        "invite_to_group" => invite_to_group(&args),
        "remove_group_member" => remove_group_member(&args),
        "set_group_auto_join" => set_group_auto_join(&args),
        "report_message" => report_message(&args),
        _ => json!({ "error": "Unknown tool." }),
    }
}

fn tool(name: &str, description: &str, schema: Json) -> Offered {
    Offered { name: name.to_string(), description: description.to_string(), schema }
}

fn object_args(arguments: &Json) -> Json {
    match arguments {
        Json::String(text) if text.trim().is_empty() => json!({}),
        Json::String(text) => serde_json::from_str::<Json>(text).ok().filter(Json::is_object).unwrap_or(Json::Null),
        Json::Object(_) => arguments.clone(),
        Json::Null => json!({}),
        _ => Json::Null,
    }
}

fn search_users(args: &Json) -> Json {
    let query = text(args, "query");
    if query.is_empty() || query.chars().count() > 40 {
        return json!({ "error": "Enter a name, LINE ID, or phone number." });
    }
    let limit = limit_of(args, "limit", 12, 20);
    let saved = saved_people(&query, limit);
    if !directory_query(&query) {
        return json!({ "saved": saved, "directory": [] });
    }
    if !line::signed_in() {
        return json!({
            "saved": saved,
            "directory": [],
            "note": "Sign in to LINE to search by LINE ID or phone number."
        });
    }
    match Client::new().find_person(&query) {
        Ok(card) => json!({ "saved": saved, "directory": [person(&card)] }),
        Err(err) if err.to_string() == "No one was found." => json!({ "saved": saved, "directory": [] }),
        Err(err) => json!({ "saved": saved, "directory": [], "note": err.to_string() }),
    }
}

fn saved_people(query: &str, limit: usize) -> Vec<Json> {
    let needle = query.to_lowercase();
    let mut people: Vec<Json> = store::cached_rows()
        .into_iter()
        .filter(|(id, chat)| user_ok(id) && chat.kind == "friend" && chat.name.to_lowercase().contains(&needle))
        .map(|(id, chat)| json!({ "id": id, "name": chat.name }))
        .collect();
    people.sort_by(|left, right| {
        left.get("name").and_then(Json::as_str).unwrap_or("").to_lowercase().cmp(&right.get("name").and_then(Json::as_str).unwrap_or("").to_lowercase())
    });
    people.truncate(limit);
    people
}

fn directory_query(query: &str) -> bool {
    let letters = query.chars().any(|ch| ch.is_ascii_alphabetic() || matches!(ch, '_' | '.' | '@' | '-'));
    let digits = query.chars().filter(|ch| ch.is_ascii_digit()).count();
    let phone = !letters && (query.starts_with('+') || digits >= 8);
    phone || query.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '@' | '-'))
}

fn list_friends(args: &Json) -> Json {
    let Some(mut client) = account() else {
        return signed_out();
    };
    match client.list_friends() {
        Ok(rows) => {
            let name = text(args, "name").to_lowercase();
            let limit = limit_of(args, "limit", 20, 40);
            let mut friends = rows
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|row| name.is_empty() || row.get("name").and_then(Json::as_str).unwrap_or("").to_lowercase().contains(&name))
                .map(|row| json!({
                    "id": row.get("id").cloned().unwrap_or(Json::Null),
                    "name": row.get("name").cloned().unwrap_or(Json::String(String::new())),
                    "favorite": row.get("favorite").and_then(Json::as_bool).unwrap_or(false),
                }))
                .collect::<Vec<_>>();
            let total = friends.len();
            friends.truncate(limit);
            json!({ "friends": friends, "count": total })
        }
        Err(err) => failed(err),
    }
}

fn friend_recommendations(args: &Json) -> Json {
    let Some(mut client) = account() else {
        return signed_out();
    };
    match client.friend_recommendations() {
        Ok(rows) => {
            let limit = limit_of(args, "limit", 12, 20);
            let mut people = rows
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|row| json!({
                    "id": row.get("id").cloned().unwrap_or(Json::Null),
                    "name": row.get("name").cloned().unwrap_or(Json::String(String::new())),
                    "reason": row.get("reason").cloned().unwrap_or(Json::String(String::new())),
                }))
                .collect::<Vec<_>>();
            people.truncate(limit);
            json!({ "people": people })
        }
        Err(err) => failed(err),
    }
}

fn get_profile(args: &Json) -> Json {
    let id = text(args, "id");
    if !user_ok(&id) {
        return json!({ "error": "That profile cannot be opened." });
    }
    let Some(mut client) = account() else {
        return signed_out();
    };
    match client.contact_profile(&id) {
        Ok(card) => person(&card),
        Err(err) => failed(err),
    }
}

fn get_my_profile() -> Json {
    let Some(mut client) = account() else {
        return signed_out();
    };
    match client.my_profile() {
        Ok(card) => person(&card),
        Err(err) => failed(err),
    }
}

fn list_group_members(args: &Json) -> Json {
    let group = text(args, "group");
    if !group_ok(&group) {
        return json!({ "error": "That group cannot be opened." });
    }
    let Some(mut client) = account() else {
        return signed_out();
    };
    match client.group_roster(&group) {
        Ok(roster) => {
            let mut members = roster.get("members").and_then(Json::as_array).cloned().unwrap_or_default();
            let total = members.len();
            let truncated = total > 200;
            members.truncate(200);
            let members: Vec<Json> = members
                .into_iter()
                .map(|row| json!({
                    "id": row.get("id").cloned().unwrap_or(Json::Null),
                    "name": row.get("name").cloned().unwrap_or(Json::String(String::new())),
                    "friend": row.get("friend").and_then(Json::as_bool).unwrap_or(false),
                    "mine": row.get("mine").and_then(Json::as_bool).unwrap_or(false),
                }))
                .collect();
            json!({
                "id": roster.get("id").cloned().unwrap_or(Json::Null),
                "name": roster.get("name").cloned().unwrap_or(Json::String(String::new())),
                "autoJoin": roster.get("autoJoin").and_then(Json::as_bool).unwrap_or(false),
                "canRemove": roster.get("canRemove").and_then(Json::as_bool).unwrap_or(false),
                "members": members,
                "count": total,
                "truncated": truncated,
            })
        }
        Err(err) => failed(err),
    }
}

fn list_invite_candidates(args: &Json) -> Json {
    let group = text(args, "group");
    if !group_ok(&group) {
        return json!({ "error": "That group cannot be opened." });
    }
    let Some(mut client) = account() else {
        return signed_out();
    };
    match client.group_invite_candidates(&group) {
        Ok(rows) => {
            let name = text(args, "name").to_lowercase();
            let limit = limit_of(args, "limit", 20, 40);
            let mut people = rows
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|row| name.is_empty() || row.get("name").and_then(Json::as_str).unwrap_or("").to_lowercase().contains(&name))
                .map(|row| json!({
                    "id": row.get("id").cloned().unwrap_or(Json::Null),
                    "name": row.get("name").cloned().unwrap_or(Json::String(String::new())),
                }))
                .collect::<Vec<_>>();
            let total = people.len();
            people.truncate(limit);
            json!({ "people": people, "count": total })
        }
        Err(err) => failed(err),
    }
}

fn fetch_chat(args: &Json) -> Json {
    let chat = text(args, "chat");
    if !chat_ok(&chat) {
        return json!({ "error": "That chat cannot be opened." });
    }
    let count = limit_of(args, "count", 20, 40);
    let Some(mut client) = account() else {
        return signed_out();
    };
    match client.chat_history(&chat) {
        Ok(history) => {
            let mut messages = history.get("messages").and_then(Json::as_array).cloned().unwrap_or_default();
            if messages.len() > count {
                messages = messages.split_off(messages.len() - count);
            }
            let messages: Vec<Json> = messages
                .into_iter()
                .map(|row| json!({
                    "id": row.get("id").cloned().unwrap_or(Json::String(String::new())),
                    "mine": row.get("mine").and_then(Json::as_bool).unwrap_or(false),
                    "name": row.get("name").cloned().unwrap_or(Json::String(String::new())),
                    "text": row.get("text").cloned().unwrap_or(Json::String(String::new())),
                    "time": row.get("time").cloned().unwrap_or(Json::from(0)),
                    "kind": row.get("kind").cloned().unwrap_or(Json::String(String::new())),
                    "sealed": row.get("sealed").and_then(Json::as_bool).unwrap_or(false),
                }))
                .collect();
            json!({
                "id": chat,
                "lastMessageId": history.get("lastMessageId").cloned().unwrap_or(Json::String(String::new())),
                "messages": messages,
            })
        }
        Err(err) => failed(err),
    }
}

fn send_text(args: &Json) -> Json {
    let chat = text(args, "chat");
    let body = text(args, "text");
    let reply = text(args, "replyTo");
    if !chat_ok(&chat) {
        return json!({ "error": "That chat cannot be opened." });
    }
    if body.is_empty() || body.chars().count() > 5000 {
        return json!({ "error": "Enter a message first." });
    }
    if !reply.is_empty() && !message_ok(&reply) {
        return json!({ "error": "That message cannot be replied to." });
    }
    let Some(mut client) = account() else {
        return signed_out();
    };
    let sent = {
        let _send = line::TALK_SEND.lock().unwrap_or_else(|err| err.into_inner());
        client.send_for_window(&chat, &body, &reply, false)
    };
    if sent.get("ok") == Some(&Json::Bool(true)) {
        let _ = store::save_outgoing(&chat, &sent);
    }
    sent
}

fn send_sticker(args: &Json) -> Json {
    let chat = text(args, "chat");
    let package_id = text(args, "packageId");
    let sticker_id = text(args, "stickerId");
    if !chat_ok(&chat) {
        return json!({ "error": "That chat cannot be opened." });
    }
    if !sticker_ok(&package_id) || !sticker_ok(&sticker_id) {
        return json!({ "error": "That sticker cannot be sent." });
    }
    let animated = args.get("animated").and_then(Json::as_bool).unwrap_or(false);
    let Some(mut client) = account() else {
        return signed_out();
    };
    let sent = {
        let _send = line::TALK_SEND.lock().unwrap_or_else(|err| err.into_inner());
        client.send_sticker_for_window(&chat, &package_id, &sticker_id, animated)
    };
    if sent.get("ok") == Some(&Json::Bool(true)) {
        let _ = store::save_outgoing(&chat, &sent);
    }
    sent
}

fn send_contact(args: &Json) -> Json {
    let chat = text(args, "chat");
    let mid = text(args, "mid");
    if !chat_ok(&chat) {
        return json!({ "error": "That chat cannot be opened." });
    }
    if !user_ok(&mid) {
        return json!({ "error": "That contact cannot be sent." });
    }
    let Some(mut client) = account() else {
        return signed_out();
    };
    let sent = {
        let _send = line::TALK_SEND.lock().unwrap_or_else(|err| err.into_inner());
        client.send_contact_for_window(&chat, &mid)
    };
    if sent.get("ok") == Some(&Json::Bool(true)) {
        let _ = store::save_outgoing(&chat, &sent);
    }
    sent
}

fn unsend_message(args: &Json) -> Json {
    let message_id = text(args, "messageId");
    if !message_ok(&message_id) {
        return json!({ "error": "That message cannot be unsent." });
    }
    let Some(mut client) = account() else {
        return signed_out();
    };
    let result = {
        let _send = line::TALK_SEND.lock().unwrap_or_else(|err| err.into_inner());
        client.unsend_message(&message_id)
    };
    match result {
        Ok(()) => {
            let _ = store::mark_unsent(&message_id);
            json!({ "ok": true, "messageId": message_id })
        }
        Err(err) => failed(err),
    }
}

fn react_to_message(args: &Json) -> Json {
    let message_id = text(args, "messageId");
    let emoji = text(args, "emoji");
    if !message_ok(&message_id) || !matches!(emoji.as_str(), "👍" | "😎" | "😍" | "😠" | "🥺" | "😭" | "😮") {
        return json!({ "error": "That reaction is not available." });
    }
    let Some(mut client) = account() else {
        return signed_out();
    };
    match client.send_reaction(&message_id, &emoji) {
        Ok(()) => json!({ "ok": true, "messageId": message_id, "emoji": emoji }),
        Err(err) => failed(err),
    }
}

fn mark_chat_read(args: &Json) -> Json {
    let chat = text(args, "chat");
    let message_id = text(args, "messageId");
    if !chat_ok(&chat) || !message_ok(&message_id) {
        return json!({ "error": "That chat cannot be marked read." });
    }
    let Some(mut client) = account() else {
        return signed_out();
    };
    match client.mark_chat_read(&chat, &message_id) {
        Ok(()) => {
            let _ = store::apply_poll(&json!({
                "unreads": [{ "id": chat, "unread": 0 }],
                "updates": [{ "id": chat, "read": true, "messageId": message_id, "lastMessage": "", "time": 0, "sealed": false }]
            }));
            json!({ "ok": true, "chat": chat, "messageId": message_id })
        }
        Err(err) => failed(err),
    }
}

fn add_friend(args: &Json) -> Json {
    let id = text(args, "id");
    if !user_ok(&id) {
        return json!({ "error": "That person cannot be added." });
    }
    let Some(mut client) = account() else {
        return signed_out();
    };
    match client.add_contact(&id) {
        Ok(()) => json!({ "ok": true, "id": id }),
        Err(err) => failed(err),
    }
}

fn set_blocked(args: &Json) -> Json {
    let id = text(args, "id");
    if !user_ok(&id) {
        return json!({ "error": "That profile cannot be updated." });
    }
    let Ok(blocked) = flag(args, "blocked") else {
        return json!({ "error": "Pass blocked as true or false." });
    };
    let Some(mut client) = account() else {
        return signed_out();
    };
    match client.set_blocked(&id, blocked) {
        Ok(()) => json!({ "ok": true, "id": id, "blocked": blocked }),
        Err(err) => failed(err),
    }
}

fn set_favorite(args: &Json) -> Json {
    let id = text(args, "id");
    if !user_ok(&id) {
        return json!({ "error": "That profile cannot be updated." });
    }
    let Ok(favorite) = flag(args, "favorite") else {
        return json!({ "error": "Pass favorite as true or false." });
    };
    let value = if favorite { "true" } else { "false" };
    let Some(mut client) = account() else {
        return signed_out();
    };
    match client.update_contact_setting(&id, 8, value) {
        Ok(()) => json!({ "ok": true, "id": id, "favorite": favorite }),
        Err(err) => failed(err),
    }
}

fn rename_friend(args: &Json) -> Json {
    let id = text(args, "id");
    let name = text(args, "name");
    if !user_ok(&id) {
        return json!({ "error": "That profile cannot be renamed." });
    }
    if name.is_empty() || name.chars().count() > 20 || name.chars().any(|ch| ch.is_control()) {
        return json!({ "error": "That name cannot be used." });
    }
    let Some(mut client) = account() else {
        return signed_out();
    };
    match client.update_contact_setting(&id, 2, &name) {
        Ok(()) => json!({ "ok": true, "id": id, "name": name }),
        Err(err) => failed(err),
    }
}

fn create_room(args: &Json) -> Json {
    let Ok(members) = member_ids(args) else {
        return json!({ "error": "Choose at least one friend." });
    };
    let Some(mut client) = account() else {
        return signed_out();
    };
    match client.create_room(&members) {
        Ok(made) => made,
        Err(err) => failed(err),
    }
}

fn leave_chat(args: &Json) -> Json {
    let chat = text(args, "chat");
    if !group_ok(&chat) && !((2..=64).contains(&chat.len()) && chat.starts_with('r') && chat.chars().all(|ch| ch.is_ascii_alphanumeric())) {
        return json!({ "error": "That chat cannot be left." });
    }
    let Some(mut client) = account() else {
        return signed_out();
    };
    match client.leave_chat(&chat) {
        Ok(()) => json!({ "ok": true }),
        Err(err) => failed(err),
    }
}

fn reissue_invite_link(args: &Json) -> Json {
    let group = text(args, "group");
    if !group_ok(&group) {
        return json!({ "error": "That group cannot be opened." });
    }
    let Some(mut client) = account() else {
        return signed_out();
    };
    match client.reissue_group_ticket(&group) {
        Ok(card) => json!({
            "ok": true,
            "inviteUrl": card.get("inviteUrl").and_then(|value| value.as_str()).unwrap_or(""),
        }),
        Err(err) => failed(err),
    }
}

fn set_invite_link(args: &Json) -> Json {
    let group = text(args, "group");
    let Some(enabled) = args.get("enabled").and_then(|value| value.as_bool()) else {
        return json!({ "error": "Pass enabled as true or false." });
    };
    if !group_ok(&group) {
        return json!({ "error": "That group cannot be opened." });
    }
    let Some(mut client) = account() else {
        return signed_out();
    };
    match client.set_group_invite_link(&group, enabled) {
        Ok(_) => json!({ "ok": true, "enabled": enabled }),
        Err(err) => failed(err),
    }
}

fn create_group(args: &Json) -> Json {
    let name = text(args, "name");
    let Ok(members) = member_ids(args) else {
        return json!({ "error": "Choose at least one friend." });
    };
    if name.chars().count() > 50 || name.chars().any(|ch| ch.is_control()) {
        return json!({ "error": "That name cannot be used." });
    }
    let Some(mut client) = account() else {
        return signed_out();
    };
    match client.create_group(&name, &members) {
        Ok(created) => created,
        Err(err) => failed(err),
    }
}

fn rename_group(args: &Json) -> Json {
    let group = text(args, "group");
    let name = text(args, "name");
    if !group_ok(&group) {
        return json!({ "error": "That group cannot be renamed." });
    }
    if name.is_empty() || name.chars().count() > 50 || name.chars().any(|ch| ch.is_control()) {
        return json!({ "error": "That name cannot be used." });
    }
    let Some(mut client) = account() else {
        return signed_out();
    };
    match client.rename_group(&group, &name) {
        Ok(()) => json!({ "ok": true, "group": group, "name": name }),
        Err(err) => failed(err),
    }
}

fn invite_to_group(args: &Json) -> Json {
    let group = text(args, "group");
    if !group_ok(&group) {
        return json!({ "error": "That group cannot be opened." });
    }
    let Ok(members) = member_ids(args) else {
        return json!({ "error": "Choose at least one friend." });
    };
    let Some(mut client) = account() else {
        return signed_out();
    };
    match client.invite_into_group(&group, &members) {
        Ok(()) => json!({ "ok": true, "group": group, "members": members }),
        Err(err) => failed(err),
    }
}

fn remove_group_member(args: &Json) -> Json {
    let group = text(args, "group");
    let id = text(args, "id");
    if !group_ok(&group) || !user_ok(&id) {
        return json!({ "error": "That member cannot be removed." });
    }
    let Some(mut client) = account() else {
        return signed_out();
    };
    match client.remove_group_member(&group, &id) {
        Ok(()) => json!({ "ok": true, "group": group, "id": id }),
        Err(err) => failed(err),
    }
}

fn set_group_auto_join(args: &Json) -> Json {
    let group = text(args, "group");
    if !group_ok(&group) {
        return json!({ "error": "That group cannot be updated." });
    }
    let Ok(enabled) = flag(args, "enabled") else {
        return json!({ "error": "Pass enabled as true or false." });
    };
    let Some(mut client) = account() else {
        return signed_out();
    };
    match client.set_group_auto_join(&group, enabled) {
        Ok(()) => json!({ "ok": true, "group": group, "enabled": enabled }),
        Err(err) => failed(err),
    }
}

fn report_message(args: &Json) -> Json {
    let chat = text(args, "chat");
    let sender = text(args, "sender");
    let message_id = text(args, "messageId");
    let reason = match text(args, "reason").as_str() {
        "other" => 0,
        "advertising" => 1,
        "gender" => 2,
        "harassment" => 3,
        _ => return json!({ "error": "Choose a report reason." }),
    };
    if !chat_ok(&chat) || !user_ok(&sender) || !message_ok(&message_id) {
        return json!({ "error": "That message cannot be reported." });
    }
    let note: String = text(args, "text").chars().take(1000).collect();
    let Some(mut client) = account() else {
        return signed_out();
    };
    match client.report_message(&chat, &sender, &message_id, reason, &note) {
        Ok(()) => json!({ "ok": true }),
        Err(err) => failed(err),
    }
}

fn person(card: &Json) -> Json {
    json!({
        "id": card.get("id").cloned().unwrap_or(Json::String(String::new())),
        "name": card.get("name").cloned().unwrap_or(Json::String(String::new())),
        "status": card.get("status").cloned().unwrap_or(Json::String(String::new())),
        "friend": card.get("friend").and_then(Json::as_bool).unwrap_or(false),
        "blocked": card.get("blocked").and_then(Json::as_bool).unwrap_or(false),
        "favorite": card.get("favorite").and_then(Json::as_bool).unwrap_or(false),
    })
}

fn account() -> Option<Client> {
    if line::signed_in() { Some(Client::new()) } else { None }
}

fn signed_out() -> Json {
    json!({ "error": "Sign in to LINE first." })
}

fn failed(err: line::LineError) -> Json {
    json!({ "error": err.to_string() })
}

fn text(args: &Json, key: &str) -> String {
    args.get(key).and_then(Json::as_str).unwrap_or("").trim().to_string()
}

fn flag(args: &Json, key: &str) -> Result<bool, ()> {
    match args.get(key) {
        Some(Json::Bool(value)) => Ok(*value),
        Some(Json::String(text)) if text == "true" || text == "false" => Ok(text == "true"),
        _ => Err(()),
    }
}

fn member_ids(args: &Json) -> Result<Vec<String>, ()> {
    let list = args.get("members").and_then(Json::as_array).ok_or(())?;
    if list.is_empty() || list.len() > 100 {
        return Err(());
    }
    let mut ids = Vec::new();
    for item in list {
        let id = item.as_str().unwrap_or("").trim();
        if !user_ok(id) || ids.iter().any(|seen| seen == id) {
            return Err(());
        }
        ids.push(id.to_string());
    }
    Ok(ids)
}

fn limit_of(args: &Json, key: &str, default: usize, max: usize) -> usize {
    let Some(raw) = args.get(key) else {
        return default;
    };
    let parsed = raw
        .as_u64()
        .or_else(|| raw.as_i64().filter(|value| *value > 0).map(|value| value as u64))
        .or_else(|| raw.as_str().and_then(|text| text.trim().parse::<u64>().ok()));
    parsed.unwrap_or(default as u64).clamp(1, max as u64) as usize
}

fn chat_ok(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.chars().all(|ch| ch.is_ascii_alphanumeric())
}

fn user_ok(id: &str) -> bool {
    (2..=64).contains(&id.len()) && id.starts_with('u') && id.chars().all(|ch| ch.is_ascii_alphanumeric())
}

fn group_ok(id: &str) -> bool {
    (2..=64).contains(&id.len()) && id.starts_with('c') && id.chars().all(|ch| ch.is_ascii_alphanumeric())
}

fn message_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 24 && id.chars().all(|ch| ch.is_ascii_digit())
}

fn sticker_ok(id: &str) -> bool {
    (1..=32).contains(&id.len()) && id.chars().all(|ch| ch.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_tools_have_unique_names_and_search_is_one_of_them() {
        let tools = offered();
        let mut names: Vec<_> = tools.iter().map(|tool| tool.name.as_str()).collect();
        assert!(names.contains(&"search_users"));
        assert!(names.contains(&"send_text"));
        assert!(names.contains(&"send_contact"));
        assert!(names.contains(&"list_group_members"));
        names.sort_unstable();
        let mut unique = names.clone();
        unique.dedup();
        assert_eq!(names, unique);
        assert!(tools.iter().all(|tool| tool.schema["type"] == "object" && !tool.description.is_empty()));
    }

    #[test]
    fn a_blank_user_search_is_rejected_before_line_is_called() {
        let result = invoke("search_users", &json!({ "query": "  " }));
        assert_eq!(result["error"], "Enter a name, LINE ID, or phone number.");
    }

    #[test]
    fn a_phrase_is_not_sent_to_the_line_directory() {
        assert!(!directory_query("ada park"));
        assert!(directory_query("ada"));
        assert!(directory_query("+819012345678"));
        assert!(directory_query("09012345678"));
    }

    #[test]
    fn changing_tools_require_the_fields_they_name() {
        assert_eq!(invoke("set_blocked", &json!({ "id": "not-a-user", "blocked": "yes" }))["error"], "That profile cannot be updated.");
        assert_eq!(invoke("set_blocked", &json!({ "id": "uAbcdef", "blocked": "yes" }))["error"], "Pass blocked as true or false.");
        assert_eq!(invoke("report_message", &json!({ "chat": "uAbc", "sender": "uAbc", "messageId": "12", "reason": "spam" }))["error"], "Choose a report reason.");
        assert_eq!(invoke("send_text", &json!({ "chat": "uAbc", "text": "" }))["error"], "Enter a message first.");
        assert_eq!(invoke("send_contact", &json!({ "chat": "uAbc", "mid": "group" }))["error"], "That contact cannot be sent.");
        assert_eq!(invoke("create_group", &json!({ "name": "Picnic", "members": [] }))["error"], "Choose at least one friend.");
        assert_eq!(invoke("create_room", &json!({ "members": [] }))["error"], "Choose at least one friend.");
        assert_eq!(invoke("leave_chat", &json!({ "chat": "nope" }))["error"], "That chat cannot be left.");
        assert_eq!(invoke("set_invite_link", &json!({ "group": "cAbc", "enabled": "yes" }))["error"], "Pass enabled as true or false.");
    }
}
