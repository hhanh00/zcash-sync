---
title: Tables
weight: 10
---

In the previous section, we discusses the overall goals of scanning the
blockchain. Now we are going to look at the data obtained after the scan
completes.

From the "largest" data to the "smallest" data:

## Blocks
First we have blocks. We keep the block:

- height, hash and time,
- sapling tree

The sapling tree field is unique to Zcash. The wallet needs it
to update the note witnesses.

## Transactions

The wallet only keeps the transactions for which it has detected either
an incoming note or a spent note. Transparent transactions are not 
kept. They are not included in the wallet history either.

- id_tx: the id of the transactions. This is an internal ID only used by
in our database. 
- the account id
- the txid: the hash of the transaction. That's public information
- the height of the block that contains this transaction. Since there can only
be one block at a given height, it uniquely identifies the block too
- the *net* value of the transaction in Zats
- the address of the spend/destination. For transactions that involve multiple recipients
that we know about, the address is arbitrary one of them. For example, if you make
a transaction from your account to several of your own accounts, there will be
one transaction row per account, but the destination address will just be one of the
recipients
- the memo of one of the notes. If you make a multi-payment transaction, only one of
the memos will be stored
- the tx_index: the position of the transaction in the block

{{%notice note %}}
Multi-payment transactions are rare. That's why the database model does not match
directly with the UTXO model. Technically speaking, every UTXO has its own
address and memo.
{{%/notice %}}

The Transaction table is only used for the Transaction History view. It is
not used for calculating the balance or for building new transactions.
In other words, it's purely informational.

## Received Notes

On the contrary, the Received Notes table plays a critical role in
defining the account state.

The Received Notes table has the following columns:

- id_note: The primary key of the table
- account: the account ID to which this note belongs
- position: the absolute position of the note in the overall commitment tree.
The first shielded output has position 0 and every output (regardless of
its owner) increments the position. The order is determined by the order in
which the output appeared in the blockchain. Unconfirmed transactions do not
have a position
- tx: the id of the transaction (not the tx hash)
- height: the height of the block (same as tx height),
- output_index: A transaction can have multiple outputs. This is the index of
the received note inside the transaction
- diversifier: Once decoded, we know the diversifier value that was used to
derive the address. In zcash, a secret key can generate millions of addresses
though many wallets just use one of them
- value: the amount of the note in Zats
- rcm: the random value used by sender when generating the output note. 
- nf: the note nullifier. We calculate this value from the note position and
the full viewing key. The note nullifier is *not* known by the sender
- spent: the block height when the note is spent. If the note is unspent, spent
is NULL. If the note is spent but unconfirmed, height is equal to 0
- excluded: a boolean flag that indicates if this note should be excluded from
note selection when make a new payment

The Received Notes table allows us to:

- compute the account balances by summing the value of unspent notes,
- find which notes can be spent in new transactions
- rollback the blockchain when there is a [reorganization]({{<relref "reorg">}})

## Witnesses

Finally, we have the note witnesses table.

- id_witness: The primary key of the table
- note: The id of the note
- height: The height of the witness
- witness: The value of the witness

There is a unique witness for a given a note and a height.
