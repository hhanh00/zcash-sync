use crate::coinconfig::RAPTORQ;
use blake2b_simd::Params;
use byteorder::{ReadBytesExt, WriteBytesExt, LE};
use raptorq::{Decoder, Encoder, EncodingPacket, ObjectTransmissionInformation};
use serde::Serialize;
use std::convert::TryInto;
use std::io::{Cursor, Write};

pub const QR_DATA_SIZE: u16 = 256;

pub struct FountainCodes {
    id: u32,
    decoder: Option<Decoder>,
}

#[derive(Serialize)]
pub struct RaptorQDrops {
    drops: Vec<String>,
}

impl FountainCodes {
    pub fn new() -> Self {
        FountainCodes {
            id: 0,
            decoder: None,
        }
    }

    pub fn encode_into_drops(id: u32, data: &[u8]) -> anyhow::Result<RaptorQDrops> {
        let total_length = data.len() as u32;
        let encoder = Encoder::with_defaults(data, QR_DATA_SIZE);
        let drops: Vec<_> = encoder
            .get_encoded_packets(1)
            .iter()
            .map(|p| {
                let mut result = vec![];
                let data = p.serialize();
                let checksum = Self::get_checksum(&data, id, total_length);
                result.write_u32::<LE>(id).unwrap();
                result.write_u32::<LE>(total_length as u32).unwrap();
                result.write_u32::<LE>(checksum).unwrap();
                result.write_all(&data).unwrap();
                base64::encode(&result)
            })
            .collect();
        Ok(RaptorQDrops { drops })
    }

    pub fn put_drop(&mut self, drop: &str) -> anyhow::Result<Option<Vec<u8>>> {
        let drop = base64::decode(drop)?;
        if drop.len() < 12 {
            anyhow::bail!("Not enough data");
        }
        let (header, data) = drop.split_at(12);
        let mut c = Cursor::new(header);
        let id = c.read_u32::<LE>()?;
        let total_length = c.read_u32::<LE>()?;
        let checksum = c.read_u32::<LE>()?;
        let checksum2 = Self::get_checksum(data, id, total_length);
        if checksum != checksum2 {
            anyhow::bail!("Invalid checksum");
        }

        if self.id != id {
            self.id = id;
            let decoder = Decoder::new(ObjectTransmissionInformation::with_defaults(
                total_length as u64,
                QR_DATA_SIZE,
            ));
            self.decoder = Some(decoder);
        }

        if let Some(ref mut decoder) = self.decoder {
            let res = decoder.decode(EncodingPacket::deserialize(data));
            if res.is_some() {
                self.id = 0;
                self.decoder = None;
            }
            return Ok(res);
        }

        Ok(None)
    }

    fn get_checksum(data: &[u8], id: u32, total_length: u32) -> u32 {
        let hash = Params::new()
            .personal(b"QR_CHECKSUM")
            .hash_length(4)
            .to_state()
            .update(&id.to_le_bytes())
            .update(&total_length.to_le_bytes())
            .update(data)
            .finalize();
        let h = u32::from_le_bytes(hash.as_bytes().try_into().unwrap());
        h
    }
}

pub fn put_drop(drop: &str) -> anyhow::Result<Option<Vec<u8>>> {
    let mut fc = RAPTORQ.lock().unwrap();
    fc.put_drop(drop)
}
