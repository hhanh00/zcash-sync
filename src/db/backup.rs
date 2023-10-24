use crate::db::data_generated::fb::AGEKeysT;
use age::secrecy::ExposeSecret;
use anyhow::anyhow;
use rusqlite::backup::Backup;
use rusqlite::Connection;
use std::fs::File;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::{iter, time};
use zip::write::FileOptions;

pub struct FullEncryptedBackup {
    temp_dir: PathBuf,
    target_path: PathBuf,
    db_names: Vec<String>,
}

impl FullEncryptedBackup {
    pub fn new(target_path: &str, temp_dir: &str) -> Self {
        FullEncryptedBackup {
            temp_dir: PathBuf::from_str(temp_dir).unwrap(),
            target_path: PathBuf::from_str(target_path).unwrap(),
            db_names: vec![],
        }
    }

    pub fn generate_key() -> anyhow::Result<AGEKeysT> {
        let key = age::x25519::Identity::generate();
        let sk = key.to_string().expose_secret().clone();
        let pk = key.to_public().to_string();
        Ok(AGEKeysT {
            sk: Some(sk),
            pk: Some(pk),
        })
    }

    pub fn add(&mut self, src: &Connection, db_name: &str) -> anyhow::Result<()> {
        let dst_path = self.temp_dir.join(db_name);
        let mut dst = Connection::open(&dst_path)?;
        println!("dst path {}", dst_path.display());
        let backup = Backup::new(src, &mut dst)?;
        backup.run_to_completion(100, time::Duration::from_millis(10), None)?;
        self.db_names.push(db_name.to_string());
        Ok(())
    }

    pub fn close(&self, pk: &str) -> anyhow::Result<()> {
        let data = self.make_zip()?;
        let pubkey = age::x25519::Recipient::from_str(pk).map_err(|e| anyhow!(e.to_string()))?;

        let mut encrypted_file = File::create(&self.target_path)?;
        let encryptor = age::Encryptor::with_recipients(vec![Box::new(pubkey)]).unwrap();
        let mut writer = encryptor.wrap_output(&mut encrypted_file)?;
        writer.write_all(&*data)?;
        writer.finish()?;
        Ok(())
    }

    pub fn restore(&self, cipher_key: &str, data_path: &str) -> anyhow::Result<()> {
        let key =
            age::x25519::Identity::from_str(cipher_key).map_err(|e| anyhow!(e.to_string()))?;
        let mut cipher_text = Vec::new();
        let mut f = File::open(data_path)?;
        f.read_to_end(&mut cipher_text)?;

        let decryptor =
            match age::Decryptor::new(&*cipher_text).map_err(|_| anyhow!("Decryption Error"))? {
                age::Decryptor::Recipients(d) => d,
                _ => unreachable!(),
            };

        let mut plain_text = vec![];
        let mut reader = decryptor
            .decrypt(iter::once(&key as &dyn age::Identity))
            .map_err(|_| anyhow!("Decryption Error"))?;
        reader.read_to_end(&mut plain_text)?;

        self.unzip(&plain_text)?;
        Ok(())
    }

    fn make_zip(&self) -> anyhow::Result<Vec<u8>> {
        let mut buffer = Vec::new();
        let zip_data = vec![];
        let buff = Cursor::new(zip_data);
        let mut zip_writer = zip::ZipWriter::new(buff);
        for db_name in self.db_names.iter() {
            zip_writer.start_file(db_name, FileOptions::default())?;
            let mut f = File::open(self.temp_dir.join(db_name))?;
            f.read_to_end(&mut buffer)?;
            zip_writer.write_all(&*buffer)?;
            buffer.clear();
        }
        let r = zip_writer.finish()?;
        Ok(r.into_inner())
    }

    fn unzip(&self, data: &[u8]) -> anyhow::Result<()> {
        let buff = Cursor::new(data);
        let mut zip_reader = zip::ZipArchive::new(buff)?;
        let db_names: Vec<_> = zip_reader.file_names().map(|s| s.to_string()).collect();
        for db_name in db_names {
            let mut zip_file = zip_reader.by_name(&db_name)?;
            let mut out_file = File::create(&self.temp_dir.join(db_name))?;
            std::io::copy(&mut zip_file, &mut out_file)?;
        }
        Ok(())
    }
}
