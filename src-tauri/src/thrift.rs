//! Apache Thrift TCompact, the frame LINE posts to legy.

use std::collections::BTreeMap;

pub const PROTOCOL_ID: u8 = 0x82;
pub const VERSION: u8 = 1;
pub const CALL: u8 = 1;
#[allow(dead_code)]
pub const REPLY: u8 = 2;

pub const STOP: u8 = 0;
pub const TRUE: u8 = 1;
pub const FALSE: u8 = 2;
pub const BYTE: u8 = 3;
pub const I32: u8 = 5;
pub const I64: u8 = 6;
pub const DOUBLE: u8 = 7;
pub const BINARY: u8 = 8;
pub const LIST: u8 = 9;
pub const SET: u8 = 10;
pub const MAP: u8 = 11;
pub const STRUCT: u8 = 12;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Byte(i8),
    Int(i64),
    Double(f64),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Map(Vec<(Value, Value)>),
    Struct(BTreeMap<i16, Value>),
}

/// A field to write. Structs are sorted by field id before they hit the wire.
#[derive(Clone, Debug)]
pub enum Field {
    Bool(bool),
    Byte(u8),
    I32(i32),
    I64(i64),
    Bin(Vec<u8>),
    Struct(Vec<(i16, Field)>),
    List(u8, Vec<Field>),
    Set(u8, Vec<Field>),
    MapBin(Vec<(Vec<u8>, Vec<u8>)>),
}

#[allow(dead_code)]
pub struct ThriftMessage {
    pub name: String,
    pub mtype: u8,
    pub seq: i64,
    pub fields: BTreeMap<i16, Value>,
}

pub fn zigzag(n: i64) -> u64 {
    ((n << 1) ^ (n >> 63)) as u64
}

pub fn unzigzag(n: u64) -> i64 {
    ((n >> 1) as i64) ^ -((n & 1) as i64)
}

fn write_varint(buf: &mut Vec<u8>, mut n: u64) {
    loop {
        let byte = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            buf.push(byte);
            return;
        }
        buf.push(byte | 0x80);
    }
}

pub fn write_struct(fields: &[(i16, Field)]) -> Vec<u8> {
    let mut ordered: Vec<&(i16, Field)> = fields.iter().collect();
    ordered.sort_by_key(|item| item.0);
    let mut buf = Vec::new();
    let mut last = 0i16;
    for (fid, value) in ordered {
        let typ = wire_type(value);
        let delta = fid.wrapping_sub(last);
        if delta > 0 && delta <= 15 {
            buf.push(((delta as u8) << 4) | typ);
        } else {
            buf.push(typ);
            write_varint(&mut buf, zigzag(*fid as i64));
        }
        last = *fid;
        write_value(&mut buf, value);
    }
    buf.push(STOP);
    buf
}

fn wire_type(value: &Field) -> u8 {
    match value {
        Field::Bool(true) => TRUE,
        Field::Bool(false) => FALSE,
        Field::Byte(_) => BYTE,
        Field::I32(_) => I32,
        Field::I64(_) => I64,
        Field::Bin(_) => BINARY,
        Field::Struct(_) => STRUCT,
        Field::List(_, _) => LIST,
        Field::Set(_, _) => SET,
        Field::MapBin(_) => MAP,
    }
}

fn write_value(buf: &mut Vec<u8>, value: &Field) {
    match value {
        Field::Bool(_) => {}
        Field::Byte(byte) => buf.push(*byte),
        Field::I32(n) => write_varint(buf, zigzag(*n as i64)),
        Field::I64(n) => write_varint(buf, zigzag(*n)),
        Field::Bin(raw) => {
            write_varint(buf, raw.len() as u64);
            buf.extend_from_slice(raw);
        }
        Field::Struct(fields) => buf.extend(write_struct(fields)),
        Field::List(elem, items) | Field::Set(elem, items) => {
            let size = items.len();
            if size <= 14 {
                buf.push(((size as u8) << 4) | elem);
            } else {
                buf.push(0xf0 | elem);
                write_varint(buf, size as u64);
            }
            for item in items {
                write_value(buf, item);
            }
        }
        Field::MapBin(mapping) => {
            if mapping.is_empty() {
                buf.push(0);
                return;
            }
            write_varint(buf, mapping.len() as u64);
            buf.push((BINARY << 4) | BINARY);
            for (key, item) in mapping {
                write_value(buf, &Field::Bin(key.clone()));
                write_value(buf, &Field::Bin(item.clone()));
            }
        }
    }
}

pub fn write_call(name: &str, fields: &[(i16, Field)]) -> Vec<u8> {
    write_message(CALL, name, fields, 0)
}

#[allow(dead_code)]
pub fn write_reply(name: &str, fields: &[(i16, Field)]) -> Vec<u8> {
    write_message(REPLY, name, fields, 0)
}

fn write_message(mtype: u8, name: &str, fields: &[(i16, Field)], seq: i64) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.push(PROTOCOL_ID);
    buf.push((mtype << 5) | VERSION);
    write_varint(&mut buf, zigzag(seq));
    let raw = name.as_bytes();
    write_varint(&mut buf, raw.len() as u64);
    buf.extend_from_slice(raw);
    buf.extend(write_struct(fields));
    buf
}

struct Reader<'a> {
    data: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn u8(&mut self) -> Result<u8, String> {
        if self.i >= self.data.len() {
            return Err("truncated thrift compact frame".into());
        }
        let byte = self.data[self.i];
        self.i += 1;
        Ok(byte)
    }

    fn raw(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.i + n > self.data.len() {
            return Err("truncated thrift compact frame".into());
        }
        let chunk = &self.data[self.i..self.i + n];
        self.i += n;
        Ok(chunk)
    }

    fn varint(&mut self) -> Result<u64, String> {
        let mut shift = 0;
        let mut number = 0u64;
        for _ in 0..10 {
            let byte = self.u8()?;
            number |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 {
                return Ok(number);
            }
            shift += 7;
        }
        Err("thrift varint is too long".into())
    }

    fn read_struct(&mut self) -> Result<BTreeMap<i16, Value>, String> {
        let mut fields = BTreeMap::new();
        let mut last = 0i16;
        loop {
            let header = self.u8()?;
            if header == STOP {
                return Ok(fields);
            }
            let typ = header & 0x0f;
            let modifier = header >> 4;
            let fid = if modifier == 0 {
                unzigzag(self.varint()?) as i16
            } else {
                last.wrapping_add(modifier as i16)
            };
            last = fid;
            fields.insert(fid, self.read_value(typ)?);
        }
    }

    fn read_value(&mut self, typ: u8) -> Result<Value, String> {
        match typ {
            TRUE => Ok(Value::Bool(true)),
            FALSE => Ok(Value::Bool(false)),
            BYTE => {
                let byte = self.u8()?;
                Ok(Value::Byte(byte as i8))
            }
            4 | I32 | I64 => Ok(Value::Int(unzigzag(self.varint()?))),
            DOUBLE => {
                let raw = self.raw(8)?;
                let mut buf = [0u8; 8];
                buf.copy_from_slice(raw);
                Ok(Value::Double(f64::from_le_bytes(buf)))
            }
            BINARY => {
                let n = self.varint()? as usize;
                Ok(Value::Bytes(self.raw(n)?.to_vec()))
            }
            STRUCT => Ok(Value::Struct(self.read_struct()?)),
            LIST | SET => {
                let header = self.u8()?;
                let mut size = (header >> 4) as usize;
                let elem = header & 0x0f;
                if size == 15 {
                    size = self.varint()? as usize;
                }
                let mut items = Vec::with_capacity(size);
                for _ in 0..size {
                    items.push(self.read_value(elem)?);
                }
                Ok(Value::List(items))
            }
            MAP => {
                let size = self.varint()? as usize;
                if size == 0 {
                    return Ok(Value::Map(Vec::new()));
                }
                let types = self.u8()?;
                let key_type = types >> 4;
                let val_type = types & 0x0f;
                let mut pairs = Vec::with_capacity(size);
                for _ in 0..size {
                    pairs.push((self.read_value(key_type)?, self.read_value(val_type)?));
                }
                Ok(Value::Map(pairs))
            }
            other => Err(format!("unknown thrift type {other}")),
        }
    }
}

pub fn read_struct(data: &[u8]) -> Result<BTreeMap<i16, Value>, String> {
    Reader { data, i: 0 }.read_struct()
}

pub fn read_message(data: &[u8]) -> Result<ThriftMessage, String> {
    let mut reader = Reader { data, i: 0 };
    if reader.u8()? != PROTOCOL_ID {
        return Err("not a TCompact frame".into());
    }
    let ver_type = reader.u8()?;
    if ver_type & 0x1f != VERSION {
        return Err(format!("unsupported TCompact version {}", ver_type & 0x1f));
    }
    let mtype = ver_type >> 5;
    let seq = unzigzag(reader.varint()?);
    let name_len = reader.varint()? as usize;
    let name = String::from_utf8_lossy(reader.raw(name_len)?).into_owned();
    let fields = reader.read_struct()?;
    if reader.i != data.len() {
        return Err(format!(
            "trailing {} byte(s) after TCompact frame",
            data.len() - reader.i
        ));
    }
    Ok(ThriftMessage {
        name,
        mtype,
        seq,
        fields,
    })
}

pub fn bin(text: impl AsRef<[u8]>) -> Field {
    Field::Bin(text.as_ref().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_and_send_match_the_desktop_frames() {
        let profile = write_call("getProfile", &[]);
        assert_eq!(to_hex(&profile), "8221000a67657450726f66696c6500");
        let parsed = read_message(&profile).unwrap();
        assert_eq!(parsed.name, "getProfile");
        assert!(parsed.fields.is_empty());

        let sent = write_call(
            "sendMessage",
            &[
                (1, Field::I32(7)),
                (
                    2,
                    Field::Struct(vec![
                        (2, bin(format!("u{}", "ab".repeat(16)))),
                        (3, Field::I32(0)),
                        (5, Field::I64(0)),
                        (6, Field::I64(0)),
                        (10, bin("hello")),
                        (14, Field::Bool(false)),
                        (15, Field::I32(0)),
                        (18, Field::MapBin(vec![])),
                        (19, Field::Byte(0)),
                    ]),
                ),
            ],
        );
        assert_eq!(
            to_hex(&sent),
            "8221000b73656e644d657373616765150e1c2821756162616261626162616261626162616261626162616261626162616261626162150026001600480568656c6c6f4215003b0013000000"
        );
        let message = read_message(&sent).unwrap();
        let body = message.fields.get(&2).unwrap();
        let Value::Struct(fields) = body else { panic!("struct") };
        assert_eq!(fields.get(&10), Some(&Value::Bytes(b"hello".to_vec())));
    }

    fn to_hex(data: &[u8]) -> String {
        data.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn talk_exception_field_is_readable() {
        let failed = write_reply(
            "sendMessage",
            &[(
                1,
                Field::Struct(vec![
                    (1, Field::I32(82)),
                    (2, bin("E2EE_RETRY_ENCRYPT")),
                ]),
            )],
        );
        assert_eq!(
            to_hex(&failed),
            "8241000b73656e644d6573736167651c15a4011812453245455f52455452595f454e43525950540000"
        );
        let err = read_message(&failed).unwrap();
        let Value::Struct(body) = err.fields.get(&1).unwrap() else { panic!("struct") };
        assert_eq!(body.get(&1), Some(&Value::Int(82)));
    }

    #[test]
    fn a_set_round_trips_as_a_list() {
        let frame = write_call(
            "inviteIntoChat",
            &[(1, Field::Struct(vec![(3, Field::Set(BINARY, vec![bin("ua"), bin("ub")]))]))],
        );
        let message = read_message(&frame).unwrap();
        let Value::Struct(fields) = message.fields.get(&1).unwrap() else { panic!("struct") };
        let Value::List(items) = fields.get(&3).unwrap() else { panic!("set") };
        assert_eq!(items, &vec![Value::Bytes(b"ua".to_vec()), Value::Bytes(b"ub".to_vec())]);
    }
}
