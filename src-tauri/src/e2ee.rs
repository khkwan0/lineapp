//! LINE letter sealing for 1:1 text.
//!
//! Version 2 is AES-GCM. The key is SHA-256 of the X25519 shared secret, a
//! random salt, and the bytes `Key`.

use aes::Aes256;
use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use cbc::{Decryptor, Encryptor};
use ctr::cipher::StreamCipher;
use ctr::Ctr32BE;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
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

fn aad(recipient: &str, sender: &str, sender_key_id: i64, receiver_key_id: i64, spec_version: i64, content_type: i64) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(recipient.as_bytes());
    out.extend_from_slice(sender.as_bytes());
    out.extend_from_slice(&int32_bytes(sender_key_id));
    out.extend_from_slice(&int32_bytes(receiver_key_id));
    out.extend_from_slice(&int32_bytes(spec_version));
    out.extend_from_slice(&int32_bytes(content_type));
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
    seal_raw(
        private_key,
        sender_key_id,
        receiver_public,
        receiver_key_id,
        sender,
        recipient,
        &payload(text),
        spec_version,
        0,
    )
}

/// Letter-seal an arbitrary JSON body. `content_type` is the Talk content type
/// and is part of the associated data, so a photo is sealed as type 1, not as text.
pub fn seal_raw(
    private_key: &[u8],
    sender_key_id: i64,
    receiver_public: &[u8],
    receiver_key_id: i64,
    sender: &str,
    recipient: &str,
    body: &[u8],
    spec_version: i64,
    content_type: i64,
) -> Result<Vec<Vec<u8>>, String> {
    let secret = shared_secret(private_key, receiver_public)?;
    let mut salt = vec![0u8; 16];
    let mut nonce = vec![0u8; 12];
    rand::thread_rng().fill_bytes(&mut salt);
    rand::thread_rng().fill_bytes(&mut nonce);
    let extra = aad(recipient, sender, sender_key_id, receiver_key_id, spec_version, content_type);
    let encrypted = if spec_version >= 2 {
        let mut material = secret.to_vec();
        material.extend_from_slice(&salt);
        material.extend_from_slice(b"Key");
        let gcm_key = Sha256::digest(&material);
        let cipher = Aes256Gcm::new_from_slice(&gcm_key).map_err(|err| err.to_string())?;
        cipher
            .encrypt(Nonce::from_slice(&nonce), Payload { msg: body, aad: &extra })
            .map_err(|_| "could not seal the message".to_string())?
    } else {
        seal_cbc(&secret, &salt, body)?
    };
    Ok(vec![
        salt,
        encrypted,
        nonce,
        int32_bytes(sender_key_id).to_vec(),
        int32_bytes(receiver_key_id).to_vec(),
    ])
}

/// Encrypt a file the way LINE object storage expects a letter-sealed attachment.
///
/// HKDF-SHA256 with info `FileEncryption` yields the AES-256-CTR key, the HMAC
/// key, and a 12-byte nonce. The blob is ciphertext followed by HMAC-SHA256.
pub fn encrypt_file(plain: &[u8]) -> Result<(String, Vec<u8>), String> {
    let mut key_material = vec![0u8; 32];
    rand::thread_rng().fill_bytes(&mut key_material);
    let body = encrypt_file_with(&key_material, plain)?;
    Ok((STANDARD.encode(&key_material), body))
}

fn encrypt_file_with(key_material: &[u8], plain: &[u8]) -> Result<Vec<u8>, String> {
    let mut okm = [0u8; 76];
    Hkdf::<Sha256>::new(None, key_material)
        .expand(b"FileEncryption", &mut okm)
        .map_err(|_| "could not derive the file key".to_string())?;
    let mut iv = [0u8; 16];
    iv[..12].copy_from_slice(&okm[64..76]);
    let mut body = plain.to_vec();
    let mut cipher = Ctr32BE::<Aes256>::new_from_slices(&okm[..32], &iv).map_err(|_| "could not seal the file".to_string())?;
    cipher.apply_keystream(&mut body);
    let mut signer = <Hmac<Sha256> as Mac>::new_from_slice(&okm[32..64]).map_err(|err| err.to_string())?;
    signer.update(&body);
    let mac = signer.finalize().into_bytes();
    body.extend_from_slice(&mac);
    Ok(body)
}

pub fn open_text(
    chunks: &[Vec<u8>],
    private_key: &[u8],
    sender_public: &[u8],
    sender: &str,
    recipient: &str,
    spec_version: i64,
) -> Result<String, String> {
    let raw = open_bytes(chunks, private_key, sender_public, sender, recipient, spec_version, 0)?;
    let body: serde_json::Value = serde_json::from_slice(&raw).map_err(|_| "sealed text was not json".to_string())?;
    Ok(body.get("text").and_then(|value| value.as_str()).unwrap_or("").to_string())
}

pub fn open_bytes(
    chunks: &[Vec<u8>],
    private_key: &[u8],
    sender_public: &[u8],
    sender: &str,
    recipient: &str,
    spec_version: i64,
    content_type: i64,
) -> Result<Vec<u8>, String> {
    if chunks.len() < 5 {
        return Err("letter-sealed message is incomplete".into());
    }
    let secret = shared_secret(private_key, sender_public)?;
    let sender_key_id = i64::from(u32::from_be_bytes(chunks[3].as_slice().try_into().map_err(|_| "bad key id")?));
    let receiver_key_id = i64::from(u32::from_be_bytes(chunks[4].as_slice().try_into().map_err(|_| "bad key id")?));
    let extra = aad(recipient, sender, sender_key_id, receiver_key_id, spec_version, content_type);
    if spec_version >= 2 {
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
            .map_err(|_| "could not open the letter-sealed message".to_string())
    } else {
        open_cbc(&secret, &chunks[0], &chunks[1])
    }
}

pub fn decrypt_file(data: &[u8], key_material: &str) -> Result<Vec<u8>, String> {
    let material = decode_b64(key_material)?;
    let mut okm = [0u8; 76];
    Hkdf::<Sha256>::new(None, &material)
        .expand(b"FileEncryption", &mut okm)
        .map_err(|_| "could not derive the file key".to_string())?;
    let mut body = data.to_vec();
    if body.len() > 32 {
        let mac = body.split_off(body.len() - 32);
        let mut signer = <Hmac<Sha256> as Mac>::new_from_slice(&okm[32..64]).map_err(|err| err.to_string())?;
        signer.update(&body);
        if signer.verify_slice(&mac).is_err() {
            body.extend_from_slice(&mac);
        }
    }
    let mut iv = [0u8; 16];
    iv[..12].copy_from_slice(&okm[64..76]);
    let mut cipher = Ctr32BE::<Aes256>::new_from_slices(&okm[..32], &iv).map_err(|_| "could not open the file".to_string())?;
    cipher.apply_keystream(&mut body);
    Ok(body)
}

pub fn open_group_private(private_key: &[u8], creator_public: &[u8], encrypted: &[u8]) -> Option<Vec<u8>> {
    let secret = shared_secret(private_key, creator_public).ok()?;
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
    let raw = strip_pad(&padded).unwrap_or(padded);
    if raw.len() == 32 { Some(raw) } else { None }
}

fn decode_b64(raw: &str) -> Result<Vec<u8>, String> {
    let mut text = raw.trim().replace("-", "+").replace("_", "/");
    while text.len() % 4 != 0 {
        text.push('=');
    }
    STANDARD.decode(text).map_err(|_| "key material was not valid".to_string())
}

fn strip_pad(padded: &[u8]) -> Option<Vec<u8>> {
    let pad = *padded.last()? as usize;
    if !(1..=16).contains(&pad) || padded.len() < pad || padded[padded.len() - pad..] != vec![pad as u8; pad] {
        return None;
    }
    Some(padded[..padded.len() - pad].to_vec())
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

pub struct ChainKey {
    pub key_id: Option<i64>,
    pub private_key: Vec<u8>,
    pub public_key: Vec<u8>,
}

pub fn decrypt_key_chain(public_key: &[u8], private_key: &[u8], encrypted: &[u8]) -> Option<Vec<ChainKey>> {
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
    let mut found = Vec::new();
    collect_keys(&Value::Struct(fields), &mut found);
    if found.is_empty() { None } else { Some(found) }
}

fn derives(private_key: &[u8], public_key: &[u8]) -> bool {
    let Ok(private) = <[u8; 32]>::try_from(private_key) else { return false };
    let secret = StaticSecret::from(private);
    PublicKey::from(&secret).as_bytes().as_slice() == public_key
}

fn collect_keys(value: &Value, found: &mut Vec<ChainKey>) {
    match value {
        Value::Struct(fields) => {
            if let Some(key) = chain_key(fields) {
                if !found.iter().any(|have| have.private_key == key.private_key) {
                    found.push(key);
                }
            }
            for nested in fields.values() {
                collect_keys(nested, found);
            }
        }
        Value::List(items) => {
            for item in items {
                collect_keys(item, found);
            }
        }
        _ => {}
    }
}

fn chain_key(fields: &std::collections::BTreeMap<i16, Value>) -> Option<ChainKey> {
    let left = match fields.get(&4) {
        Some(Value::Bytes(raw)) if raw.len() == 32 => raw.clone(),
        _ => return None,
    };
    let right = match fields.get(&5) {
        Some(Value::Bytes(raw)) if raw.len() == 32 => raw.clone(),
        _ => return None,
    };
    let key_id = match fields.get(&2) {
        Some(Value::Int(value)) => Some(*value),
        _ => None,
    };
    if derives(&right, &left) {
        Some(ChainKey { key_id, private_key: right, public_key: left })
    } else if derives(&left, &right) {
        Some(ChainKey { key_id, private_key: left, public_key: right })
    } else {
        None
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

    #[test]
    fn file_key_roundtrip() {
        let material = [9u8; 32];
        let plain = b"hello image bytes!!";
        let mut okm = [0u8; 76];
        Hkdf::<Sha256>::new(None, &material).expand(b"FileEncryption", &mut okm).unwrap();
        let mut iv = [0u8; 16];
        iv[..12].copy_from_slice(&okm[64..76]);
        let mut body = plain.to_vec();
        let mut cipher = Ctr32BE::<Aes256>::new_from_slices(&okm[..32], &iv).unwrap();
        cipher.apply_keystream(&mut body);
        let mut signer = <Hmac<Sha256> as Mac>::new_from_slice(&okm[32..64]).unwrap();
        signer.update(&body);
        body.extend_from_slice(&signer.finalize().into_bytes());
        let encoded = STANDARD.encode(material);
        assert_eq!(decrypt_file(&body, &encoded).unwrap(), plain);
        let (fresh, sealed) = encrypt_file(plain).unwrap();
        assert_eq!(decrypt_file(&sealed, &fresh).unwrap(), plain);
    }

    #[test]
    fn key_chain_roundtrip_keeps_the_private_key() {
        let wrapper = StaticSecret::random_from_rng(rand::thread_rng());
        let phone = StaticSecret::random_from_rng(rand::thread_rng());
        let wrapper_public = PublicKey::from(&wrapper);
        let phone_public = PublicKey::from(&phone);
        let body = thrift::write_struct(&[(
            1,
            crate::thrift::Field::List(
                crate::thrift::STRUCT,
                vec![crate::thrift::Field::Struct(vec![
                    (1, crate::thrift::Field::I32(1)),
                    (2, crate::thrift::Field::I32(4231344)),
                    (4, crate::thrift::Field::Bin(phone_public.as_bytes().to_vec())),
                    (5, crate::thrift::Field::Bin(phone.as_bytes().to_vec())),
                ])],
            ),
        )]);
        let secret = shared_secret(wrapper.as_bytes(), wrapper_public.as_bytes()).unwrap();
        let mut key_material = secret.to_vec();
        key_material.extend_from_slice(b"Key");
        let mut iv_material = secret.to_vec();
        iv_material.extend_from_slice(b"IV");
        let key: [u8; 32] = Sha256::digest(&key_material).into();
        let iv = half_xor(&Sha256::digest(&iv_material));
        let pad = 16 - (body.len() % 16);
        let mut buffer = body;
        buffer.extend(std::iter::repeat(pad as u8).take(pad));
        let encryptor = Encryptor::<Aes256>::new_from_slices(&key, &iv).unwrap();
        let length = buffer.len();
        encryptor
            .encrypt_padded_mut::<cbc::cipher::block_padding::NoPadding>(&mut buffer, length)
            .unwrap();
        let found = decrypt_key_chain(wrapper_public.as_bytes(), wrapper.as_bytes(), &buffer).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].key_id, Some(4231344));
        assert_eq!(found[0].private_key, phone.as_bytes());
    }
}
