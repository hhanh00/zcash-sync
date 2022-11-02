---
title: Overview
weight: 10
---

Zcash unified addresses make creating transactions more complex than before. 

In the past, an address corresponded to a single receiver and was tied to a specific type of note. For example, a z-addr is a sapling address and only receives sapling funds. 
A unified address may contain more than one receiver. Simply put, it combines several legacy addresses.

When the user wants to pay to a unified address, the wallet app has to determine which unspent transaction outputs it can use to minimize information leakage. 
As a result, the wallet may decide to automatically split a payment.

Let's take this example. Alice has 100 ZEC in her wallet. 60 ZEC in combined sapling notes and 40 ZEC in combined orchard notes.
She wants to send 50 ZEC to Bob.

Bob gives her a z-addr. The wallet will build a transaction from Alice's sapling pool because it has sufficient funds.

If instead, Bob gives a unified address with only an Orchard receiver, Alice does not have enough Orchard funds. She has 40 ZEC but needs 50 ZEC.
She can decide to take 10 ZEC from her sapling funds, but it will reveal a transfer of 10 ZEC on the Blockchain. 
If she does not want to reveal any amount, the transaction cannot be made.

Let's say she's ok with revealing amounts. The wallet can split the payment into two outputs: 40 ZEC from Orchard + 10 ZEC from Sapling.
10 ZEC moved from Sapling to Orchard.
Her wallet now has 50 ZEC remaining in the sapling pool.

Alternatively, since she's OK with revealing amounts, the wallet can now use a single output: 50 ZEC from Sapling. 
But she will reveal that 50 ZEC has migrated from Sapling to Orchard.

Now if Bob gives a unified address that has both Sapling and Orchard receivers, the wallet can spend:
- 50 ZEC from Sapling, or
- 40 ZEC from Orchard + 10 ZEC from Sapling
- and many other combinations

No amount is revealed since no funds have crossed pools.

More complex cases can occur when the transaction has multiple recipients and/or involves transparent funds as well.

In the next section, we'll discuss the inputs and settings to the transaction building algorithm. We want to 
keep the settings understandable. 

