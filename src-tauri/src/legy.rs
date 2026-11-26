//! LEGY encrypted gateway for authenticated Talk calls.
//!
//! A desktop v3 JWT is sent as `x-line-access` on plain `/S4`. Primary
//! `mid:secret` tokens are an AES-128-CBC blob to `gf.line.naver.jp/enc`.

use aes::Aes128;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use cbc::{Decryptor, Encryptor};
use rand::RngCore;
use rsa::oaep::Oaep;
use rsa::pkcs8::DecodePublicKey;
use rsa::RsaPublicKey;
use sha1::Sha1;

pub const LEGY_HOST: &str = "gf.line.naver.jp";
pub const LEGY_PATH: &str = "/enc";
pub const LEGY_LE: u8 = 7;
pub const LEGY_LAP: &str = "5";

const LEGY_IV: [u8; 16] = [78, 9, 72, 62, 56, 245, 255, 114, 128, 18, 123, 158, 251, 92, 45, 51];

const LINE_PUBLIC_KEY_PEM: &str = "-----BEGIN PUBLIC KEY-----\n\
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAsMC6HAYeMq4R59e2yRw6\n\
W1OWT2t9aepiAp4fbSCXzRj7A29BOAFAvKlzAub4oxN13Nt8dbcB+ICAufyDnN5N\n\
d3+vXgDxEXZ/sx2/wuFbC3B3evSNKR4hKcs80suRs8aL6EeWi+bAU2oYIc78Bbqh\n\
Nzx0WCzZSJbMBFw1VlsU/HQ/XdiUufopl5QSa0S246XXmwJmmXRO0v7bNvrxaNV0\n\
cbviGkOvTlBt1+RerIFHMTw3SwLDnCOolTz3CuE5V2OrPZCmC0nlmPRzwUfxoxxs\n\
/6qFdpZNoORH/s5mQenSyqPkmH8TBOlHJWPH3eN1k6aZIlK5S54mcUb/oNRRq9wD\n\
1wIDAQAB\n\
-----END PUBLIC KEY-----";

pub fn xxhash32(data: &[u8], seed: u32) -> u32 {
    const PRIME1: u32 = 0x9E37_79B1;
    const PRIME2: u32 = 0x85EB_CA77;
    const PRIME3: u32 = 0xC2B2_AE3D;
    const PRIME4: u32 = 0x27D4_EB2F;
    const PRIME5: u32 = 0x1656_67B1;
    fn rotl(value: u32, bits: u32) -> u32 {
        value.rotate_left(bits)
    }
    fn round(mut acc: u32, lane: u32) -> u32 {
        acc = acc.wrapping_add(lane.wrapping_mul(PRIME2));
        acc = rotl(acc, 13);
        acc.wrapping_mul(PRIME1)
    }
    let length = data.len();
    let mut offset = 0;
    let mut digest = if length >= 16 {
        let mut v1 = seed.wrapping_add(PRIME1).wrapping_add(PRIME2);
        let mut v2 = seed.wrapping_add(PRIME2);
        let mut v3 = seed;
        let mut v4 = seed.wrapping_sub(PRIME1);
        while offset + 16 <= length {
            v1 = round(v1, u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()));
            v2 = round(v2, u32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap()));
            v3 = round(v3, u32::from_le_bytes(data[offset + 8..offset + 12].try_into().unwrap()));
            v4 = round(v4, u32::from_le_bytes(data[offset + 12..offset + 16].try_into().unwrap()));
            offset += 16;
        }
        rotl(v1, 1)
            .wrapping_add(rotl(v2, 7))
            .wrapping_add(rotl(v3, 12))
            .wrapping_add(rotl(v4, 18))
    } else {
        seed.wrapping_add(PRIME5)
    };
    digest = digest.wrapping_add(length as u32);
    while offset + 4 <= length {
        let lane = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
        digest = digest.wrapping_add(lane.wrapping_mul(PRIME3));
        digest = rotl(digest, 17);
        digest = digest.wrapping_mul(PRIME4);
        offset += 4;
    }
    while offset < length {
        digest = digest.wrapping_add((data[offset] as u32).wrapping_mul(PRIME5));
        digest = rotl(digest, 11);
        digest = digest.wrapping_mul(PRIME1);
        offset += 1;
    }
    digest ^= digest >> 15;
    digest = digest.wrapping_mul(PRIME2);
    digest ^= digest >> 13;
    digest = digest.wrapping_mul(PRIME3);
    digest ^= digest >> 16;
    digest
}

pub fn legy_mac(key: &[u8], data: &[u8]) -> [u8; 4] {
    let opad: Vec<u8> = key.iter().map(|byte| byte ^ 0x5c).collect();
    let ipad: Vec<u8> = key.iter().map(|byte| byte ^ 0x36).collect();
    let mut inner = ipad;
    inner.extend_from_slice(data);
    let inner = xxhash32(&inner, 0).to_be_bytes();
    let mut outer = opad;
    outer.extend_from_slice(&inner);
    xxhash32(&outer, 0).to_be_bytes()
}

fn encode_headers(headers: &[(&str, &str)]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&(headers.len() as u16).to_be_bytes());
    for (key, value) in headers {
        let key_raw = key.as_bytes();
        let value_raw = value.as_bytes();
        body.extend_from_slice(&(key_raw.len() as u16).to_be_bytes());
        body.extend_from_slice(key_raw);
        body.extend_from_slice(&(value_raw.len() as u16).to_be_bytes());
        body.extend_from_slice(value_raw);
    }
    let mut out = Vec::new();
    out.extend_from_slice(&(body.len() as u16).to_be_bytes());
    out.extend(body);
    out
}

fn decode_headers(data: &[u8]) -> Result<(Vec<(String, String)>, &[u8]), String> {
    if data.len() < 4 {
        return Err("truncated LEGY header".into());
    }
    let block = u16::from_be_bytes([data[0], data[1]]) as usize;
    let end = block + 2;
    if end > data.len() {
        return Err("truncated LEGY header".into());
    }
    let count = u16::from_be_bytes([data[2], data[3]]) as usize;
    let mut offset = 4;
    let mut headers = Vec::new();
    for _ in 0..count {
        if offset + 2 > end {
            return Err("truncated LEGY header".into());
        }
        let key_len = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
        offset += 2;
        let key = String::from_utf8_lossy(&data[offset..offset + key_len]).into_owned();
        offset += key_len;
        let value_len = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
        offset += 2;
        let value = String::from_utf8_lossy(&data[offset..offset + value_len]).into_owned();
        offset += value_len;
        headers.push((key, value));
    }
    if offset != end {
        return Err("LEGY header length does not match its fields".into());
    }
    Ok((headers, &data[end..]))
}

fn aes_encrypt(key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let pad = 16 - (plaintext.len() % 16);
    let mut padded = plaintext.to_vec();
    padded.extend(std::iter::repeat(pad as u8).take(pad));
    let encryptor = Encryptor::<Aes128>::new_from_slices(key, &LEGY_IV).map_err(|err| err.to_string())?;
    encryptor
        .encrypt_padded_mut::<cbc::cipher::block_padding::NoPadding>(&mut padded, plaintext.len() + pad)
        .map_err(|err| err.to_string())?;
    Ok(padded)
}

fn aes_decrypt(key: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    if ciphertext.is_empty() || ciphertext.len() % 16 != 0 {
        return Err("LEGY ciphertext is not a full AES block".into());
    }
    let mut buffer = ciphertext.to_vec();
    let decryptor = Decryptor::<Aes128>::new_from_slices(key, &LEGY_IV).map_err(|err| err.to_string())?;
    let padded = decryptor
        .decrypt_padded_mut::<cbc::cipher::block_padding::NoPadding>(&mut buffer)
        .map_err(|err| err.to_string())?
        .to_vec();
    let pad = *padded.last().ok_or("LEGY padding is invalid")? as usize;
    if !(1..=16).contains(&pad) || padded[padded.len() - pad..] != vec![pad as u8; pad] {
        return Err("LEGY padding is invalid".into());
    }
    Ok(padded[..padded.len() - pad].to_vec())
}

pub struct LegyPlaintext {
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub struct LegyTransport {
    aes_key: [u8; 16],
    x_lcs: Option<String>,
}

impl LegyTransport {
    pub fn new() -> Self {
        let mut aes_key = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut aes_key);
        Self { aes_key, x_lcs: None }
    }

    pub fn x_lcs(&mut self) -> Result<String, String> {
        if self.x_lcs.is_none() {
            let public = RsaPublicKey::from_public_key_pem(LINE_PUBLIC_KEY_PEM)
                .map_err(|err| format!("LINE gateway key: {err}"))?;
            let encrypted = public
                .encrypt(&mut rand::thread_rng(), Oaep::new::<Sha1>(), &self.aes_key)
                .map_err(|err| format!("could not seal the gateway key: {err}"))?;
            self.x_lcs = Some(format!("0008{}", STANDARD.encode(encrypted)));
        }
        Ok(self.x_lcs.clone().unwrap_or_default())
    }

    pub fn seal(&self, path: &str, access_token: &str, thrift_body: &[u8]) -> Result<Vec<u8>, String> {
        let mut headers = vec![("x-lpqs", path)];
        if !access_token.is_empty() {
            headers.push(("x-lt", access_token));
        }
        let mut payload = vec![LEGY_LE];
        payload.extend(encode_headers(&headers));
        payload.extend_from_slice(thrift_body);
        let mut encrypted = aes_encrypt(&self.aes_key, &payload)?;
        if LEGY_LE & 2 != 0 {
            encrypted.extend_from_slice(&legy_mac(&self.aes_key, &encrypted));
        }
        Ok(encrypted)
    }

    pub fn open(&self, blob: &[u8]) -> Result<LegyPlaintext, String> {
        let ciphertext = if !blob.is_empty() && blob.len() % 16 == 4 {
            &blob[..blob.len() - 4]
        } else {
            blob
        };
        let mut decrypted = aes_decrypt(&self.aes_key, ciphertext)?;
        if decrypted.first() == Some(&LEGY_LE) {
            decrypted.remove(0);
        }
        let (headers, body) = decode_headers(&decrypted)?;
        Ok(LegyPlaintext {
            headers,
            body: body.to_vec(),
        })
    }
}

pub fn looks_like_jwt(value: &str) -> bool {
    let mut parts = value.split('.');
    let Some(header) = parts.next() else { return false };
    if parts.next().is_none() || parts.next().is_none() || parts.next().is_some() {
        return false;
    }
    let padded = format!("{header}{}", "=".repeat((4 - header.len() % 4) % 4));
    let Ok(raw) = base64::engine::general_purpose::URL_SAFE.decode(padded) else {
        return false;
    };
    raw.starts_with(b"{") && raw.windows(3).any(|window| window == b"alg")
}

pub fn needs_legy(token: &str) -> bool {
    let value = token.trim();
    if value.is_empty() || looks_like_jwt(value) {
        return false;
    }
    let Some(colon) = value.find(':') else { return false };
    let payload = &value[colon + 1..];
    if looks_like_jwt(payload) {
        return true;
    }
    let mid = &value[..colon];
    if mid.len() == 33 && mid.chars().next().is_some_and(|ch| ch.is_ascii_alphabetic()) {
        return STANDARD.decode(payload).map(|raw| !raw.is_empty()).unwrap_or(false);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xxhash_matches_the_gateway_vectors() {
        assert_eq!(xxhash32(b"", 0), 0x02cc_5d05);
        assert_eq!(xxhash32(b"hello", 0), 0xfb00_77f9);
        assert_eq!(legy_mac(b"kkkkkkkkkkkkkkkk", b"data"), [0xc1, 0xbe, 0x54, 0x6a]);
    }

    #[test]
    fn seal_roundtrip_keeps_the_thrift_body() {
        let transport = LegyTransport::new();
        let sealed = transport.seal("/S4", "token", b"\x82frame").unwrap();
        let opened = transport.open(&sealed).unwrap();
        assert_eq!(opened.body, b"\x82frame");
        assert!(opened.headers.iter().any(|(key, value)| key == "x-lpqs" && value == "/S4"));
    }
}
