---
title: Privacy
weight: 20
---

Non-private coins make up the vast majority of wallets.
Almost all cryptocurrency users rely on them. 
As a result, they have come to expect a certain level of functionality and speed.

But private coins present significant challenges that non-private coins do not have.
Because the content of the blockchain is hidden or encrypted, third party services
cannot index the transactions and maintain address balances in advance.

Imagine having a dictionary where every word and definition is redacted. It would
be impossible to sort the definitions by alphabetical order. Even though with
zero knowledge technology, the node validators can ensure that the transaction
are valid, they cannot decode the amounts and addresses of the shielded transactions.

Today and until further progress in cryptography is made, we are presented with a
dilemna: 
- Either transactions are public and wallets are fast, OR
- transactions are hidden but wallets are slower.

This is not at all specific to zcash. Every private cryptocurrency has the same issue.

{{%notice note%}}
For private coins, wallets have to *scan* the blockchain to find their transactions.
But for public coins, wallets can consult a server that has scanned for them.
{{%/notice%}}

Arguably, if you are not concerned with privacy, you could send your decryption key
to a server and have it scan for you. But obviously, you need to trust that server
to keep your information private. 

{{%notice note%}}
Zcash light wallets do not transmit the decryption key to a third party service
and perform the decryption themselves.
{{%/notice%}}