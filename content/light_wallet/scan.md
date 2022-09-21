---
title: Scan / Sync
weight: 40
---

Scanning the blockchain can be done in several ways. Ultimately 
they all achieve the same goal:
 
- Determine which notes you received, ie adding incoming funds, 
- Cross-out the notes you spent, ie substracting outgoing funds, 
- And finally, allowing you to spend your UTXO.

The first two bullet points are typical of any UTXO based cryptocurrency wallet,
but the last bullet point may be unusual.

{{%notice note%}}
Once the blockchain scan finishes, your wallet is said to be synchronized.
{{%/notice%}}

In most cryptocurrencies, as long as you have the secret key and 
a reference to the UTXO, you can spend it.

This is not the case of Zcash. 

To spend a shielded UTXO, your wallet must also keep track of a value
called the "witness" specific to a given note *and* block height.
In other words, every note has a witness which is a several hundred byte
long that changes every time we get a new block.

When the wallet wants to spend a note, it needs to compute a "proof"
of validity called a zero knowledge proof (ZKP). The ZKP ensures that
we have the secret key and the reference to the UTXO without actually
disclosing this information.

## ZKP Creation Function

One of the argument of the ZKP creation function is the witness. Therefore,
if you want to spend a note, the wallet needs to compute its witness
at a given height.

Wallets are not obligated to provide the latest witnesses. But since
the height is public information, if a wallet does not update the witnesses
it would be possible to gain some knowledge by looking at the witness heights
of a transaction.

{{%notice note%}}
If a wallet does not update the note witnesses, one can check
the height when a note is spent and deduce when it was received.
{{%/notice %}}

## Nullifiers

In Bitcoin, to spend a UTXO, a wallet simply has to refer to it
and attach a digital signature proving it knows the secret key.

In Zcash, UTXO are encrypted. But that's not enough. Spending a
UTXO must also be hidden. If Zcash transactions referred to UTXO
like Bitcoin does, one could know when the UTXO was created.
Therefore even if it would be impossible to know the values and
addresses of the notes, it would be possible to link notes with 
transactions.

To avoid this, Zcash transactions do not directly refer to UTXO
but they refer to a nullifier instead.

Nullifiers are unique values associated with a note. Only the
receiver of the note can create the nullifier and it's impossible
to make a nullifier for a note that does not exist. The exact
mechanism used to achieve this is beyond the scope of this document.

{{%notice note%}}
Each UTXO has a unique nullifier. Nullifiers cannot be faked: Anyone can verify 
that a nullifier is associated with a real UTXO.
{{%/notice %}}

## Scan Outputs

In conclusion, let's review what a scan must do:

- It must scan each transaction and try to decrypt its outputs.
- Successful decryptions generate "received notes" (fresh UTXO).
- For each received note, the wallet can calculate its nullifier.
- Spent notes are detected when their nullifier is used in a later transaction.
- For each UTXO, the wallet maintains a witness that it should keep updating.

