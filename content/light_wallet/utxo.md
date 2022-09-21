---
title: UTXO Model
weight: 30
---

Zcash is a Blockchain that uses the UTXO model first introduced in Bitcoin.

## UTXO

Initially, every account/address has an empty balance. The only way to get
coins into an account is through a transaction (TX).

Transactions take *inputs* and produce *outputs*. Except for the mining 
transaction called the coinbase transaction, every transaction has inputs
that fund the outputs.

We'll not consider mining or minting and therefore we'll ignore the 
coinbase transaction.

## Notes

Inputs and outputs are *notes*. They have an amount and belong to an address.
The address is associated with a secret key that let's you use the note as
an input of a transaction. If you do not have the secret key, you cannot to spend
the output. 

For public coins, notes are in clear text. By analyzing the blockchain, one can
calculate the balance of every address in use. They just need to tally every
incoming note and deduct every spent note.

However, Zcash has both public and private notes. Public notes behave exactly
like explained above but private notes are *encrypted*.

Encrypted notes also have an amount and an address but this information is
not readable unless you have a viewing key. 

{{%notice note%}}
Without the right viewing key, an encrypted note appears as random bytes.
Encrypted notes are also called shielded notes.
{{%/notice %}}

It may be worth remembering that a note belong to a single address but
an address may own any number of notes.

## Unspent Transaction Outputs (UTXO)

UTXO are the notes that haven't been spent yet. 

Their total is the amount of coins in circulation. The UTXO for which you have the secret key are
the funds you can spend. 

Therefore it is very important that your wallet keeps track of all the UTXO you can
spend.

The only way to know which UTXO are yours is to *scan* the blockchain and look at
every transaction inputs and outputs.

### Transparent UTXO

If the UTXO belong to a transparent address, a wallet can leverage an external
service, for example a block explorer, and delegate the scan. The service can scan
the Blockchain once and track every transparent UTXO in a database. 

{{%notice note %}}
Zcashd can index every transaction and keep a track of every transparent address 
ever used.
{{%/notice %}}

This requires address storage and processing power, therefore `zcashd` needs to run
with the `txindex=1` option.

### Shielded UTXO

Shielded UTXO *cannot* be indexed by an external service or by `zcashd`. `zcashd` knows 
about your shielded UTXO but it cannot possibly decrypt other users' encrypted notes.

Therefore, your wallet must scan the blockchain itself.

{{%notice info %}}
To determine the balance of your shielded address, your wallet MUST scan
the blockchain.
{{%/notice %}}

