---
title: Database
weight: 30
---

You can also directly query the database if you just want to leverage
WarpSync for synchronization but you want to implement the wallet logic
yourself.

{{% notice warning %}}
In this workflow, you *must* only query the database and never 
update it.
{{% /notice %}}

You should use the [REST API]({{< relref "rest" >}}) to manage
accounts and perform synchronization.

Then you can query the tables `accounts`, `received_notes`
and `transactions`.

## Accounts

```sql
CREATE TABLE IF NOT EXISTS accounts (
    id_account INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    seed TEXT,
    aindex INTEGER NOT NULL,
    sk TEXT,
    ivk TEXT NOT NULL UNIQUE,
    address TEXT NOT NULL)
```

- seed: account passphrase. Can be NULL if the account was created by secret key or viewing key
- aindex: account sub index
- sk: secret key. Can be NULL if the account was created by viewing key
- ivk: viewing key
- address: shielded address

## Transactions, Notes, Witnesses

They are documented in the [Data Model]({{< relref "data_model/tables" >}})
