use crate::ledger::transport::*;
use crate::taddr::derive_from_pubkey;
use anyhow::Result;
use secp256k1::SecretKey;
use zcash_client_backend::encoding::decode_transparent_address;
use zcash_primitives::consensus::Network;
use zcash_primitives::consensus::Parameters;
use zcash_primitives::legacy::{Script, TransparentAddress};
use zcash_primitives::transaction::components::transparent::builder::Unauthorized as TransparentUnauthorized;
use zcash_primitives::transaction::components::transparent::{Authorized, Bundle};
use zcash_primitives::transaction::components::{transparent, Amount, OutPoint, TxIn, TxOut};
use zcash_primitives::transaction::sighash::{SIGHASH_ALL, SignableInput, TransparentAuthorizingContext};
use zcash_primitives::transaction::sighash_v4::v4_signature_hash;
use zcash_primitives::transaction::{TransactionData, Unauthorized};

pub trait TransparentAuth {}

pub struct Unauth {
    builder: transparent::builder::TransparentBuilder,
}
pub struct Proven;

pub struct TransparentBuilder<A> {
    pub taddr_str: String,
    taddr: TransparentAddress,
    pubkey: Vec<u8>,
    pub auth: A,
}

impl TransparentBuilder<Unauth> {
    pub fn new(network: &Network, pubkey: &[u8]) -> Self {
        let taddr_str = derive_from_pubkey(network, &pubkey).unwrap();
        let taddr = decode_transparent_address(
            &network.b58_pubkey_address_prefix(),
            &network.b58_script_address_prefix(),
            &taddr_str,
        )
        .unwrap()
        .unwrap();
        println!("Your Ledger address is {}", taddr_str);
        let builder = transparent::builder::TransparentBuilder::empty();
        TransparentBuilder {
            taddr_str,
            taddr,
            pubkey: pubkey.to_vec(),
            auth: Unauth {
                builder,
            },
        }
    }

    pub fn add_input(&mut self, txid: [u8; 32], index: u32, amount: u64) -> Result<()> {
        self.auth
            .builder
            .add_input_unchecked(
                self.pubkey.clone().try_into().unwrap(),
                OutPoint::new(txid, index),
                TxOut {
                    value: Amount::from_u64(amount).unwrap(),
                    script_pubkey: self.taddr.script(), // will always use the h/w address
                },
            )
            .unwrap();
        Ok(())
    }

    pub fn add_output(&mut self, raw_address: [u8; 21], amount: u64) -> Result<()> {
        if raw_address[0] != 0 {
            anyhow::bail!("Only t1 addresses are supported");
        }
        let ta = TransparentAddress::PublicKey(raw_address[1..21].try_into().unwrap());
        self.auth
            .builder
            .add_output(&ta, Amount::from_u64(amount).unwrap())
            .unwrap();
        Ok(())
    }

    pub fn prepare(
        self,
    ) -> (
        TransparentBuilder<Proven>,
        Option<Bundle<TransparentUnauthorized>>,
    ) {
        let bundle = self.auth.builder.build();
        let builder = TransparentBuilder::<Proven> {
            taddr_str: self.taddr_str,
            taddr: self.taddr,
            pubkey: self.pubkey,
            auth: Proven {},
        };
        (builder, bundle)
    }
}

impl TransparentBuilder<Proven> {
    pub fn sign(
        &self,
        tx_data: &TransactionData<Unauthorized>,
    ) -> Result<Option<Bundle<Authorized>>> {
        let bundle = match tx_data.transparent_bundle.as_ref() {
            Some(bundle) => {
                let mut script_sigs = vec![];
                for (index, amount) in
                    bundle.authorization.input_amounts().iter().enumerate()
                {
                    let txin = SignableInput::Transparent {
                        hash_type: SIGHASH_ALL,
                        index,
                        script_code: &self.taddr.script(),
                        script_pubkey: &self.taddr.script(),
                        value: amount.clone(),
                    };
                    let hash = v4_signature_hash(tx_data, &txin);
                    let signature = ledger_sign_transparent(hash.as_bytes())?;
                    let signature = secp256k1::ecdsa::Signature::from_compact(&signature)?;
                    let mut sig_bytes: Vec<u8> = signature.serialize_der()[..].to_vec();
                    sig_bytes.extend([1]);

                    // P2PKH scriptSig
                    let script_sig = Script::default() << &sig_bytes[..] << &self.pubkey[..];
                    script_sigs.push(script_sig);
                }

                let bundle = Bundle {
                    vin: bundle
                        .vin
                        .iter()
                        .zip(script_sigs)
                        .map(|(txin, sig)| TxIn {
                            prevout: txin.prevout.clone(),
                            script_sig: sig,
                            sequence: txin.sequence,
                        })
                        .collect(),
                    vout: bundle.vout.clone(),
                    authorization: Authorized,
                };
                Some(bundle)
            }
            None => None,
        };

        Ok(bundle)
    }
}
