use rusqlite::{Connection, OptionalExtension, params};
use anyhow::Result;
use crate::db::data_generated::fb::*;
use crate::DbAdapter;

pub fn has_account(connection: &Connection) -> Result<bool> {
    let res = connection.query_row("SELECT 1 FROM accounts", [], |_| { Ok(()) }).optional()?;
    Ok(res.is_some())
}

pub fn get_account_list(connection: &Connection) -> Result<Vec<u8>> {
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let mut stmt = connection.prepare("WITH notes AS (SELECT a.id_account, a.name, CASE WHEN r.spent IS NULL THEN r.value ELSE 0 END AS nv FROM accounts a LEFT JOIN received_notes r ON a.id_account = r.account), \
                       accounts2 AS (SELECT id_account, name, COALESCE(sum(nv), 0) AS balance FROM notes GROUP by id_account) \
                       SELECT a.id_account, a.name, a.balance FROM accounts2 a")?;
    let rows = stmt.query_map([], |row| {
        let id: u32 = row.get("id_account")?;
        let name: String = row.get("name")?;
        let balance: i64 = row.get("balance")?;
        let name = builder.create_string(&name);
        let account = Account::create(&mut builder, &AccountArgs {
            id,
            name: Some(name),
            balance: balance as u64,
        });
        Ok(account)
    })?;
    let mut accounts = vec![];
    for r in rows {
        accounts.push(r?);
    }
    let accounts = builder.create_vector(&accounts);
    let accounts = AccountVec::create(&mut builder, &AccountVecArgs { accounts: Some(accounts) });
    builder.finish(accounts, None);
    let data = builder.finished_data().to_vec();
    Ok(data)
}

pub fn get_available_account_id(connection: &Connection, id: u32) -> Result<u32> {
    let r = connection.query_row("SELECT 1 FROM accounts WHERE id_account = ?1", [id], |_| { Ok(()) }).optional()?;
    if r.is_some() { return Ok(id) }
    let r = connection.query_row("SELECT MAX(id_account) FROM accounts", [], |row| {
        let id: Option<u32> = row.get(0)?;
        Ok(id)
    })?.unwrap_or(0);
    Ok(r)
}

pub fn get_t_addr(connection: &Connection, id: u32) -> Result<String> {
    let address = connection.query_row("SELECT address FROM taddrs WHERE account = ?1", [id], |row| {
        let address: String = row.get(0)?;
        Ok(address)
    })?;
    Ok(address)
}

pub fn get_sk(connection: &Connection, id: u32) -> Result<String> {
    let sk = connection.query_row("SELECT sk FROM accounts WHERE id_account = ?1", [id], |row| {
        let sk: Option<String> = row.get(0)?;
        Ok(sk.unwrap_or(String::new()))
    })?;
    Ok(sk)
}

pub fn update_account_name(connection: &Connection, id: u32, name: &str) -> Result<()> {
    connection.execute("UPDATE accounts SET name = ?2 WHERE id_account = ?1", params![id, name])?;
    Ok(())
}

pub fn get_balances(connection: &Connection, id: u32, confirmed_height: u32) -> Result<Vec<u8>> {
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let shielded = connection.query_row(
        "SELECT SUM(value) AS value FROM received_notes WHERE account = ?1 AND spent IS NULL",
       params![id], |row| {
            let value: Option<i64> = row.get(0)?;
            Ok(value.unwrap_or(0) as u64)
        })?; // funds not spent yet
    let unconfirmed_spent = connection.query_row(
        "SELECT SUM(value) AS value FROM received_notes WHERE account = ?1 AND spent = 0",
        params![id], |row| {
            let value: Option<i64> = row.get(0)?;
            Ok(value.unwrap_or(0) as u64)
        })?; // funds used in unconfirmed tx
    let balance = shielded + unconfirmed_spent;
    let under_confirmed = connection.query_row(
        "SELECT SUM(value) AS value FROM received_notes WHERE account = ?1 AND spent IS NULL AND height > ?2",
        params![id, confirmed_height], |row| {
            let value: Option<i64> = row.get(0)?;
            Ok(value.unwrap_or(0) as u64)
        })?; // funds received but not old enough
    let excluded = connection.query_row(
        "SELECT SUM(value) FROM received_notes WHERE account = ?1 AND spent IS NULL \
        AND height <= ?2 AND excluded",
        params![id, confirmed_height], |row| {
            let value: Option<i64> = row.get(0)?;
            Ok(value.unwrap_or(0) as u64)
        })?; // funds excluded from spending
    let sapling = connection.query_row(
        "SELECT SUM(value) FROM received_notes WHERE account = ?1 AND spent IS NULL AND orchard = 0 AND height <= ?2",
        params![id, confirmed_height], |row| {
            let value: Option<i64> = row.get(0)?;
            Ok(value.unwrap_or(0) as u64)
        })?;
    let orchard = connection.query_row(
        "SELECT SUM(value) FROM received_notes WHERE account = ?1 AND spent IS NULL AND orchard = 1 AND height <= ?2",
        params![id, confirmed_height], |row| {
            let value: Option<i64> = row.get(0)?;
            Ok(value.unwrap_or(0) as u64)
        })?;

    let balance = Balance::create(&mut builder, &BalanceArgs {
        shielded,
        unconfirmed_spent,
        balance,
        under_confirmed,
        excluded,
        sapling,
        orchard
    });
    builder.finish(balance, None);
    let data = builder.finished_data().to_vec();
    Ok(data)
}

pub fn get_db_height(connection: &Connection) -> Result<Vec<u8>> {
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let height = connection.query_row(
        "SELECT height, timestamp FROM blocks WHERE height = (SELECT MAX(height) FROM blocks)",
        [], |row| {
            let height: u32 = row.get(0)?;
            let timestamp: u32 = row.get(1)?;
            let height = Height::create(&mut builder, &HeightArgs {
                height,
                timestamp,
            });
            Ok(height)
        }).optional()?;
    let data = height.map(|h| {
        builder.finish(h, None);
        builder.finished_data().to_vec()
    }).unwrap_or(vec![]);
    Ok(data)
}

pub fn get_notes(connection: &Connection, id: u32) -> Result<Vec<u8>> {
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let mut stmt = connection.prepare(
        "SELECT n.id_note, n.height, n.value, t.timestamp, n.orchard, n.excluded, n.spent FROM received_notes n, transactions t \
           WHERE n.account = ?1 AND (n.spent IS NULL OR n.spent = 0) \
           AND n.tx = t.id_tx ORDER BY n.height DESC")?;
    let rows = stmt.query_map(params![id], |row| {
        let id: u32 = row.get("id_note")?;
        let height: u32 = row.get("height")?;
        let value: i64 = row.get("value")?;
        let timestamp: u32 = row.get("timestamp")?;
        let orchard: u8 = row.get("orchard")?;
        let excluded: Option<bool> = row.get("excluded")?;
        let spent: Option<u32> = row.get("spent")?;
        let note = ShieldedNote::create(&mut builder, &ShieldedNoteArgs {
            id,
            height,
            value: value as u64,
            timestamp,
            orchard: orchard == 1,
            excluded: excluded.unwrap_or(false),
            spent: spent.is_some()
        });
        Ok(note)
    })?;
    let mut notes = vec![];
    for r in rows {
        notes.push(r?);
    }
    let notes = builder.create_vector(&notes);
    let notes = ShieldedNoteVec::create(&mut builder, &ShieldedNoteVecArgs { notes: Some(notes) });
    builder.finish(notes, None);
    let data = builder.finished_data().to_vec();
    Ok(data)
}


/*      final id = row['id_note'];
      final height = row['height'];
      final timestamp = DateTime.fromMillisecondsSinceEpoch(row['timestamp'] * 1000);
      final orchard = row['orchard'] != 0;
      final excluded = (row['excluded'] ?? 0) != 0;
      final spent = row['spent'] == 0;




    final rows = await db.rawQuery("SELECT height, timestamp FROM blocks WHERE height = (SELECT MAX(height) FROM blocks)");
    if (rows.isNotEmpty) {
      final row = rows.first;
      final height = row['height'] as int;
      final timestampEpoch = row['timestamp'] as int;
      final timestamp = DateTime.fromMillisecondsSinceEpoch(timestampEpoch * 1000);
      final blockInfo = BlockInfo(height, timestamp);
      return blockInfo;
    }
    return null;

    return Sqflite.firstIntValue(await db.rawQuery("SELECT SUM(value) FROM received_notes WHERE account = ?1 AND spent IS NULL AND orchard = 0",
      [id])) ?? 0;
  }

  Future<int> getOrchardBalance() async {
    return Sqflite.firstIntValue(await db.rawQuery("SELECT SUM(value) FROM received_notes WHERE account = ?1 AND spent IS NULL AND orchard = 1",
      [id])) ?? 0;

    final balance = Sqflite.firstIntValue(await db.rawQuery(
        "SELECT SUM(value) AS value FROM received_notes WHERE account = ?1 AND (spent IS NULL OR spent = 0)",
        [id])) ?? 0;
    final shieldedBalance = Sqflite.firstIntValue(await db.rawQuery(
        "SELECT SUM(value) AS value FROM received_notes WHERE account = ?1 AND spent IS NULL",
        [id])) ?? 0;
    final unconfirmedSpentBalance = Sqflite.firstIntValue(await db.rawQuery(
        "SELECT SUM(value) AS value FROM received_notes WHERE account = ?1 AND spent = 0",
        [id])) ?? 0;
    final underConfirmedBalance = Sqflite.firstIntValue(await db.rawQuery(
        "SELECT SUM(value) AS value FROM received_notes WHERE account = ?1 AND spent IS NULL AND height > ?2",
        [id, confirmHeight])) ?? 0;
    final excludedBalance = Sqflite.firstIntValue(await db.rawQuery(
        "SELECT SUM(value) FROM received_notes WHERE account = ?1 AND spent IS NULL "
            "AND height <= ?2 AND excluded",
        [id, confirmHeight])) ?? 0;


        "", [id]);
    await db.execute("UPDATE accounts SET name = ?2 WHERE id_account = ?1",


    final List<Map> res1 = await db.rawQuery(
        "SELECT address FROM taddrs WHERE account = ?1", [id]);
    final taddress = res1.isNotEmpty ? res1[0]['address'] : "";
    return taddress;


  // check that the account still exists
  // if not, pick any account
  // if there are none, return 0
  Future<AccountId?> getAvailableAccountId() async {
    final List<Map> res1 = await db.rawQuery(
        "SELECT 1 FROM accounts WHERE id_account = ?1", [id]);
    if (res1.isNotEmpty)
      return AccountId(coin, id);
    final List<Map> res2 = await db.rawQuery(
        "SELECT id_account FROM accounts", []);
    if (res2.isNotEmpty) {
      final id = res2[0]['id_account'];
      return AccountId(coin, id);
    }
    return null;
  }



  Future<List<Account>> getAccountList() async {
    List<Account> accounts = [];

    final List<Map> res = await db.rawQuery(
        "WITH notes AS (SELECT a.id_account, a.name, CASE WHEN r.spent IS NULL THEN r.value ELSE 0 END AS nv FROM accounts a LEFT JOIN received_notes r ON a.id_account = r.account),"
            "accounts2 AS (SELECT id_account, name, COALESCE(sum(nv), 0) AS balance FROM notes GROUP by id_account) "
            "SELECT a.id_account, a.name, a.balance FROM accounts2 a",
        []);
    for (var r in res) {
      final int id = r['id_account'];
      final account = Account(
          coin,
          id,
          r['name'],
          r['balance'],
          0,
          null);
      accounts.add(account);
    }
    return accounts;
  }

  // check that the account still exists
  // if not, pick any account
  // if there are none, return 0
  Future<AccountId?> getAvailableAccountId() async {
    final List<Map> res1 = await db.rawQuery(
        "SELECT 1 FROM accounts WHERE id_account = ?1", [id]);
    if (res1.isNotEmpty)
      return AccountId(coin, id);
    final List<Map> res2 = await db.rawQuery(
        "SELECT id_account FROM accounts", []);
    if (res2.isNotEmpty) {
      final id = res2[0]['id_account'];
      return AccountId(coin, id);
    }
    return null;
  }

  Future<String> getTAddr() async {
    final List<Map> res1 = await db.rawQuery(
        "SELECT address FROM taddrs WHERE account = ?1", [id]);
    final taddress = res1.isNotEmpty ? res1[0]['address'] : "";
    return taddress;
  }

  Future<String?> getSK() async {
    final List<Map> res1 = await db.rawQuery(
        "SELECT sk FROM accounts WHERE id_account = ?1", [id]);
    final sk = res1.isNotEmpty ? res1[0]['address'] : null;
    return sk;
  }

  Future<void> changeAccountName(String name) async {
    await db.execute("UPDATE accounts SET name = ?2 WHERE id_account = ?1",
        [id, name]);
  }
 */