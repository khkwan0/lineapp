//! LINE letter sealing for 1:1 text.
//!
//! Version 2 is AES-GCM. The key is SHA-256 of the X25519 shared secret, a
//! random salt, and the bytes `Key`.

use aes::Aes256;
use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use cbc::{Decryptor, Encryptor};
use rand::RngCore;
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

use crate::thrift::{self, Value};

pub fn int32_bytes(value: i64) -> [u8; 4] {
    (value as u32).to_be_bytes()
}

pub fn shared_secret(private_key: &[u8], public_key: &[u8]) -> Result<[u8; 32], String> {
    let private: [u8; 32] = private_key
        .try_into()
        .map_err(|_| "letter sealing private key is the wrong size".to_string())?;
    let public: [u8; 32] = public_key
        .try_into()
        .map_err(|_| "letter sealing public key is the wrong size".to_string())?;
    let secret = StaticSecret::from(private);
    Ok(secret.diffie_hellman(&PublicKey::from(public)).to_bytes())
}

fn aad(recipient: &str, sender: &str, sender_key_id: i64, receiver_key_id: i64, spec_version: i64) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(recipient.as_bytes());
    out.extend_from_slice(sender.as_bytes());
    out.extend_from_slice(&int32_bytes(sender_key_id));
    out.extend_from_slice(&int32_bytes(receiver_key_id));
    out.extend_from_slice(&int32_bytes(spec_version));
    out.extend_from_slice(&int32_bytes(0));
    out
}

fn payload(text: &str) -> Vec<u8> {
    // Same spacing as Python json.dumps: {"text": "..."}.
    let encoded = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into());
    format!(r#"{{"text": {encoded}}}"#).into_bytes()
}

pub fn seal_text(
    private_key: &[u8],
    sender_key_id: i64,
    receiver_public: &[u8],
    receiver_key_id: i64,
    sender: &str,
    recipient: &str,
    text: &str,
    spec_version: i64,
) -> Result<Vec<Vec<u8>>, String> {
    let secret = shared_secret(private_key, receiver_public)?;
    let mut salt = vec![0u8; 16];
    let mut nonce = vec![0u8; 12];
    rand::thread_rng().fill_bytes(&mut salt);
    rand::thread_rng().fill_bytes(&mut nonce);
    let extra = aad(recipient, sender, sender_key_id, receiver_key_id, spec_version);
    let encrypted = if spec_version >= 2 {
        let mut material = secret.to_vec();
        material.extend_from_slice(&salt);
        material.extend_from_slice(b"Key");
        let gcm_key = Sha256::digest(&material);
        let cipher = Aes256Gcm::new_from_slice(&gcm_key).map_err(|err| err.to_string())?;
        cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload { msg: &payload(text), aad: &extra },
            )
            .map_err(|_| "could not seal the message".to_string())?
    } else {
        seal_cbc(&secret, &salt, &payload(text))?
    };
    Ok(vec![
        salt,
        encrypted,
        nonce,
        int32_bytes(sender_key_id).to_vec(),
        int32_bytes(receiver_key_id).to_vec(),
    ])
}

pub fn open_text(
    chunks: &[Vec<u8>],
    private_key: &[u8],
    sender_public: &[u8],
    sender: &str,
    recipient: &str,
    spec_version: i64,
) -> Result<String, String> {
    if chunks.len() < 5 {
        return Err("letter-sealed message is incomplete".into());
    }
    let secret = shared_secret(private_key, sender_public)?;
    let sender_key_id = i64::from(u32::from_be_bytes(chunks[3].as_slice().try_into().map_err(|_| "bad key id")?));
    let receiver_key_id = i64::from(u32::from_be_bytes(chunks[4].as_slice().try_into().map_err(|_| "bad key id")?));
    let extra = aad(recipient, sender, sender_key_id, receiver_key_id, spec_version);
    let raw = if spec_version >= 2 {
        let mut material = secret.to_vec();
        material.extend_from_slice(&chunks[0]);
        material.extend_from_slice(b"Key");
        let gcm_key = Sha256::digest(&material);
        let cipher = Aes256Gcm::new_from_slice(&gcm_key).map_err(|err| err.to_string())?;
        cipher
            .decrypt(
                Nonce::from_slice(&chunks[2]),
                Payload { msg: &chunks[1], aad: &extra },
            )
            .map_err(|_| "could not open the letter-sealed message".to_string())?
    } else {
        open_cbc(&secret, &chunks[0], &chunks[1])?
    };
    let body: serde_json::Value = serde_json::from_slice(&raw).map_err(|_| "sealed text was not json".to_string())?;
    Ok(body.get("text").and_then(|value| value.as_str()).unwrap_or("").to_string())
}

fn half_xor(buf: &[u8]) -> Vec<u8> {
    let half = buf.len() / 2;
    buf[..half].iter().zip(&buf[half..]).map(|(left, right)| left ^ right).collect()
}

fn seal_cbc(secret: &[u8], salt: &[u8], payload: &[u8]) -> Result<Vec<u8>, String> {
    let (key, iv) = cbc_key(secret, salt);
    let pad = 16 - (payload.len() % 16);
    let mut buffer = payload.to_vec();
    buffer.extend(std::iter::repeat(pad as u8).take(pad));
    let encryptor = Encryptor::<Aes256>::new_from_slices(&key, &iv).map_err(|err| err.to_string())?;
    let length = buffer.len();
    encryptor
        .encrypt_padded_mut::<cbc::cipher::block_padding::NoPadding>(&mut buffer, length)
        .map_err(|err| err.to_string())?;
    Ok(buffer)
}

fn open_cbc(secret: &[u8], salt: &[u8], encrypted: &[u8]) -> Result<Vec<u8>, String> {
    let (key, iv) = cbc_key(secret, salt);
    let mut buffer = encrypted.to_vec();
    let decryptor = Decryptor::<Aes256>::new_from_slices(&key, &iv).map_err(|err| err.to_string())?;
    let padded = decryptor
        .decrypt_padded_mut::<cbc::cipher::block_padding::NoPadding>(&mut buffer)
        .map_err(|err| err.to_string())?
        .to_vec();
    let pad = *padded.last().ok_or("sealed padding is invalid")? as usize;
    if !(1..=16).contains(&pad) || padded[padded.len() - pad..] != vec![pad as u8; pad] {
        return Err("sealed padding is invalid".into());
    }
    Ok(padded[..padded.len() - pad].to_vec())
}

fn cbc_key(secret: &[u8], salt: &[u8]) -> ([u8; 32], Vec<u8>) {
    let mut key_material = secret.to_vec();
    key_material.extend_from_slice(salt);
    key_material.extend_from_slice(b"Key");
    let mut iv_material = secret.to_vec();
    iv_material.extend_from_slice(salt);
    iv_material.extend_from_slice(b"IV");
    let key: [u8; 32] = Sha256::digest(&key_material).into();
    let iv_hash = Sha256::digest(&iv_material);
    (key, half_xor(&iv_hash))
}

pub fn decrypt_key_chain(public_key: &[u8], private_key: &[u8], encrypted: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let secret = shared_secret(private_key, public_key).ok()?;
    let mut key_material = secret.to_vec();
    key_material.extend_from_slice(b"Key");
    let mut iv_material = secret.to_vec();
    iv_material.extend_from_slice(b"IV");
    let key: [u8; 32] = Sha256::digest(&key_material).into();
    let iv = half_xor(&Sha256::digest(&iv_material));
    let mut buffer = encrypted.to_vec();
    let decryptor = Decryptor::<Aes256>::new_from_slices(&key, &iv).ok()?;
    let padded = decryptor
        .decrypt_padded_mut::<cbc::cipher::block_padding::NoPadding>(&mut buffer)
        .ok()?
        .to_vec();
    let raw = if let Some(pad) = padded.last().copied() {
        let pad = pad as usize;
        if (1..=16).contains(&pad) && padded.len() >= pad && padded[padded.len() - pad..] == vec![pad as u8; pad] {
            padded[..padded.len() - pad].to_vec()
        } else {
            padded
        }
    } else {
        return None;
    };
    let fields = thrift::read_struct(&raw).ok()?;
    key_pair(&Value::Struct(fields))
}

fn key_pair(value: &Value) -> Option<(Vec<u8>, Vec<u8>)> {
    match value {
        Value::Struct(fields) => {
            if let (Some(Value::Bytes(public)), Some(Value::Bytes(private))) = (fields.get(&4), fields.get(&5)) {
                if private.len() == 32 {
                    return Some((private.clone(), public.clone()));
                }
            }
            for nested in fields.values() {
                if let Some(found) = key_pair(nested) {
                    return Some(found);
                }
            }
            None
        }
        Value::List(items) => items.iter().find_map(key_pair),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_hello_opens_again() {
        let sender = StaticSecret::random_from_rng(rand::thread_rng());
        let receiver = StaticSecret::random_from_rng(rand::thread_rng());
        let sender_public = PublicKey::from(&sender);
        let receiver_public = PublicKey::from(&receiver);
        let me = "u".to_string() + &"ab".repeat(16);
        let peer = "u".repeat(33);
        let chunks = seal_text(
            sender.as_bytes(),
            4,
            receiver_public.as_bytes(),
            9,
            &me,
            &peer,
            "hello",
            2,
        )
        .unwrap();
        assert!(!chunks[1].windows(5).any(|window| window == b"hello"));
        let opened = open_text(&chunks, receiver.as_bytes(), sender_public.as_bytes(), &me, &peer, 2).unwrap();
        assert_eq!(opened, "hello");
        assert_eq!(chunks[2].len(), 12);
    }
}
